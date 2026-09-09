use rapid_agent_team_config::frontmatter::{parse_frontmatter, replace_frontmatter_model};
use rapid_agent_team_config::installer::{install_from_local_dir, is_safe_zip_path};
use rapid_agent_team_config::jsonc::strip_jsonc_comments_and_trailing_commas;
use rapid_agent_team_config::scanner::scan_environment;
use rapid_agent_team_config::writer::{apply_model_changes, generate_change_plan, ModelChangeItem};
use std::fs;
use std::path::Path;

#[test]
fn test_frontmatter_replacement_and_preservation() {
    let raw = "---\ndescription: Rapid Dev Team Lead\nmode: primary\nmodel: glm/glm-4-flash\n---\n\n# System Prompt\nYou are rapid lead.";
    let parsed = parse_frontmatter(raw).expect("Must parse frontmatter");
    assert_eq!(parsed.model.as_deref(), Some("glm/glm-4-flash"));
    assert_eq!(parsed.mode.as_deref(), Some("primary"));

    let replaced = replace_frontmatter_model(raw, "openai/gpt-4o");
    assert!(replaced.contains("model: openai/gpt-4o"));
    assert!(replaced.contains("# System Prompt\nYou are rapid lead."));
    assert!(!replaced.contains("glm/glm-4-flash"));
}

#[test]
fn test_jsonc_cleaner() {
    let jsonc = r#"{
        // Single line comment
        "provider": {
            /* Block comment */
            "deepseek": {
                "models": {
                    "deepseek-chat": {},
                    "deepseek-reasoner": {},
                },
            },
        },
    }"#;
    let cleaned = strip_jsonc_comments_and_trailing_commas(jsonc);
    let val: serde_json::Value = serde_json::from_str(&cleaned).expect("JSON must be valid");
    assert!(val["provider"]["deepseek"]["models"]["deepseek-chat"].is_object());
}

#[test]
fn test_zip_slip_security() {
    let target = Path::new("/var/data/opencode");
    assert!(is_safe_zip_path(target, "agents/rapid-dev-team.md").is_some());
    assert!(is_safe_zip_path(target, "commands/rapid-dev.md").is_some());
    assert!(is_safe_zip_path(target, "../../../etc/shadow").is_none());
    assert!(is_safe_zip_path(target, "/absolute/path").is_none());
    assert!(is_safe_zip_path(target, "agents/../../escape.md").is_none());
}

#[test]
fn test_plan_and_atomic_apply_and_rollback() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_opencode_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let agents_dir = temp_dir.join("agents");
    fs::create_dir_all(&agents_dir).unwrap();

    let agent_file = agents_dir.join("rapid-dev-team.md");
    let initial_content = "---\ndescription: Lead\nmodel: old/model\n---\n\nBody";
    fs::write(&agent_file, initial_content).unwrap();

    let meta = agent_file.metadata().unwrap();
    let mtime = meta
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let items = vec![ModelChangeItem {
        agent_name: "rapid-dev-team".to_string(),
        file_path: agent_file.to_string_lossy().to_string(),
        original_model: Some("old/model".to_string()),
        new_model: "new/awesome-model".to_string(),
        expected_mtime: mtime,
    }];

    // 1. Plan
    let plan = generate_change_plan(&items);
    assert!(plan.has_changes);
    assert_eq!(plan.changes.len(), 1);

    // 2. Apply
    let res = apply_model_changes(&temp_dir, &items).expect("Must apply changes");
    assert!(res.success);
    assert_eq!(res.changed_count, 1);

    let updated = fs::read_to_string(&agent_file).unwrap();
    assert!(updated.contains("model: new/awesome-model"));

    // Verify backup existed
    let backup_dir = Path::new(res.backup_dir.as_ref().unwrap());
    assert!(backup_dir.exists());
    assert!(backup_dir.join("rapid-dev-team.md").exists());

    // Clean up
    let _ = fs::remove_dir_all(&temp_dir);
}

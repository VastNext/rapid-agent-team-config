use rapid_agent_team_config::frontmatter::{parse_frontmatter, replace_frontmatter_model};
use rapid_agent_team_config::installer::{
    find_zip_root_prefix, generate_zip_install_plan, install_from_local_dir, install_from_zip,
    is_allowed_team_file, is_safe_zip_path, validate_manifest, TeamConfigManifest,
};
use rapid_agent_team_config::jsonc::strip_jsonc_comments_and_trailing_commas;
use rapid_agent_team_config::network::{
    is_allowed_download_url, is_allowed_repo_url, ALLOWED_REPOSITORIES, OFFICIAL_RAPID_TEAM_REPO,
};
use rapid_agent_team_config::scanner::scan_environment;
use rapid_agent_team_config::writer::{
    apply_model_changes, compute_backup_relative_path, compute_sha256_hex, generate_change_plan,
    ModelChangeItem,
};
use std::fs;
use std::io::Write;
use std::path::Path;

#[test]
fn test_real_sha256_hex() {
    let empty_hash = compute_sha256_hex(b"");
    assert_eq!(
        empty_hash,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );

    let hello_hash = compute_sha256_hex(b"hello world");
    assert_eq!(
        hello_hash,
        "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
    );
    assert_eq!(hello_hash.len(), 64);
}

#[test]
fn test_frontmatter_replacement_and_preservation() {
    let raw = "---\ndescription: Rapid Dev Team Lead\nmode: primary\nmodel: glm/glm-4-flash\n---\n\n# System Prompt\nYou are rapid lead.";
    let parsed = parse_frontmatter(raw).expect("Must parse frontmatter");
    assert_eq!(parsed.model.as_deref(), Some("glm/glm-4-flash"));
    assert_eq!(parsed.mode.as_deref(), Some("primary"));

    let replaced =
        replace_frontmatter_model(raw, "openai/gpt-4o").expect("Replacement must succeed");
    assert!(replaced.contains("model: openai/gpt-4o"));
    assert!(replaced.contains("# System Prompt\nYou are rapid lead."));
    assert!(!replaced.contains("glm/glm-4-flash"));
}

#[test]
fn test_frontmatter_rejects_missing_or_corrupted() {
    let raw_no_fm = "# Plain Markdown\nNo frontmatter here.";
    assert!(replace_frontmatter_model(raw_no_fm, "openai/gpt-4o").is_err());

    let raw_unclosed = "---\nmodel: old/model\nSome text without closing dashes";
    assert!(replace_frontmatter_model(raw_unclosed, "openai/gpt-4o").is_err());
}

#[test]
fn test_crlf_byte_preservation_strict() {
    let raw = "---\r\ndescription: 极速团队总指挥\r\nmode: primary\r\nmodel: glm/glm-4-flash\r\n---\r\n\r\n# 系统设定\r\n你负责协调全部开发工作。";
    let replaced =
        replace_frontmatter_model(raw, "deepseek/deepseek-chat").expect("Replacement must succeed");

    assert!(replaced.contains("\r\n"));
    // Verify no single \n exists without \r
    assert!(!replaced.replace("\r\n", "").contains("\n"));
    assert!(replaced.contains("model: deepseek/deepseek-chat"));
    assert!(replaced.contains("# 系统设定\r\n你负责协调全部开发工作。"));
    assert!(!replaced.contains("glm/glm-4-flash"));
}

#[test]
fn test_crlf_missing_model_insertion() {
    let raw = "---\r\ndescription: 极速团队\r\nmode: subagent\r\n---\r\n\r\n# Body";
    let replaced = replace_frontmatter_model(raw, "openai/gpt-4o").expect("Must succeed");
    assert!(replaced.contains("\r\n"));
    assert!(!replaced.replace("\r\n", "").contains("\n"));
    assert!(replaced.contains("model: openai/gpt-4o"));
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
        "agent": {
            "rapid-builder-deepseek-go": {
                "model": "deepseek/deepseek-chat",
            },
        },
    }"#;
    let cleaned = strip_jsonc_comments_and_trailing_commas(jsonc);
    let val: serde_json::Value = serde_json::from_str(&cleaned).expect("JSON must be valid");
    assert!(val["provider"]["deepseek"]["models"]["deepseek-chat"].is_object());
    assert_eq!(
        val["agent"]["rapid-builder-deepseek-go"]["model"],
        "deepseek/deepseek-chat"
    );
}

#[test]
fn test_zip_slip_security() {
    let target = Path::new("/var/data/opencode");
    assert!(is_safe_zip_path(target, "agents/rapid-dev-team.md").is_some());
    assert!(is_safe_zip_path(target, "commands/rapid-dev.md").is_some());
    assert!(is_safe_zip_path(target, "../../../etc/shadow").is_none());
    assert!(is_safe_zip_path(target, "/absolute/path").is_none());
    assert!(is_safe_zip_path(target, "agents/../../escape.md").is_none());
    assert!(is_safe_zip_path(target, "C:\\Windows\\System32").is_none());
}

#[test]
fn test_allowed_team_file_types_whitelist() {
    assert!(is_allowed_team_file(Path::new("team.config.json")));
    assert!(is_allowed_team_file(Path::new("README.md")));
    assert!(is_allowed_team_file(Path::new("agents/rapid-dev-team.md")));
    assert!(is_allowed_team_file(Path::new(
        "agent/rapid-builder-glm-zhipu.md"
    )));
    assert!(is_allowed_team_file(Path::new("commands/rapid-dev.md")));
    assert!(is_allowed_team_file(Path::new(
        "skills/rapid-dev-team/SKILL.md"
    )));

    // Disallowed / non-rapid files
    assert!(!is_allowed_team_file(Path::new("malicious.exe")));
    assert!(!is_allowed_team_file(Path::new("agents/malicious.dll")));
    assert!(!is_allowed_team_file(Path::new("other_dir/file.txt")));
    assert!(!is_allowed_team_file(Path::new("agents/arbitrary.md")));
}

#[test]
fn test_manifest_validation() {
    let valid = TeamConfigManifest {
        team: Some("rapid-dev-team".to_string()),
        name: Some("Rapid Dev Team".to_string()),
        version: Some("0.1.0".to_string()),
        description: Some("Rapid Team Config".to_string()),
        agents: Some(vec!["rapid-dev-team".to_string()]),
        commands: Some(vec!["rapid-dev".to_string()]),
        skills: Some(vec!["rapid-dev-team".to_string()]),
        command: None,
        skill: None,
    };
    assert!(validate_manifest(&valid).is_ok());

    let invalid = TeamConfigManifest {
        team: None,
        name: Some("Rogue Third Party Hacker Team".to_string()),
        version: Some("1.0".to_string()),
        description: None,
        agents: None,
        commands: None,
        skills: None,
        command: None,
        skill: None,
    };
    assert!(validate_manifest(&invalid).is_err());
}

#[test]
fn test_download_url_allowlist() {
    assert!(is_allowed_download_url(
        "https://github.com/VastNext/rapid-agent-team/releases/download/v0.1.0/release.zip"
    )
    .is_ok());
    assert!(
        is_allowed_download_url("https://api.github.com/repos/VastNext/rapid-agent-team").is_ok()
    );
    assert!(
        is_allowed_download_url("https://objects.githubusercontent.com/some/asset.zip").is_ok()
    );

    // Insecure / Not allowed
    assert!(is_allowed_download_url("http://github.com/insecure").is_err());
    assert!(is_allowed_download_url("https://evil-hacker.com/malicious.zip").is_err());
    assert!(is_allowed_download_url("file:///etc/passwd").is_err());
}

#[test]
fn test_repo_constants() {
    assert_eq!(
        OFFICIAL_RAPID_TEAM_REPO,
        "VastNext/opencode-rapid-agent-team"
    );
    assert!(ALLOWED_REPOSITORIES.contains(&OFFICIAL_RAPID_TEAM_REPO));
    assert!(ALLOWED_REPOSITORIES.contains(&"VastNext/rapid-agent-team"));
    assert!(ALLOWED_REPOSITORIES.contains(&"VastNext/rapid-agent-team-config"));
    assert!(!ALLOWED_REPOSITORIES.contains(&"unknown-user/evil-repo"));
}

#[test]
fn test_plan_and_atomic_apply_and_hash_mtime_guard() {
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

    let bytes = fs::read(&agent_file).unwrap();
    let initial_hash = compute_sha256_hex(&bytes);

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
        expected_hash: Some(initial_hash.clone()),
    }];

    // 1. Plan
    let plan = generate_change_plan(&temp_dir, &items);
    assert!(plan.has_changes);
    assert_eq!(plan.changes.len(), 1);
    assert_eq!(
        Path::new(&plan.changes[0].backup_rel_path),
        Path::new("agents").join("rapid-dev-team.md")
    );

    // 2. Apply
    let res = apply_model_changes(&temp_dir, &items).expect("Must apply changes");
    assert!(res.success);
    assert_eq!(res.changed_count, 1);

    let updated = fs::read_to_string(&agent_file).unwrap();
    assert!(updated.contains("model: new/awesome-model"));

    // Verify backup existed in relative path
    let backup_dir = Path::new(res.backup_dir.as_ref().unwrap());
    assert!(backup_dir.exists());
    assert!(backup_dir.join("agents").join("rapid-dev-team.md").exists());

    // 3. Test external change detection via Hash
    let stale_items = vec![ModelChangeItem {
        agent_name: "rapid-dev-team".to_string(),
        file_path: agent_file.to_string_lossy().to_string(),
        original_model: Some("old/model".to_string()),
        new_model: "another/model".to_string(),
        expected_mtime: mtime,
        expected_hash: Some("outdated_hash_value_12345".to_string()),
    }];
    let err_res = apply_model_changes(&temp_dir, &stale_items);
    assert!(err_res.is_err());
    assert!(err_res.unwrap_err().contains("哈希不匹配"));

    // Clean up
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_local_dir_install_and_rollback() {
    let temp_root = std::env::temp_dir().join(format!(
        "test_local_install_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let src_dir = temp_root.join("source");
    let dst_dir = temp_root.join("target");

    fs::create_dir_all(src_dir.join("agents")).unwrap();
    fs::create_dir_all(src_dir.join("commands")).unwrap();
    fs::create_dir_all(dst_dir.join("agents")).unwrap();

    fs::write(
        src_dir.join("team.config.json"),
        r#"{"name":"Rapid Dev Team","version":"1.0.0","agents":["rapid-dev-team"],"commands":["rapid-dev"],"skills":["rapid-dev-team"]}"#,
    )
    .unwrap();
    fs::write(
        src_dir.join("agents").join("rapid-dev-team.md"),
        "---\nmodel: glm-4\n---\nPrompt",
    )
    .unwrap();
    fs::write(
        src_dir.join("commands").join("rapid-dev.md"),
        "---\ndescription: command\n---\nBody",
    )
    .unwrap();

    // Existing file in destination that will be overwritten
    fs::write(
        dst_dir.join("agents").join("rapid-dev-team.md"),
        "---\nmodel: old\n---\nOld Prompt",
    )
    .unwrap();

    let res = install_from_local_dir(&src_dir, &dst_dir).expect("Install must succeed");
    assert!(res.success);
    assert!(res.installed_files.len() >= 2);
    assert!(res.backup_dir.is_some());

    // Verify file updated
    let content = fs::read_to_string(dst_dir.join("agents").join("rapid-dev-team.md")).unwrap();
    assert!(content.contains("model: glm-4"));

    // Clean up
    let _ = fs::remove_dir_all(&temp_root);
}

#[test]
fn test_local_dir_install_rejects_missing_manifest() {
    let temp_root = std::env::temp_dir().join(format!(
        "test_local_no_manifest_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let src_dir = temp_root.join("source");
    let dst_dir = temp_root.join("target");
    fs::create_dir_all(src_dir.join("agents")).unwrap();
    fs::write(
        src_dir.join("agents").join("rapid-dev-team.md"),
        "---\nmodel: glm-4\n---\nPrompt",
    )
    .unwrap();

    let err = install_from_local_dir(&src_dir, &dst_dir).expect_err("must refuse without manifest");
    assert!(err.contains("team.config.json"));

    let _ = fs::remove_dir_all(&temp_root);
}

#[test]
fn test_local_dir_install_rejects_invalid_manifest() {
    let temp_root = std::env::temp_dir().join(format!(
        "test_local_bad_manifest_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let src_dir = temp_root.join("source");
    let dst_dir = temp_root.join("target");
    fs::create_dir_all(src_dir.join("agents")).unwrap();
    fs::write(
        src_dir.join("team.config.json"),
        r#"{"name":"Rogue Team","version":"1.0.0"}"#,
    )
    .unwrap();
    fs::write(
        src_dir.join("agents").join("rapid-dev-team.md"),
        "---\nmodel: glm-4\n---\nPrompt",
    )
    .unwrap();

    let err = install_from_local_dir(&src_dir, &dst_dir).expect_err("must refuse bad manifest");
    assert!(err.contains("Rapid Dev Team"));

    let _ = fs::remove_dir_all(&temp_root);
}

#[test]
fn test_local_dir_install_backup_preserves_relative_path() {
    let temp_root = std::env::temp_dir().join(format!(
        "test_local_backup_path_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let src_dir = temp_root.join("source");
    let dst_dir = temp_root.join("target");
    fs::create_dir_all(src_dir.join("agents")).unwrap();
    fs::write(
        src_dir.join("team.config.json"),
        r#"{"name":"Rapid Dev Team","version":"1.0.0","agents":["rapid-dev-team"],"commands":["rapid-dev"],"skills":["rapid-dev-team"]}"#,
    )
    .unwrap();
    fs::write(
        src_dir.join("agents").join("rapid-dev-team.md"),
        "---\nmodel: glm-4\n---\nPrompt",
    )
    .unwrap();
    // Existing destination file to be overwritten and backed up
    fs::create_dir_all(dst_dir.join("agents")).unwrap();
    fs::write(
        dst_dir.join("agents").join("rapid-dev-team.md"),
        "---\nmodel: old\n---\nOld Prompt",
    )
    .unwrap();

    let res = install_from_local_dir(&src_dir, &dst_dir).expect("install must succeed");
    assert!(res.success);
    assert!(res.backup_dir.is_some());
    let updated = fs::read_to_string(dst_dir.join("agents").join("rapid-dev-team.md")).unwrap();
    assert!(updated.contains("model: glm-4"));
    // Backup must exist preserving the relative path
    let backup_dir = Path::new(res.backup_dir.as_ref().unwrap());
    assert!(backup_dir.join("agents").join("rapid-dev-team.md").exists());

    let _ = fs::remove_dir_all(&temp_root);
}

#[test]
fn test_same_basename_different_directories_backup_collision_free() {
    let base = Path::new("/app/opencode");
    let p1 = Path::new("/app/opencode/agents/rapid.md");
    let p2 = Path::new("/app/opencode/commands/rapid.md");

    let r1 = compute_backup_relative_path(base, p1);
    let r2 = compute_backup_relative_path(base, p2);

    assert_ne!(r1, r2);
    assert_eq!(r1.to_string_lossy(), "agents/rapid.md");
    assert_eq!(r2.to_string_lossy(), "commands/rapid.md");
}

#[test]
fn test_project_mode_strict_dot_opencode_target() {
    let dummy_project = Path::new("/workspace/my-app");
    let scan = scan_environment(Some(dummy_project), true);
    assert_eq!(scan.target_scope, "project");
    assert!(scan.opencode_dir.ends_with(".opencode"));
}

// ---------------------------------------------------------------------------
// ZIP 安装路径端到端测试（此前未覆盖的核心功能）
// ---------------------------------------------------------------------------

/// 构造一个带根目录前缀的团队 ZIP 包，返回 zip 文件路径
fn build_team_zip(zip_path: &Path, prefix: &str, files: &[(&str, &str)]) {
    let file = fs::File::create(zip_path).unwrap();
    let mut archive = zip::ZipWriter::new(file);
    let options: zip::write::SimpleFileOptions =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    for (rel, content) in files {
        let entry_name = format!("{}{}", prefix, rel);
        archive
            .start_file(entry_name, options)
            .expect("start_file failed");
        archive.write_all(content.as_bytes()).expect("write failed");
    }
    archive.finish().unwrap();
}

#[test]
fn test_zip_install_full_flow_with_prefix_and_backup() {
    let temp_root = std::env::temp_dir().join(format!(
        "test_zip_install_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let zip_path = temp_root.join("team.zip");
    let target_dir = temp_root.join("opencode");
    fs::create_dir_all(target_dir.join("agents")).unwrap();

    // 目标中已存在将被覆盖的 agents/rapid-dev-team.md
    fs::write(
        target_dir.join("agents").join("rapid-dev-team.md"),
        "---\nmodel: old\n---\nOld Prompt",
    )
    .unwrap();

    build_team_zip(
        &zip_path,
        "rapid-agent-team-main/",
        &[
            (
                "team.config.json",
                r#"{"name":"Rapid Dev Team","version":"1.0.0","agents":["rapid-dev-team"],"commands":["rapid-dev"],"skills":["rapid-dev-team"]}"#,
            ),
            (
                "agents/rapid-dev-team.md",
                "---\nmodel: glm-4\n---\nNew Prompt",
            ),
            ("commands/rapid-dev.md", "---\ndescription: cmd\n---\nCmd"),
        ],
    );

    // 1. 计划
    let plan = generate_zip_install_plan(&zip_path, &target_dir).expect("plan must succeed");
    assert_eq!(plan.total_files, 3); // team.config.json + agent + command
    assert_eq!(plan.overwrite_count, 1); // 仅 rapid-dev-team.md 已存在
    let manifest = plan.manifest_info.expect("manifest must be parsed");
    assert_eq!(manifest.name.as_deref(), Some("Rapid Dev Team"));

    // 2. 安装
    let res = install_from_zip(&zip_path, &target_dir).expect("install must succeed");
    assert!(res.success);
    assert!(res.backup_dir.is_some());
    let installed =
        fs::read_to_string(target_dir.join("agents").join("rapid-dev-team.md")).unwrap();
    assert!(installed.contains("model: glm-4"));

    // 3. 备份保留了相对路径
    let backup_dir = Path::new(res.backup_dir.as_ref().unwrap());
    assert!(backup_dir.join("agents").join("rapid-dev-team.md").exists());

    let _ = fs::remove_dir_all(&temp_root);
}

#[test]
fn test_zip_install_rejects_missing_or_invalid_manifest() {
    let temp_root = std::env::temp_dir().join(format!(
        "test_zip_no_manifest_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let zip_path = temp_root.join("team.zip");
    let target_dir = temp_root.join("opencode");
    fs::create_dir_all(&target_dir).unwrap();

    // 无 team.config.json
    build_team_zip(
        &zip_path,
        "",
        &[("agents/rapid-dev-team.md", "---\nmodel: x\n---\nP")],
    );
    let err = generate_zip_install_plan(&zip_path, &target_dir).expect_err("must reject");
    assert!(err.contains("team.config.json"));

    // 非法清单（名称不匹配 Rapid）
    let zip2 = temp_root.join("team_bad.zip");
    build_team_zip(
        &zip2,
        "",
        &[
            ("team.config.json", r#"{"name":"Rogue","version":"1.0.0"}"#),
            ("agents/rapid-dev-team.md", "---\nmodel: x\n---\nP"),
        ],
    );
    let err2 = generate_zip_install_plan(&zip2, &target_dir).expect_err("must reject bad manifest");
    assert!(err2.contains("Rapid Dev Team"));

    let _ = fs::remove_dir_all(&temp_root);
}

#[test]
fn test_zip_install_rollback_deletes_new_files_and_restores_overwritten() {
    let temp_root = std::env::temp_dir().join(format!(
        "test_zip_rollback_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let zip_path = temp_root.join("team.zip");
    let target_dir = temp_root.join("opencode");
    fs::create_dir_all(&target_dir).unwrap();

    // 目标中存在将被覆盖的文件
    fs::create_dir_all(target_dir.join("agents")).unwrap();
    fs::write(
        target_dir.join("agents").join("rapid-dev-team.md"),
        "---\nmodel: original\n---\nOriginal",
    )
    .unwrap();

    // 在目标放置一个名为 templates 的“文件”，阻止 templates/ 目录创建 → 触发中途失败
    fs::write(target_dir.join("templates"), "I am a file, not a dir").unwrap();

    // zip 内顺序：先 agents、再 commands（新文件），最后 templates（失败点）
    build_team_zip(
        &zip_path,
        "",
        &[
            (
                "team.config.json",
                r#"{"name":"Rapid Dev Team","version":"1.0.0","agents":["rapid-dev-team"],"commands":["rapid-dev"],"skills":["rapid-dev-team"]}"#,
            ),
            ("agents/rapid-dev-team.md", "---\nmodel: glm-4\n---\nNew"),
            ("commands/rapid-dev.md", "---\ndescription: c\n---\nCmd"),
            (
                "templates/rapid-template.md",
                "---\ndescription: t\n---\nTpl",
            ),
        ],
    );

    let err = install_from_zip(&zip_path, &target_dir).expect_err("must fail on templates dir");
    assert!(err.contains("回滚"), "错误应含回滚提示: {}", err);

    // 覆盖文件应被恢复为原始内容
    let restored = fs::read_to_string(target_dir.join("agents").join("rapid-dev-team.md")).unwrap();
    assert!(restored.contains("model: original"));

    // 新增文件（commands/rapid-dev.md）应被删除
    assert!(
        !target_dir.join("commands").join("rapid-dev.md").exists(),
        "新增文件在回滚时应被删除"
    );

    let _ = fs::remove_dir_all(&temp_root);
}

#[test]
fn test_scanner_extracts_providers_models_and_config_bindings() {
    use rapid_agent_team_config::scanner::get_global_opencode_dir;
    use std::env;

    // 隔离：通过临时 OPENCODE_CONFIG_DIR 指向一个受控目录，避免污染真实配置
    let temp_root = std::env::temp_dir().join(format!(
        "test_scanner_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let cfg_dir = temp_root.join("opencode");
    fs::create_dir_all(cfg_dir.join("agents")).unwrap();

    fs::write(
        cfg_dir.join("opencode.json"),
        r#"{
            "provider": {
                "openai": { "models": { "gpt-4o": {}, "gpt-4o-mini": {} } },
                "deepseek": { "models": { "deepseek-chat": {} } },
                "openrouter": { "models": { "z-ai/glm-5": {} } }
            },
             "agent": {
                 "rapid-builder-deepseek-go": { "model": "deepseek/deepseek-chat" },
                 "rapid-ui": { "model": "openai/gpt-4o" },
                 "rapid-builder-glm-zhipu": { "model": "zhipuai-coding-plan/glm-5.3-flash" }
            }
        }"#,
    )
    .unwrap();
    fs::write(
        cfg_dir.join("agents").join("rapid-dev-team.md"),
        "---\ndescription: Lead\nmode: primary\nmodel: openai/gpt-4o\n---\n# P",
    )
    .unwrap();
    // 该 agent 无 frontmatter model，应从 opencode.json 的 agent 绑定回退
    fs::write(
        cfg_dir.join("agents").join("rapid-builder-deepseek-go.md"),
        "---\ndescription: Builder\n---\n# P",
    )
    .unwrap();

    // 设置 OPENCODE_CONFIG_DIR 指向受控目录
    let key = "OPENCODE_CONFIG_DIR";
    let old = env::var_os(key);
    env::set_var(key, &cfg_dir);
    let scan = scan_environment(None, false);
    // 隔离目录应能被 get_global_opencode_dir 解析到（此时 env 仍指向受控目录）
    assert_eq!(get_global_opencode_dir(), cfg_dir);
    if let Some(o) = old {
        env::set_var(key, o);
    } else {
        env::remove_var(key);
    }

    // provider 与模型
    assert!(scan.providers.contains(&"openai".to_string()));
    assert!(scan.providers.contains(&"deepseek".to_string()));
    assert!(scan.providers.contains(&"openrouter".to_string()));
    let model_ids: Vec<String> = scan.models.iter().map(|m| m.id.clone()).collect();
    assert!(model_ids.contains(&"openai/gpt-4o".to_string()));
    assert!(model_ids.contains(&"deepseek/deepseek-chat".to_string()));
    assert!(model_ids.contains(&"openrouter/z-ai/glm-5".to_string()));

    // agent 绑定来自 opencode.json 的 agent 键
    assert_eq!(
        scan.config_agent_bindings.get("rapid-ui"),
        Some(&"openai/gpt-4o".to_string())
    );

    // agent 文件扫描：rapid-dev-team 直接来自 frontmatter
    let lead = scan
        .agents
        .iter()
        .find(|a| a.name == "rapid-dev-team")
        .expect("lead agent found");
    assert_eq!(lead.current_model.as_deref(), Some("openai/gpt-4o"));
    assert_eq!(lead.mode.as_deref(), Some("primary"));

    // rapid-builder-deepseek-go 无 frontmatter model，应从 config 绑定回退
    let builder = scan
        .agents
        .iter()
        .find(|a| a.name == "rapid-builder-deepseek-go")
        .expect("builder agent found");
    assert_eq!(
        builder.current_model.as_deref(),
        Some("deepseek/deepseek-chat")
    );

    // 即使模型只存在于 Agent 当前绑定，也必须进入可选模型清单。
    assert!(model_ids.contains(&"zhipuai-coding-plan/glm-5.3-flash".to_string()));

    let _ = fs::remove_dir_all(&temp_root);
}

#[test]
fn test_find_zip_root_prefix_detects_common_dir() {
    let names = vec![
        "rapid-agent-team-main/team.config.json".to_string(),
        "rapid-agent-team-main/agents/rapid-dev-team.md".to_string(),
    ];
    assert_eq!(find_zip_root_prefix(&names), "rapid-agent-team-main/");

    let no_prefix = vec![
        "team.config.json".to_string(),
        "agents/rapid-dev-team.md".to_string(),
    ];
    assert_eq!(find_zip_root_prefix(&no_prefix), "");
}

#[test]
fn test_repo_whitelist_url_shapes() {
    // 官方 release 资产直链（browser_download_url 形态）
    assert!(is_allowed_repo_url(
        "https://github.com/VastNext/opencode-rapid-agent-team/releases/download/v1.0.0/RapidAgentTeamConfig-windows-x64.exe"
    )
    .is_ok());
    // zipball / archive 形态
    assert!(is_allowed_repo_url(
        "https://api.github.com/repos/VastNext/opencode-rapid-agent-team/zipball/v1.0.0"
    )
    .is_ok());
    // 白名单内其它仓库
    assert!(is_allowed_repo_url(
        "https://github.com/VastNext/rapid-agent-team/releases/download/v1.0.0/team.zip"
    )
    .is_ok());

    // 非白名单仓库必须拒绝
    assert!(is_allowed_repo_url(
        "https://github.com/unknown-user/evil-repo/releases/download/v1.0.0/x.zip"
    )
    .is_err());
    // 恶意域名必须拒绝
    assert!(
        is_allowed_repo_url("https://evil.com/VastNext/opencode-rapid-agent-team/x.zip").is_err()
    );
}

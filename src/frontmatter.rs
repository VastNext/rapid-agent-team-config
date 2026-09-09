//! Frontmatter Parser & Precise In-Place Model Replacer
//!
//! Replaces or extracts the `model:` field in YAML frontmatter while preserving
//! the exact byte sequence of surrounding headers, comments, prompt text, and line endings.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontmatterInfo {
    pub raw_frontmatter: String,
    pub model: Option<String>,
    pub mode: Option<String>,
    pub description: Option<String>,
    pub start_offset: usize,
    pub end_offset: usize,
}

pub fn parse_frontmatter(content: &str) -> Option<FrontmatterInfo> {
    if !content.starts_with("---") {
        return None;
    }

    let end_idx = content[3..].find("\n---")?;
    let fm_content = &content[3..3 + end_idx];
    let mut model = None;
    let mut mode = None;
    let mut description = None;

    for line in fm_content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("model:") {
            let m = rest.trim().trim_matches('"').trim_matches('\'').to_string();
            if !m.is_empty() {
                model = Some(m);
            }
        } else if let Some(rest) = trimmed.strip_prefix("mode:") {
            let m = rest.trim().trim_matches('"').trim_matches('\'').to_string();
            if !m.is_empty() {
                mode = Some(m);
            }
        } else if let Some(rest) = trimmed.strip_prefix("description:") {
            let d = rest.trim().trim_matches('"').trim_matches('\'').to_string();
            if !d.is_empty() {
                description = Some(d);
            }
        }
    }

    Some(FrontmatterInfo {
        raw_frontmatter: fm_content.to_string(),
        model,
        mode,
        description,
        start_offset: 0,
        end_offset: 3 + end_idx + 4, // length of opening + content + \n---
    })
}

/// Replace the `model: ...` line inside the YAML frontmatter.
/// If `model:` doesn't exist inside the frontmatter, it appends `model: <new_model>` before the closing `---`.
/// Line endings (\r\n or \n) are strictly preserved without memory leaks.
pub fn replace_frontmatter_model(content: &str, new_model: &str) -> String {
    if !content.starts_with("---") {
        // No frontmatter, create minimal one
        return format!("---\nmodel: {}\n---\n\n{}", new_model, content);
    }

    let Some(end_idx) = content[3..].find("\n---") else {
        return content.to_string();
    };

    let fm_raw = &content[0..3 + end_idx];
    let rest = &content[3 + end_idx..];

    // Detect CRLF vs LF
    let is_crlf = fm_raw.contains("\r\n");
    let newline = if is_crlf { "\r\n" } else { "\n" };

    let mut lines: Vec<String> = fm_raw
        .split('\n')
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect();
    let mut model_found = false;

    for line in lines.iter_mut() {
        let trimmed = line.trim();
        if trimmed.starts_with("model:") {
            // Keep leading whitespace indentation if any
            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            *line = format!("{}model: {}", indent, new_model);
            model_found = true;
            break;
        }
    }

    if !model_found {
        lines.push(format!("model: {}", new_model));
    }

    let mut new_fm = lines.join(newline);
    new_fm.push_str(rest);
    new_fm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter() {
        let doc = "---\ndescription: test agent\nmode: subagent\nmodel: openai/gpt-4o\n---\n\n# Prompt\nHello world";
        let info = parse_frontmatter(doc).expect("should parse");
        assert_eq!(info.model.as_deref(), Some("openai/gpt-4o"));
        assert_eq!(info.mode.as_deref(), Some("subagent"));
        assert_eq!(info.description.as_deref(), Some("test agent"));
    }

    #[test]
    fn test_replace_frontmatter_model_existing() {
        let doc = "---\ndescription: test\nmodel: old/model-v1\nmode: subagent\n---\n\n# Body";
        let updated = replace_frontmatter_model(doc, "new/model-v2");
        assert!(updated.contains("model: new/model-v2"));
        assert!(!updated.contains("old/model-v1"));
        assert!(updated.contains("# Body"));
    }

    #[test]
    fn test_replace_frontmatter_model_missing() {
        let doc = "---\ndescription: test\nmode: subagent\n---\n\n# Body";
        let updated = replace_frontmatter_model(doc, "new/model-v2");
        assert!(updated.contains("model: new/model-v2"));
        assert!(updated.contains("# Body"));
    }

    #[test]
    fn test_crlf_preservation() {
        let doc = "---\r\ndescription: test\r\nmodel: old/model\r\n---\r\n\r\n# Body";
        let updated = replace_frontmatter_model(doc, "new/model");
        assert!(updated.contains("\r\n"));
        assert!(updated.contains("model: new/model"));
    }
}

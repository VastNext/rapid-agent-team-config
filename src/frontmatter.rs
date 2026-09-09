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

/// Finds the position of closing `---` line inside `sub` (which starts right after opening `---\n` or `---\r\n`).
/// Returns `(close_pos, close_len)` where `close_pos` is the byte index where the closing newline begins,
/// and `close_len` is the byte length of `[newline]---[trailing_newline]`.
fn find_closing_frontmatter(sub: &str) -> Option<(usize, usize)> {
    let mut offset = 0;
    while let Some(idx) = sub[offset..].find("---") {
        let actual_idx = offset + idx;
        // Verify that '---' is preceded by a newline or is at the very beginning of sub
        let is_start_of_line = actual_idx == 0
            || sub[..actual_idx].ends_with("\r\n")
            || sub[..actual_idx].ends_with('\n');

        if is_start_of_line {
            let after = &sub[actual_idx + 3..];
            let is_end_of_line =
                after.is_empty() || after.starts_with("\r\n") || after.starts_with('\n');
            if is_end_of_line {
                let (close_pos, newline_len) =
                    if actual_idx >= 2 && &sub[actual_idx - 2..actual_idx] == "\r\n" {
                        (actual_idx - 2, 2)
                    } else if actual_idx >= 1 && &sub[actual_idx - 1..actual_idx] == "\n" {
                        (actual_idx - 1, 1)
                    } else {
                        (0, 0)
                    };
                let trailing_len = if after.starts_with("\r\n") {
                    2
                } else if after.starts_with('\n') {
                    1
                } else {
                    0
                };
                let close_len = newline_len + 3 + trailing_len;
                return Some((close_pos, close_len));
            }
        }
        offset = actual_idx + 3;
    }
    None
}

pub fn parse_frontmatter(content: &str) -> Option<FrontmatterInfo> {
    if !content.starts_with("---") {
        return None;
    }

    let opening_len = if content.starts_with("---\r\n") {
        5
    } else if content.starts_with("---\n") {
        4
    } else {
        return None;
    };

    let sub = &content[opening_len..];
    let (close_pos, close_len) = find_closing_frontmatter(sub)?;
    let fm_content = &sub[..close_pos];
    let end_offset = opening_len + close_pos + close_len;

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
        end_offset,
    })
}

/// Replace the `model:` line inside the YAML frontmatter.
/// If content has no valid YAML frontmatter, returns an Err to protect file integrity.
/// If `model:` doesn't exist inside the frontmatter, it appends `model: <new_model>` before the closing `---`.
/// Line endings (\r\n or \n) and surrounding non-frontmatter bytes are strictly preserved.
pub fn replace_frontmatter_model(content: &str, new_model: &str) -> Result<String, String> {
    if !content.starts_with("---") {
        return Err(
            "文件缺少 YAML Frontmatter 起始标记 (---)，拒绝修改以防破坏文件正文".to_string(),
        );
    }

    let (opening_len, newline) = if content.starts_with("---\r\n") {
        (5, "\r\n")
    } else if content.starts_with("---\n") {
        (4, "\n")
    } else {
        return Err("文件 YAML Frontmatter 起始标记后缺少换行符".to_string());
    };

    let sub = &content[opening_len..];
    let Some((close_pos, _)) = find_closing_frontmatter(sub) else {
        return Err("文件 YAML Frontmatter 未闭合 (未找到闭合 ---)，拒绝修改".to_string());
    };

    let fm_content = &sub[..close_pos];
    let rest = &sub[close_pos..];

    let mut lines: Vec<String> = if fm_content.is_empty() {
        Vec::new()
    } else {
        fm_content
            .split('\n')
            .map(|l| l.trim_end_matches('\r').to_string())
            .collect()
    };

    let mut model_found = false;
    for line in lines.iter_mut() {
        let trimmed = line.trim();
        if trimmed.starts_with("model:") {
            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            *line = format!("{}model: {}", indent, new_model.trim());
            model_found = true;
            break;
        }
    }

    if !model_found {
        lines.push(format!("model: {}", new_model.trim()));
    }

    let joined_fm = lines.join(newline);
    let mut result = String::with_capacity(opening_len + joined_fm.len() + rest.len());
    result.push_str(&content[..opening_len]);
    result.push_str(&joined_fm);
    result.push_str(rest);

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter_lf() {
        let doc = "---\ndescription: test agent\nmode: subagent\nmodel: openai/gpt-4o\n---\n\n# Prompt\nHello world";
        let info = parse_frontmatter(doc).expect("should parse");
        assert_eq!(info.model.as_deref(), Some("openai/gpt-4o"));
        assert_eq!(info.mode.as_deref(), Some("subagent"));
        assert_eq!(info.description.as_deref(), Some("test agent"));
    }

    #[test]
    fn test_parse_frontmatter_crlf() {
        let doc = "---\r\ndescription: test agent\r\nmode: subagent\r\nmodel: openai/gpt-4o\r\n---\r\n\r\n# Prompt\r\nHello world";
        let info = parse_frontmatter(doc).expect("should parse");
        assert_eq!(info.model.as_deref(), Some("openai/gpt-4o"));
        assert_eq!(info.mode.as_deref(), Some("subagent"));
        assert_eq!(info.description.as_deref(), Some("test agent"));
    }

    #[test]
    fn test_replace_frontmatter_model_existing_lf() {
        let doc = "---\ndescription: test\nmodel: old/model-v1\nmode: subagent\n---\n\n# Body";
        let updated = replace_frontmatter_model(doc, "new/model-v2").expect("Must succeed");
        assert!(updated.contains("model: new/model-v2"));
        assert!(!updated.contains("old/model-v1"));
        assert!(updated.contains("# Body"));
        assert!(!updated.contains("\r"));
    }

    #[test]
    fn test_replace_frontmatter_model_missing_lf() {
        let doc = "---\ndescription: test\nmode: subagent\n---\n\n# Body";
        let updated = replace_frontmatter_model(doc, "new/model-v2").expect("Must succeed");
        assert!(updated.contains("model: new/model-v2"));
        assert!(updated.contains("# Body"));
        assert!(!updated.contains("\r"));
    }

    #[test]
    fn test_replace_frontmatter_model_existing_crlf() {
        let doc = "---\r\ndescription: 极速团队\r\nmodel: old/model-v1\r\nmode: subagent\r\n---\r\n\r\n# Body\r\n中文正文";
        let updated = replace_frontmatter_model(doc, "new/model-v2").expect("Must succeed");
        assert!(updated.contains("\r\n"));
        assert!(!updated.replace("\r\n", "").contains("\n")); // Every newline is strictly CRLF
        assert!(updated.contains("model: new/model-v2"));
        assert!(!updated.contains("old/model-v1"));
        assert!(updated.contains("# Body\r\n中文正文"));
    }

    #[test]
    fn test_replace_frontmatter_model_missing_crlf() {
        let doc = "---\r\ndescription: 极速团队\r\nmode: subagent\r\n---\r\n\r\n# Body\r\n中文正文";
        let updated = replace_frontmatter_model(doc, "new/model-v2").expect("Must succeed");
        assert!(updated.contains("\r\n"));
        assert!(!updated.replace("\r\n", "").contains("\n")); // Pure CRLF
        assert!(updated.contains("model: new/model-v2"));
        assert!(updated.contains("# Body\r\n中文正文"));
    }

    #[test]
    fn test_replace_frontmatter_with_dash_in_content() {
        let doc = "---\ndescription: text with --- inside\nmodel: old/model\n---\n\n# Body";
        let updated = replace_frontmatter_model(doc, "new/model").expect("Must succeed");
        assert!(updated.contains("model: new/model"));
        assert!(updated.contains("description: text with --- inside"));
    }

    #[test]
    fn test_replace_frontmatter_rejects_no_frontmatter() {
        let doc = "# No Frontmatter\nJust body text";
        assert!(replace_frontmatter_model(doc, "new/model-v2").is_err());
    }

    #[test]
    fn test_replace_frontmatter_rejects_unclosed_frontmatter() {
        let doc = "---\ndescription: unclosed\nmodel: foo";
        assert!(replace_frontmatter_model(doc, "new/model-v2").is_err());
    }
}

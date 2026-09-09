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
/// The implementation splices bytes: every byte that does not belong to the replaced
/// `model:` value (including per-line CRLF/LF endings, indentation and the closing
/// `---` sequence) is kept exactly as-is in the original string.
pub fn replace_frontmatter_model(content: &str, new_model: &str) -> Result<String, String> {
    if !content.starts_with("---") {
        return Err(
            "文件缺少 YAML Frontmatter 起始标记 (---)，拒绝修改以防破坏文件正文".to_string(),
        );
    }

    let (opening_len, opening_newline) = if content.starts_with("---\r\n") {
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

    // body 是 frontmatter 的全部内容，但不含闭合 `---` 前的那一个换行符。
    // body_end 即该换行符(若有)或 `---` 本身的起始字节位置。
    let body_end = opening_len + close_pos;
    let body = &content[opening_len..body_end];
    let trimmed_model = new_model.trim();

    // 在 body 内按原始字节扫描各行，寻找 `model:` 键。记录该行在整串中的
    // 内容区段 [line_content_start, line_content_end)，其中 end 不包含行尾换行符。
    let mut line_start = 0usize; // 相对 body 起始的字节位置
    let mut found_model: Option<(usize, usize, usize)> = None; // (line_start, content_end, indent_end)

    while line_start <= body.len() {
        let nl_rel = body[line_start..].find('\n');
        let content_end = match nl_rel {
            Some(i) => line_start + i,
            None => body.len(),
        };
        // 去掉 CRLF 中的 \r，仅属于该行的“可见内容”。
        let mut visible_end = content_end;
        if visible_end > line_start && body.as_bytes()[visible_end - 1] == b'\r' {
            visible_end -= 1;
        }
        // 记录行首缩进（空格/制表符）结束位置
        let mut indent_end = line_start;
        while indent_end < visible_end {
            let b = body.as_bytes()[indent_end];
            if b == b' ' || b == b'\t' {
                indent_end += 1;
            } else {
                break;
            }
        }
        if body[indent_end..visible_end].starts_with("model:") {
            found_model = Some((line_start, visible_end, indent_end));
            break;
        }
        match nl_rel {
            Some(_) => line_start = content_end + 1,
            None => break,
        }
    }

    let mut result = String::with_capacity(content.len() + trimmed_model.len() + 4);

    if let Some((ls_rel, ve_rel, ie_rel)) = found_model {
        // 字节级替换：仅替换整行的“可见内容”区段，保留原始缩进，行尾换行原样保留。
        let ls = opening_len + ls_rel;
        let ve = opening_len + ve_rel;
        let ie = opening_len + ie_rel;
        result.push_str(&content[..ls]);
        result.push_str(&content[ls..ie]); // 原始缩进
        result.push_str("model: ");
        result.push_str(trimmed_model);
        result.push_str(&content[ve..]);
    } else {
        // 未找到 model: 键 → 在闭合 `---` 前追加一行。
        let after_body = &content[body_end..];
        let style = if after_body.starts_with("\r\n") {
            "\r\n"
        } else if after_body.starts_with('\n') {
            "\n"
        } else {
            opening_newline
        };

        result.push_str(&content[..body_end]);
        if body.is_empty() && after_body.starts_with("---") {
            // frontmatter 完全为空且 `---` 紧邻起始标记 → 需自带行尾
            result.push_str("model: ");
            result.push_str(trimmed_model);
            result.push_str(style);
        } else {
            // 常规情况：在现有换行之后、`---` 之前插入独立一行
            result.push_str(style);
            result.push_str("model: ");
            result.push_str(trimmed_model);
        }
        result.push_str(after_body);
    }

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
    fn test_replace_frontmatter_model_existing_lf_exact_bytes() {
        let doc = "---\ndescription: test\nmodel: old/model-v1\nmode: subagent\n---\n\n# Body";
        let updated = replace_frontmatter_model(doc, "new/model-v2").expect("Must succeed");
        // 严格字节级断言：其余行与换行符必须逐字节一致，只有 model 值变化
        let expected = "---\ndescription: test\nmodel: new/model-v2\nmode: subagent\n---\n\n# Body";
        assert_eq!(updated, expected);
        assert!(!updated.contains("\r"));
    }

    #[test]
    fn test_replace_frontmatter_model_missing_lf_exact_bytes() {
        let doc = "---\ndescription: test\nmode: subagent\n---\n\n# Body";
        let updated = replace_frontmatter_model(doc, "new/model-v2").expect("Must succeed");
        let expected = "---\ndescription: test\nmode: subagent\nmodel: new/model-v2\n---\n\n# Body";
        assert_eq!(updated, expected);
    }

    #[test]
    fn test_replace_frontmatter_model_existing_crlf_exact_bytes() {
        let doc = "---\r\ndescription: 极速团队\r\nmodel: old/model-v1\r\nmode: subagent\r\n---\r\n\r\n# Body\r\n中文正文";
        let updated = replace_frontmatter_model(doc, "new/model-v2").expect("Must succeed");
        let expected = "---\r\ndescription: 极速团队\r\nmodel: new/model-v2\r\nmode: subagent\r\n---\r\n\r\n# Body\r\n中文正文";
        assert_eq!(updated, expected);
        assert!(!updated.replace("\r\n", "").contains("\n")); // Every newline is strictly CRLF
    }

    #[test]
    fn test_replace_frontmatter_model_missing_crlf_exact_bytes() {
        let doc = "---\r\ndescription: 极速团队\r\nmode: subagent\r\n---\r\n\r\n# Body\r\n中文正文";
        let updated = replace_frontmatter_model(doc, "new/model-v2").expect("Must succeed");
        let expected = "---\r\ndescription: 极速团队\r\nmode: subagent\r\nmodel: new/model-v2\r\n---\r\n\r\n# Body\r\n中文正文";
        assert_eq!(updated, expected);
        assert!(!updated.replace("\r\n", "").contains("\n")); // Pure CRLF
    }

    #[test]
    fn test_replace_frontmatter_preserves_mixed_line_endings_per_line() {
        // 每行独立换行风格必须保留：description 行为 LF、model 行为 CRLF、
        // mode 行为 CRLF、闭合区段为 CRLF —— 替换前后这些非目标行不能有任何字节漂移。
        let doc = "---\r\ndescription: a\nmodel: old/model\r\nmode: b\r\n---\r\n# Body";
        let updated = replace_frontmatter_model(doc, "new/model").expect("Must succeed");
        let expected = "---\r\ndescription: a\nmodel: new/model\r\nmode: b\r\n---\r\n# Body";
        assert_eq!(updated, expected);
        // description 行的 LF 换行必须原样保留
        assert!(updated.contains("description: a\nmodel: new/model"));
    }

    #[test]
    fn test_replace_frontmatter_model_first_line_exact() {
        let doc = "---\nmodel: old/model\n---\nBody";
        let updated = replace_frontmatter_model(doc, "new/model").expect("Must succeed");
        assert_eq!(updated, "---\nmodel: new/model\n---\nBody");
    }

    #[test]
    fn test_replace_frontmatter_model_insert_into_empty_fm() {
        // 空 frontmatter + 紧随其后的闭合标记：需自行补全换行
        let doc = "---\n---\nBody";
        let updated = replace_frontmatter_model(doc, "new/model").expect("Must succeed");
        assert_eq!(updated, "---\nmodel: new/model\n---\nBody");
    }

    #[test]
    fn test_replace_frontmatter_model_keeps_indent_and_trailing_body_bytes() {
        let doc = "---\r\ndescription: x\r\n    model:  old/model   \r\nmode: y\r\n---\r\nTail\r\n";
        let updated = replace_frontmatter_model(doc, "new/model").expect("Must succeed");
        let expected =
            "---\r\ndescription: x\r\n    model: new/model\r\nmode: y\r\n---\r\nTail\r\n";
        assert_eq!(updated, expected);
        assert_eq!(updated, expected);
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

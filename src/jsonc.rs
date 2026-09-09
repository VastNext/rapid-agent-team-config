//! JSONC Parser (Lightweight, zero-dep comment & trailing-comma stripper)
//!
//! Converts JSONC (JSON with single-line `//`, block `/* */` comments and trailing commas)
//! into valid standard JSON for `serde_json`.

pub fn strip_jsonc_comments_and_trailing_commas(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut in_string = false;
    let mut escape = false;

    while i < len {
        let c = chars[i];

        if in_string {
            out.push(c);
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if c == '"' {
            in_string = true;
            out.push(c);
            i += 1;
            continue;
        }

        // Single-line comment
        if c == '/' && i + 1 < len && chars[i + 1] == '/' {
            i += 2;
            while i < len && chars[i] != '\n' && chars[i] != '\r' {
                i += 1;
            }
            continue;
        }

        // Multi-line comment
        if c == '/' && i + 1 < len && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2; // skip */
            continue;
        }

        out.push(c);
        i += 1;
    }

    // Now remove trailing commas before '}' or ']'
    remove_trailing_commas(&out)
}

fn remove_trailing_commas(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut in_string = false;
    let mut escape = false;

    while i < len {
        let c = chars[i];

        if in_string {
            result.push(c);
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if c == '"' {
            in_string = true;
            result.push(c);
            i += 1;
            continue;
        }

        if c == ',' {
            // Check ahead for next non-whitespace character
            let mut j = i + 1;
            while j < len && chars[j].is_whitespace() {
                j += 1;
            }
            if j < len && (chars[j] == '}' || chars[j] == ']') {
                // Trailing comma found! Skip the comma
                i += 1;
                continue;
            }
        }

        result.push(c);
        i += 1;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_comments_and_trailing_commas() {
        let raw = r#"{
            // This is a comment
            "provider": {
                /* block comment */
                "openai": {
                    "models": {
                        "gpt-4o": {},
                    },
                },
            },
        }"#;

        let cleaned = strip_jsonc_comments_and_trailing_commas(raw);
        let parsed: serde_json::Value =
            serde_json::from_str(&cleaned).expect("Must parse valid JSON");
        assert!(parsed.get("provider").is_some());
    }

    #[test]
    fn test_keep_strings_intact() {
        let raw = r#"{"url": "http://example.com//test/*not_comment*/", "key": "val,"}"#;
        let cleaned = strip_jsonc_comments_and_trailing_commas(raw);
        let parsed: serde_json::Value = serde_json::from_str(&cleaned).unwrap();
        assert_eq!(parsed["url"], "http://example.com//test/*not_comment*/");
        assert_eq!(parsed["key"], "val,");
    }
}

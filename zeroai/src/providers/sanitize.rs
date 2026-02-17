//! Sanitize API error strings: scrub secret-like tokens and truncate length.
//! Ported from zeroclaw/src/providers/mod.rs.

const MAX_API_ERROR_CHARS: usize = 200;

fn is_secret_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':')
}

fn token_end(input: &str, from: usize) -> usize {
    let mut end = from;
    for (i, c) in input[from..].char_indices() {
        if is_secret_char(c) {
            end = from + i + c.len_utf8();
        } else {
            break;
        }
    }
    end
}

/// Scrub known secret-like token prefixes from provider error strings.
///
/// Redacts tokens with prefixes like `sk-`, `xoxb-`, and `xoxp-`.
pub fn scrub_secret_patterns(input: &str) -> String {
    const PREFIXES: [&str; 3] = ["sk-", "xoxb-", "xoxp-"];

    let mut scrubbed = input.to_string();

    for prefix in PREFIXES {
        let mut search_from = 0;
        loop {
            let Some(rel) = scrubbed[search_from..].find(prefix) else {
                break;
            };

            let start = search_from + rel;
            let content_start = start + prefix.len();
            let end = token_end(&scrubbed, content_start);

            // Bare prefixes like "sk-" should not stop future scans.
            if end == content_start {
                search_from = content_start;
                continue;
            }

            scrubbed.replace_range(start..end, "[REDACTED]");
            search_from = start + "[REDACTED]".len();
        }
    }

    scrubbed
}

/// Sanitize API error text by scrubbing secrets and truncating length.
pub fn sanitize_api_error(input: &str) -> String {
    let scrubbed = scrub_secret_patterns(input);

    if scrubbed.chars().count() <= MAX_API_ERROR_CHARS {
        return scrubbed;
    }

    let mut end = MAX_API_ERROR_CHARS;
    while end > 0 && !scrubbed.is_char_boundary(end) {
        end -= 1;
    }

    format!("{}...", &scrubbed[..end])
}

/// Build a sanitized provider error from a failed HTTP response body and status.
pub fn api_error_body(status: u16, body: &str) -> super::ProviderError {
    super::ProviderError::Http {
        status,
        body: sanitize_api_error(body),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_secret_patterns_redacts_sk() {
        let input = "request failed: sk-1234567890abcdef";
        let out = scrub_secret_patterns(input);
        assert!(!out.contains("sk-1234567890abcdef"));
        assert!(out.contains("[REDACTED]"));
    }

    #[test]
    fn scrub_secret_patterns_redacts_multiple_prefixes() {
        let input = "keys sk-abcdef xoxb-12345 xoxp-67890";
        let out = scrub_secret_patterns(input);
        assert!(!out.contains("sk-abcdef"));
        assert!(!out.contains("xoxb-12345"));
        assert!(!out.contains("xoxp-67890"));
        assert!(out.contains("[REDACTED]"));
    }

    #[test]
    fn scrub_secret_patterns_keeps_bare_prefix() {
        let input = "only prefix sk- present";
        let out = scrub_secret_patterns(input);
        assert!(out.contains("sk-"));
        assert!(!out.contains("[REDACTED]"));
    }

    #[test]
    fn sanitize_api_error_truncates_to_200_chars() {
        let long = "a".repeat(400);
        let result = sanitize_api_error(&long);
        assert!(result.len() <= 203, "len={}", result.len());
        assert!(result.ends_with("..."));
    }

    #[test]
    fn sanitize_api_error_empty_string() {
        let result = sanitize_api_error("");
        assert_eq!(result, "");
    }

    #[test]
    fn sanitize_api_error_no_secrets_unchanged() {
        let input = "simple upstream timeout";
        let result = sanitize_api_error(input);
        assert_eq!(result, input);
    }

    #[test]
    fn sanitize_api_error_redacts_then_truncates() {
        let input = format!("{} sk-abcdef123456 {}", "a".repeat(190), "b".repeat(190));
        let result = sanitize_api_error(&input);
        assert!(!result.contains("sk-abcdef123456"));
        assert!(result.len() <= 203);
    }
}

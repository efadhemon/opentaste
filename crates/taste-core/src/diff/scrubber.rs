//! Redact secrets before a snippet is stored.
//!
//! Entropy is applied only to the contents of string literals. Identifiers and
//! import paths stay intact, because redacting them would destroy the snippet
//! as an example. A separate allowlist keeps UUIDs, semver strings, hex
//! colours, integrity hashes, and path-shaped literals, which are high-entropy
//! often enough to look like secrets and are not.

use std::sync::OnceLock;

use regex::Regex;
use tree_sitter::Node;

use crate::ast::{parse, SupportedLanguage};

const REDACTED: &str = "<REDACTED_SECRET>";
const ENTROPY_LIMIT: f32 = 4.5;

/// Replace secrets in `snippet` with [`REDACTED`].
pub fn scrub(snippet: &str, lang: SupportedLanguage) -> String {
    let patterned = apply_patterns(snippet);
    redact_string_entropy(&patterned, lang)
}

fn apply_patterns(snippet: &str) -> String {
    let without_keys = private_key().replace_all(snippet, REDACTED);
    let without_aws = aws_key().replace_all(&without_keys, REDACTED);
    let without_bearer = bearer().replace_all(&without_aws, "Bearer <REDACTED_SECRET>");
    env_assignment()
        .replace_all(&without_bearer, |caps: &regex::Captures<'_>| {
            let whole = caps.get(0).map(|m| m.as_str()).unwrap_or("");
            let value = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            if value.len() < 8 || value.contains("REDACTED_SECRET") {
                return whole.to_string();
            }
            let start = caps.get(2).map(|m| m.start()).unwrap_or(0);
            let whole_start = caps.get(0).map(|m| m.start()).unwrap_or(0);
            let prefix_len = start.saturating_sub(whole_start);
            format!("{}{REDACTED}", &whole[..prefix_len])
        })
        .into_owned()
}

fn redact_string_entropy(snippet: &str, lang: SupportedLanguage) -> String {
    let Some(tree) = parse(lang, snippet) else {
        return snippet.to_string();
    };
    let mut ranges = Vec::new();
    collect_string_ranges(tree.root_node(), &mut ranges);
    let mut redactions = Vec::new();
    for (start, end) in ranges {
        if end > snippet.len()
            || start > end
            || !snippet.is_char_boundary(start)
            || !snippet.is_char_boundary(end)
        {
            continue;
        }
        let literal = &snippet[start..end];
        for hit in token_regex().find_iter(literal) {
            let token = hit.as_str();
            if allowlisted(token) || shannon_entropy(token) <= ENTROPY_LIMIT {
                continue;
            }
            redactions.push((start + hit.start(), start + hit.end()));
        }
    }
    redactions.sort_by_key(|range| range.0);
    redactions.dedup();
    let mut output = snippet.to_string();
    for (start, end) in redactions.into_iter().rev() {
        if output.is_char_boundary(start) && output.is_char_boundary(end) {
            output.replace_range(start..end, REDACTED);
        }
    }
    output
}

fn collect_string_ranges(node: Node<'_>, ranges: &mut Vec<(usize, usize)>) {
    if is_string_content(node.kind()) {
        ranges.push((node.start_byte(), node.end_byte()));
        return;
    }
    if is_string_container(node.kind()) {
        let mut cursor = node.walk();
        let children: Vec<_> = node.named_children(&mut cursor).collect();
        let contents: Vec<_> = children
            .iter()
            .copied()
            .filter(|child| is_string_content(child.kind()))
            .collect();
        if contents.is_empty() {
            let start = node.start_byte();
            let end = node.end_byte();
            if end.saturating_sub(start) >= 2 {
                ranges.push((start + 1, end - 1));
            }
        } else {
            for child in contents {
                ranges.push((child.start_byte(), child.end_byte()));
            }
            for child in children {
                if !is_string_content(child.kind()) {
                    collect_string_ranges(child, ranges);
                }
            }
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_string_ranges(child, ranges);
    }
}

fn is_string_content(kind: &str) -> bool {
    matches!(kind, "string_fragment" | "string_content")
}

fn is_string_container(kind: &str) -> bool {
    matches!(
        kind,
        "string"
            | "template_string"
            | "string_literal"
            | "interpreted_string_literal"
            | "raw_string_literal"
    )
}

fn allowlisted(token: &str) -> bool {
    token.contains("REDACTED_SECRET")
        || uuid().is_match(token)
        || semver().is_match(token)
        || hex_colour().is_match(token)
        || token.starts_with("sha256-")
        || token.starts_with("sha512-")
        || token.starts_with("./")
        || token.starts_with("../")
        || token.starts_with("@/")
        || token.starts_with("~/")
        || token.starts_with("http://")
        || token.starts_with("https://")
}

fn shannon_entropy(token: &str) -> f32 {
    if token.is_empty() {
        return 0.0;
    }
    let mut counts = [0u32; 256];
    for byte in token.bytes() {
        counts[byte as usize] += 1;
    }
    let len = token.len() as f32;
    let mut entropy = 0.0f32;
    for count in counts {
        if count == 0 {
            continue;
        }
        let p = count as f32 / len;
        entropy -= p * p.log2();
    }
    entropy
}

fn private_key() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?-----END [A-Z0-9 ]*PRIVATE KEY-----",
        )
        .expect("private key regex")
    })
}

fn aws_key() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\bAKIA[0-9A-Z]{16}\b").expect("aws regex"))
}

fn bearer() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\bBearer\s+[A-Za-z0-9\-._~+/]+=*").expect("bearer regex"))
}

fn env_assignment() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?m)\b([A-Z][A-Z0-9_]*(?:SECRET|TOKEN|PASSWORD|PASSWD)|[A-Z][A-Z0-9_]*_KEY)\s*=\s*([^\s#'"]+)"#,
        )
        .expect("env assignment pattern")
    })
}

fn token_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[A-Za-z0-9+/=_.@:~-]{17,}").expect("token pattern"))
}

fn uuid() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
            .expect("uuid regex")
    })
}

fn semver() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^v?\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$")
            .expect("semver regex")
    })
}

fn hex_colour() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^#?[0-9a-fA-F]{3,8}$").expect("colour regex"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_credential_is_removed_and_the_surrounding_identifier_survives() {
        let source = r#"
            function keepMe() {
              const key = "AKIAIOSFODNN7EXAMPLE";
              return key;
            }
        "#;
        let scrubbed = scrub(source, SupportedLanguage::TypeScript);
        assert!(!scrubbed.contains("AKIAIOSFODNN7EXAMPLE"), "{scrubbed}");
        assert!(scrubbed.contains("keepMe"), "{scrubbed}");
        assert!(scrubbed.contains(REDACTED), "{scrubbed}");
    }

    #[test]
    fn uuids_and_semver_strings_are_not_redacted() {
        let source = r#"
            const id = "550e8400-e29b-41d4-a716-446655440000";
            const version = "1.0.0-beta.1+exp.sha.5114f85";
        "#;
        let scrubbed = scrub(source, SupportedLanguage::TypeScript);
        assert!(
            scrubbed.contains("550e8400-e29b-41d4-a716-446655440000"),
            "{scrubbed}"
        );
        assert!(
            scrubbed.contains("1.0.0-beta.1+exp.sha.5114f85"),
            "{scrubbed}"
        );
        assert!(!scrubbed.contains(REDACTED), "{scrubbed}");
    }

    #[test]
    fn bearer_tokens_private_keys_and_env_assignments_are_redacted() {
        let source = r#"
            const header = "Bearer abcdEFGHijklMNOP.qrst";
            const pem = `-----BEGIN RSA PRIVATE KEY-----
            MIIBOgIBAAJB
            -----END RSA PRIVATE KEY-----`;
            API_KEY=supersecretvalue
            TOKEN=short
        "#;
        let scrubbed = scrub(source, SupportedLanguage::TypeScript);
        assert!(!scrubbed.contains("abcdEFGHijklMNOP"), "{scrubbed}");
        assert!(!scrubbed.contains("MIIBOgIBAAJB"), "{scrubbed}");
        assert!(!scrubbed.contains("supersecretvalue"), "{scrubbed}");
        assert!(scrubbed.contains("TOKEN=short"), "{scrubbed}");
        assert!(scrubbed.contains("API_KEY="), "{scrubbed}");
    }
}

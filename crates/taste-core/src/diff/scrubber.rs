//! Redact secrets before a snippet is stored.
//!
//! Two layers run in order. Patterns come first: vendor key prefixes, private
//! key blocks, webhook and credential-bearing URLs, bearer tokens, and
//! assignments to a name that says it holds a secret. Entropy comes second and
//! is applied only to the contents of string literals. Identifiers and import
//! paths stay intact, because redacting them would destroy the snippet as an
//! example. A separate allowlist keeps UUIDs, semver strings, hex colours,
//! integrity hashes, and path-shaped literals, which are high-entropy often
//! enough to look like secrets and are not.
//!
//! A flat entropy limit is not enough on its own. Shannon entropy is capped by
//! the alphabet and by the token length, so hex (at most 4.0 bits per
//! character) and anything shorter than about 23 characters can never pass
//! [`ENTROPY_LIMIT`]. [`looks_random`] adds a lower limit for long hex tokens and a
//! length-scaled one for short tokens that mix upper case, lower case and
//! digits.

use std::sync::OnceLock;

use regex::{Captures, Regex};
use tree_sitter::Node;

use crate::ast::{parse, SupportedLanguage};

const REDACTED: &str = "<REDACTED_SECRET>";
const ENTROPY_LIMIT: f32 = 4.5;
/// Random hex carries close to 4.0 bits per character; English-like hex words do not.
const HEX_ENTROPY_LIMIT: f32 = 3.0;
const MIN_HEX_SECRET: usize = 32;
/// Mixed-class tokens from this length are judged against their own maximum.
const MIN_MIXED_SECRET: usize = 20;
/// Share of the maximum entropy for a token's length that counts as random.
const MIXED_ENTROPY_SHARE: f32 = 0.9;
/// Shorter values in a `SECRET=` style assignment are left alone (`TOKEN=short`).
const MIN_ASSIGNED_SECRET: usize = 8;

/// Replace secrets in `snippet` with [`REDACTED`].
pub fn scrub(snippet: &str, lang: SupportedLanguage) -> String {
    let patterned = apply_patterns(snippet);
    redact_string_entropy(&patterned, lang)
}

fn apply_patterns(snippet: &str) -> String {
    let mut text = private_key().replace_all(snippet, REDACTED).into_owned();
    text = vendor_key().replace_all(&text, REDACTED).into_owned();
    text = webhook()
        .replace_all(&text, |caps: &Captures<'_>| {
            format!("{}{REDACTED}", &caps[1])
        })
        .into_owned();
    text = url_userinfo()
        .replace_all(&text, |caps: &Captures<'_>| {
            format!("{}{REDACTED}{}", &caps[1], &caps[3])
        })
        .into_owned();
    text = url_query_secret()
        .replace_all(&text, |caps: &Captures<'_>| redact_value(caps, &[2]))
        .into_owned();
    text = bearer()
        .replace_all(&text, "Bearer <REDACTED_SECRET>")
        .into_owned();
    text = env_assignment()
        .replace_all(&text, |caps: &Captures<'_>| redact_value(caps, &[2, 3, 4]))
        .into_owned();
    code_assignment()
        .replace_all(&text, |caps: &Captures<'_>| redact_value(caps, &[1]))
        .into_owned()
}

/// The whole match with its value group replaced, unless the value is too
/// short to be a secret or was already redacted by an earlier pattern.
fn redact_value(caps: &Captures<'_>, value_groups: &[usize]) -> String {
    let whole = caps.get(0).expect("group 0 always matches");
    let Some(value) = value_groups.iter().find_map(|index| caps.get(*index)) else {
        return whole.as_str().to_string();
    };
    if value.as_str().len() < MIN_ASSIGNED_SECRET || value.as_str().contains("REDACTED_SECRET") {
        return whole.as_str().to_string();
    }
    let text = whole.as_str();
    let start = value.start() - whole.start();
    let end = value.end() - whole.start();
    format!("{}{REDACTED}{}", &text[..start], &text[end..])
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
            let offset = start + hit.start();
            if is_url(token) {
                // The scheme and host are not secret; a path or query can be.
                for part in url_part_regex().find_iter(token) {
                    if looks_random(part.as_str()) {
                        redactions.push((offset + part.start(), offset + part.end()));
                    }
                }
                continue;
            }
            if allowlisted(token) || !looks_random(token) {
                continue;
            }
            redactions.push((offset, offset + token.len()));
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

/// Whether `token` is random enough to be a credential.
fn looks_random(token: &str) -> bool {
    let entropy = shannon_entropy(token);
    if entropy > ENTROPY_LIMIT {
        return true;
    }
    let has_digit = token.bytes().any(|byte| byte.is_ascii_digit());
    let has_lower = token.bytes().any(|byte| byte.is_ascii_lowercase());
    let has_upper = token.bytes().any(|byte| byte.is_ascii_uppercase());
    if token.len() >= MIN_HEX_SECRET
        && token.bytes().all(|byte| byte.is_ascii_hexdigit())
        && has_digit
        && (has_lower || has_upper)
    {
        return entropy > HEX_ENTROPY_LIMIT;
    }
    if token.len() >= MIN_MIXED_SECRET && has_digit && has_lower && has_upper {
        let ceiling = (token.len() as f32).log2();
        return entropy > ceiling * MIXED_ENTROPY_SHARE;
    }
    false
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

fn is_url(token: &str) -> bool {
    token.starts_with("http://") || token.starts_with("https://")
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

/// Credentials whose prefix identifies the issuer. These are close to free of
/// false positives, so they are matched anywhere, not only inside strings.
///
/// Most prefixes have no leading `\b`, so a key glued to the text before it
/// (`xxxAKIA…`, `MY_ghp_…`) is still found: nothing in ordinary code starts
/// with `AKIA`, `ghp_` or `glpat-` mid-word. `ASIA` and `sk_`/`rk_` keep the
/// boundary, because `SOUTHEASIA…` and identifiers like `task_test_…` or
/// `desk_live_…` do occur, and redacting them would mangle the snippet.
fn vendor_key() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            r"AKIA[0-9A-Z]{16}\b",
            r"|\bASIA[0-9A-Z]{16}\b",
            r"|gh[pousr]_[A-Za-z0-9]{36,}\b",
            r"|github_pat_[A-Za-z0-9_]{22,}\b",
            r"|xox[abposr]-[A-Za-z0-9-]{10,}",
            r"|\b(?:sk|rk)_(?:live|test)_[A-Za-z0-9]{10,}\b",
            r"|AIza[0-9A-Za-z_-]{35}",
            r"|glpat-[A-Za-z0-9_-]{20,}",
            r"|eyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
        ))
        .expect("vendor key regex")
    })
}

/// Incoming-webhook URLs, where the path itself is the credential.
fn webhook() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            r"(https://hooks\.slack\.com/(?:services|workflows|triggers)/)[A-Za-z0-9_/-]+",
            r"|(https://(?:canary\.|ptb\.)?discord(?:app)?\.com/api/webhooks/)[A-Za-z0-9_/-]+",
        ))
        .expect("webhook regex")
    })
}

/// `scheme://user:password@host`. Group 2 is the password.
fn url_userinfo() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?i)(\b[a-z][a-z0-9+.-]*://[^/\s:@'"`]+:)([^/\s@'"`]+)(@)"#)
            .expect("url userinfo regex")
    })
}

/// A query parameter whose name says it carries a credential. Group 2 is the value.
fn url_query_secret() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?i)([?&](?:access_token|api_?key|auth|client_secret|key|password|secret|sig|signature|token)=)([^&\s'"`#]+)"#,
        )
        .expect("url query regex")
    })
}

fn bearer() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\bBearer\s+[A-Za-z0-9\-._~+/]+=*").expect("bearer regex"))
}

/// `.env`-style assignment to a `SCREAMING_CASE` secret name, quoted or not.
/// The value is group 2 (double-quoted), 3 (single-quoted) or 4 (bare).
fn env_assignment() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?m)\b(?:[A-Z0-9_]*(?:SECRET|TOKEN|PASSWORD|PASSWD)|[A-Z][A-Z0-9_]*_KEY)\s*=\s*(?:"([^"\n]*)"|'([^'\n]*)'|([^\s#'"]+))"#,
        )
        .expect("env assignment pattern")
    })
}

/// A string literal assigned to a code identifier that ends in a secret word:
/// `password = "…"`, `apiKey: '…'`, `"clientSecret": "…"`. The keyword has
/// to end the name, so `tokenType` and `passwordField` are not matched.
/// Group 1 is the value.
fn code_assignment() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?i)\b[A-Za-z0-9_$]*(?:password|passwd|secret|token|api_?key|access_?key|private_?key|credentials?)["']?\s*[:=]\s*["'`]([^"'`\n]*)["'`]"#,
        )
        .expect("code assignment pattern")
    })
}

fn token_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[A-Za-z0-9+/=_.@:~-]{17,}").expect("token pattern"))
}

/// Segments of a URL that could hold a credential: no separators, no dots.
fn url_part_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[A-Za-z0-9+=_~-]{17,}").expect("url part pattern"))
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

    fn assert_redacted(source: &str, secret: &str) {
        let scrubbed = scrub(source, SupportedLanguage::TypeScript);
        assert!(!scrubbed.contains(secret), "{secret} survived: {scrubbed}");
        assert!(scrubbed.contains(REDACTED), "{scrubbed}");
    }

    #[test]
    fn a_long_hex_secret_is_redacted_even_though_hex_cannot_reach_the_entropy_limit() {
        let secret = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
        assert_redacted(&format!("const signingKey = \"{secret}\";\n"), secret);
    }

    #[test]
    fn a_short_mixed_case_random_token_is_redacted() {
        let secret = "q8Zr2LmP0xT7vK3nW9bY";
        assert_redacted(&format!("const value = \"{secret}\";\n"), secret);
    }

    #[test]
    fn a_webhook_url_loses_its_path_and_keeps_its_host() {
        let source = "const hook = \"https://hooks.slack.com/services/T024BE7LD/B0123ABCD/x7Kq9PzR2mL5vN8w\";\n";
        let scrubbed = scrub(source, SupportedLanguage::TypeScript);
        assert!(!scrubbed.contains("x7Kq9PzR2mL5vN8w"), "{scrubbed}");
        assert!(scrubbed.contains("hooks.slack.com"), "{scrubbed}");
    }

    #[test]
    fn credentials_inside_a_url_are_redacted() {
        assert_redacted(
            "const db = \"postgres://admin:s3cr3tPassw0rd@db.internal:5432/app\";\n",
            "s3cr3tPassw0rd",
        );
        assert_redacted(
            "fetch(\"https://api.example.com/v1/items?limit=10&api_key=abcd1234efgh5678\");\n",
            "abcd1234efgh5678",
        );
    }

    #[test]
    fn a_random_url_path_segment_is_redacted_but_a_plain_url_survives() {
        assert_redacted(
            "const u = \"https://example.com/share/Zk3P9qL2mX8vR4tN7wB1\";\n",
            "Zk3P9qL2mX8vR4tN7wB1",
        );
        let plain =
            "const docs = \"https://docs.example.com/guides/getting-started-with-typescript\";\n";
        assert_eq!(scrub(plain, SupportedLanguage::TypeScript), plain);
    }

    #[test]
    fn quoted_env_values_and_secret_named_code_assignments_are_redacted() {
        assert_redacted("API_KEY=\"q8Zr2LmP0xT7vK3n\"\n", "q8Zr2LmP0xT7vK3n");
        assert_redacted("SECRET='hunter2hunter2'\n", "hunter2hunter2");
        assert_redacted(
            "const password = \"hunter2hunter2hunter2\";\n",
            "hunter2hunter2hunter2",
        );
        assert_redacted(
            "const config = { apiKey: 'plainbutsecret' };\n",
            "plainbutsecret",
        );
        assert_redacted(
            "{ \"clientSecret\": \"plainbutsecret\" }\n",
            "plainbutsecret",
        );
    }

    #[test]
    fn a_name_that_only_starts_with_a_secret_word_is_left_alone() {
        let source = "const tokenType = \"refresh-token-cookie\";\nconst passwordField = \"login-password-input\";\n";
        assert_eq!(scrub(source, SupportedLanguage::TypeScript), source);
    }

    #[test]
    fn vendor_prefixed_keys_are_redacted_wherever_they_appear() {
        for secret in [
            "ASIAIOSFODNN7EXAMPLE",
            "ghp_aBcDeFgHiJkLmNoPqRsTuVwXyZ0123456789",
            "xoxb-123456789012-abcdefghijkl",
            "sk_live_51Habcdefghijkl",
            "AIzaSyA1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q",
            "glpat-abcdefghij1234567890",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N",
        ] {
            assert_redacted(&format!("// leaked: {secret}\n"), secret);
        }
    }

    #[test]
    fn a_vendor_key_glued_to_the_text_before_it_is_still_redacted() {
        for (glued, secret) in [
            ("xxxAKIAIOSFODNN7EXAMPLE", "AKIAIOSFODNN7EXAMPLE"),
            (
                "MY_ghp_aBcDeFgHiJkLmNoPqRsTuVwXyZ0123456789",
                "ghp_aBcDeFgHiJkLmNoPqRsTuVwXyZ0123456789",
            ),
            (
                "tokenglpat-abcdefghij1234567890",
                "glpat-abcdefghij1234567890",
            ),
            (
                "Bearerxoxb-123456789012-abcdefghijkl",
                "xoxb-123456789012-abcdefghijkl",
            ),
        ] {
            let scrubbed = scrub(
                &format!("// leaked: {glued}\n"),
                SupportedLanguage::TypeScript,
            );
            assert!(!scrubbed.contains(secret), "{glued}: {scrubbed}");
            assert!(scrubbed.contains(REDACTED), "{glued}: {scrubbed}");
        }
    }

    #[test]
    fn identifiers_that_contain_a_risky_prefix_mid_word_are_left_alone() {
        let source = "const task_test_suite1234567 = 1;\nconst desk_live_preview2024 = 2;\nconst SOUTHEASIAPACIFICREGION0001 = 3;\n";
        assert_eq!(scrub(source, SupportedLanguage::TypeScript), source);
    }

    #[test]
    fn ordinary_string_literals_are_not_redacted() {
        let source = r#"
            const route = "/api/v1/users/:id/profile-settings";
            const label = "UserProfileComponent2024";
            const message = "Something went wrong while loading";
            const sha = "sha512-abcdefghijklmnopqrstuvwxyz0123456789";
        "#;
        assert_eq!(scrub(source, SupportedLanguage::TypeScript), source);
    }
}

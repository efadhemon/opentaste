//! Drive `serve()` with scripted JSON lines and assert on what comes back.
//!
//! Each test writes newline-delimited requests into a buffer, runs the server
//! to the end of it, and parses one response per output line. A notification
//! must produce no line at all, which is why counting lines is an assertion.

use std::io::Cursor;

use pretty_assertions::assert_eq;
use serde_json::{json, Value};
use taste_core::config::Paths;
use taste_core::RuleStore;
use taste_mcp::serve;

struct Session {
    _dir: tempfile::TempDir,
    paths: Paths,
    store: RuleStore,
}

impl Session {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::for_root(dir.path());
        let store = RuleStore::open_ephemeral(paths.clone()).unwrap();
        Session {
            _dir: dir,
            paths,
            store,
        }
    }

    /// Send raw lines and return one parsed response per line written.
    fn send_raw(&self, lines: &[&str]) -> Vec<Value> {
        let mut input = lines.join("\n");
        input.push('\n');
        let mut output = Vec::new();
        serve(Cursor::new(input.into_bytes()), &mut output, &self.store).unwrap();
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).expect("every output line is one JSON object"))
            .collect()
    }

    fn send(&self, messages: &[Value]) -> Vec<Value> {
        let lines: Vec<String> = messages.iter().map(Value::to_string).collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        self.send_raw(&refs)
    }

    fn call(&self, tool: &str, arguments: Value) -> Value {
        let mut responses = self.send(&[json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": tool, "arguments": arguments },
        })]);
        assert_eq!(responses.len(), 1);
        let response = responses.remove(0);
        assert!(response.get("error").is_none(), "{response}");
        response["result"].clone()
    }

    fn record(&self, category: &str, scope: &str, instruction: &str) -> Value {
        self.call(
            "record_coding_taste",
            json!({ "category": category, "scope": scope, "instruction": instruction }),
        )
    }
}

fn text(result: &Value) -> &str {
    result["content"][0]["text"].as_str().unwrap()
}

fn is_error(result: &Value) -> bool {
    result["isError"].as_bool().unwrap()
}

#[test]
fn a_full_handshake_answers_initialize_and_stays_silent_on_the_notification() {
    let session = Session::new();
    let responses = session.send(&[
        json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "1" }
            }
        }),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    ]);
    assert_eq!(responses.len(), 1, "the notification must produce no line");
    let result = &responses[0]["result"];
    assert_eq!(responses[0]["id"], json!(0));
    assert_eq!(result["protocolVersion"], json!("2025-06-18"));
    assert_eq!(result["capabilities"], json!({ "tools": {} }));
    assert_eq!(result["serverInfo"]["name"], json!("opentaste"));
}

#[test]
fn an_unknown_protocol_version_falls_back_to_the_default() {
    let session = Session::new();
    let responses = session.send(&[json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "initialize",
        "params": { "protocolVersion": "1999-01-01" }
    })]);
    assert_eq!(
        responses[0]["result"]["protocolVersion"],
        json!(taste_mcp::MCP_PROTOCOL_VERSION)
    );
}

#[test]
fn tools_list_names_exactly_the_three_tools() {
    let session = Session::new();
    let responses = session.send(&[json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })]);
    let tools = responses[0]["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        vec![
            "get_coding_taste",
            "record_coding_taste",
            "reject_coding_taste"
        ]
    );
    for tool in tools {
        assert!(tool["description"].as_str().unwrap().len() > 20, "{tool}");
        assert_eq!(tool["inputSchema"]["type"], json!("object"), "{tool}");
    }
    let record = &tools[1];
    assert!(record["description"]
        .as_str()
        .unwrap()
        .starts_with("Call this tool immediately whenever the user corrects your code style"));
}

#[test]
fn recording_a_preference_confirms_it_and_writes_the_editor_files() {
    let session = Session::new();
    let result = session.record(
        "architecture",
        "src/**/*.entity.ts",
        "Use @AutoEntity instead of @Entity.",
    );
    assert!(!is_error(&result), "{result}");
    assert!(text(&result).contains("CLAUDE.md"), "{}", text(&result));

    let raw: Value =
        serde_json::from_str(&std::fs::read_to_string(&session.paths.rules_json).unwrap()).unwrap();
    let rule = &raw["rules"][0];
    assert_eq!(rule["confidence"], json!(0.95));
    assert_eq!(rule["source"], json!("mcp-feedback"));
    assert_eq!(rule["scope"], json!("src/**/*.entity.ts"));

    let claude = std::fs::read_to_string(session.paths.claude_md()).unwrap();
    assert!(
        claude.contains("Use @AutoEntity instead of @Entity."),
        "{claude}"
    );
}

#[test]
fn recording_keeps_optional_patterns() {
    let session = Session::new();
    let result = session.call(
        "record_coding_taste",
        json!({
            "category": "error-handling",
            "scope": "**/*.ts",
            "instruction": "Throw DomainError instead of Error.",
            "anti_pattern": "throw new Error(\"missing\")",
            "preferred_pattern": "throw new DomainError(\"missing\")",
        }),
    );
    assert!(!is_error(&result), "{result}");
    let rule = &session.store.rules().unwrap().rules[0];
    assert_eq!(
        rule.anti_pattern.as_deref(),
        Some("throw new Error(\"missing\")")
    );
    assert_eq!(
        rule.preferred_pattern.as_deref(),
        Some("throw new DomainError(\"missing\")")
    );
}

#[test]
fn secrets_in_recorded_snippets_never_reach_the_rule_file_or_editor_files() {
    let session = Session::new();
    let key = "AKIAIOSFODNN7EXAMPLE";
    let token = "ghp_8f3Kd92LmQx7Rt4Vb1Nz6Hc0Jw5Ys2Pe9Ua3";
    let result = session.call(
        "record_coding_taste",
        json!({
            "category": "architecture",
            "scope": "src/**/*.service.ts",
            "instruction": "Read credentials from the config service, never inline them.",
            "anti_pattern": format!("const s3 = new S3({{ accessKeyId: \"{key}\" }});"),
            "preferred_pattern": format!("// was: Authorization: Bearer {token}\nconst s3 = new S3(config.aws);"),
        }),
    );
    assert!(!is_error(&result), "{result}");

    let rule = &session.store.rules().unwrap().rules[0];
    let anti = rule.anti_pattern.as_deref().unwrap();
    let preferred = rule.preferred_pattern.as_deref().unwrap();
    assert!(anti.contains("<REDACTED_SECRET>"), "{anti}");
    assert!(
        preferred.contains("const s3 = new S3(config.aws);"),
        "{preferred}"
    );
    for file in [
        std::fs::read_to_string(&session.paths.rules_json).unwrap(),
        std::fs::read_to_string(session.paths.claude_md()).unwrap(),
    ] {
        assert!(!file.contains(key), "{file}");
        assert!(!file.contains(token), "{file}");
    }
}

#[test]
fn get_returns_only_the_rules_whose_scope_matches_the_file() {
    let session = Session::new();
    session.record(
        "architecture",
        "src/**/*.entity.ts",
        "Use @AutoEntity instead of @Entity.",
    );
    session.record(
        "testing",
        "**/*.spec.ts",
        "Co-locate spec files beside the implementation.",
    );

    let entity = session.call(
        "get_coding_taste",
        json!({ "file_path": "src/users/user.entity.ts" }),
    );
    assert!(!is_error(&entity));
    assert!(text(&entity).contains("@AutoEntity"), "{}", text(&entity));
    assert!(!text(&entity).contains("Co-locate"), "{}", text(&entity));

    let other = session.call("get_coding_taste", json!({ "file_path": "README.md" }));
    assert!(!is_error(&other));
    assert!(
        text(&other).starts_with("No OpenTaste preferences apply"),
        "{}",
        text(&other)
    );
}

#[test]
fn reject_disables_by_default_and_deletes_only_when_asked() {
    let session = Session::new();
    session.record(
        "naming",
        "**/*.ts",
        "Name boolean variables with an is or has prefix.",
    );
    let id = session.store.rules().unwrap().rules[0].id.clone();

    let disabled = session.call("reject_coding_taste", json!({ "rule_id": id }));
    assert!(!is_error(&disabled), "{disabled}");
    let rules = session.store.rules().unwrap().rules;
    assert_eq!(rules.len(), 1, "disabling keeps the rule");
    assert!(!rules[0].enabled);
    let claude = std::fs::read_to_string(session.paths.claude_md()).unwrap_or_default();
    assert!(
        !claude.contains("is or has prefix"),
        "a disabled rule leaves the editor files"
    );

    let deleted = session.call(
        "reject_coding_taste",
        json!({ "rule_id": id, "delete": true }),
    );
    assert!(!is_error(&deleted), "{deleted}");
    assert!(session.store.rules().unwrap().rules.is_empty());
}

#[test]
fn an_unknown_rule_id_is_a_tool_error_inside_a_successful_response() {
    let session = Session::new();
    for arguments in [
        json!({ "rule_id": "naming-nope-000000" }),
        json!({ "rule_id": "naming-nope-000000", "delete": true }),
    ] {
        let result = session.call("reject_coding_taste", arguments);
        assert!(is_error(&result), "{result}");
        assert!(
            text(&result).contains("naming-nope-000000"),
            "{}",
            text(&result)
        );
    }
}

#[test]
fn an_invalid_scope_is_rejected_with_the_offending_pattern_named() {
    let session = Session::new();
    let result = session.record("imports", "src/**/[unclosed", "Prefer path aliases.");
    assert!(is_error(&result), "{result}");
    assert!(
        text(&result).contains("src/**/[unclosed"),
        "{}",
        text(&result)
    );
    assert!(session.store.rules().unwrap().rules.is_empty());
}

#[test]
fn an_unknown_category_lists_the_valid_ones() {
    let session = Session::new();
    let result = session.record("style", "**/*", "Use tabs.");
    assert!(is_error(&result), "{result}");
    assert!(text(&result).contains("code-style"), "{}", text(&result));
    assert!(
        text(&result).contains("error-handling"),
        "{}",
        text(&result)
    );
}

#[test]
fn missing_or_mistyped_arguments_are_tool_errors() {
    let session = Session::new();
    let missing = session.call("record_coding_taste", json!({ "category": "naming" }));
    assert!(is_error(&missing));
    assert!(
        text(&missing).contains("`scope` is required"),
        "{}",
        text(&missing)
    );

    let wrong = session.call(
        "reject_coding_taste",
        json!({ "rule_id": "x", "delete": "yes" }),
    );
    assert!(is_error(&wrong));
}

#[test]
fn the_opposite_of_a_confirmed_rule_is_refused_with_a_way_forward() {
    let session = Session::new();
    session.record("types", "**/*.{ts,tsx}", taste_core::ast::PREFER_INTERFACE);
    let result = session.record("types", "**/*.ts", taste_core::ast::PREFER_TYPE_ALIAS);
    assert!(is_error(&result), "{result}");
    assert!(
        text(&result).contains("reject_coding_taste"),
        "{}",
        text(&result)
    );
    assert_eq!(session.store.rules().unwrap().rules.len(), 1);
}

#[test]
fn an_unknown_tool_is_a_protocol_error() {
    let session = Session::new();
    let responses = session.send(&[json!({
        "jsonrpc": "2.0",
        "id": 9,
        "method": "tools/call",
        "params": { "name": "reject_taste", "arguments": {} }
    })]);
    assert_eq!(responses[0]["error"]["code"], json!(-32602));
    assert_eq!(responses[0]["id"], json!(9));
}

#[test]
fn a_malformed_line_gets_a_parse_error_and_the_loop_keeps_answering() {
    let session = Session::new();
    let responses = session.send_raw(&["{not json", r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#]);
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["error"]["code"], json!(-32700));
    assert_eq!(responses[0]["id"], Value::Null);
    assert_eq!(
        responses[1],
        json!({ "jsonrpc": "2.0", "id": 3, "result": {} })
    );
}

#[test]
fn invalid_utf8_is_a_parse_error_not_the_end_of_the_session() {
    let session = Session::new();
    let mut input = vec![0xff, 0xfe, b'\n'];
    input.extend_from_slice(br#"{"jsonrpc":"2.0","id":4,"method":"ping"}"#);
    input.push(b'\n');
    let mut output = Vec::new();
    serve(Cursor::new(input), &mut output, &session.store).unwrap();
    let lines: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["error"]["code"], json!(-32700));
    assert_eq!(lines[1]["id"], json!(4));
}

#[test]
fn ping_is_answered_with_an_empty_result() {
    let session = Session::new();
    let responses = session.send(&[json!({ "jsonrpc": "2.0", "id": "p", "method": "ping" })]);
    assert_eq!(
        responses,
        vec![json!({ "jsonrpc": "2.0", "id": "p", "result": {} })]
    );
}

#[test]
fn an_unknown_method_is_method_not_found_and_an_unknown_notification_is_ignored() {
    let session = Session::new();
    let responses = session.send(&[
        json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": { "requestId": 1 } }),
        json!({ "jsonrpc": "2.0", "id": 5, "method": "resources/list" }),
    ]);
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["error"]["code"], json!(-32601));
}

#[test]
fn a_request_without_a_method_is_an_invalid_request_against_its_id() {
    let session = Session::new();
    let responses = session.send_raw(&[r#"{"jsonrpc":"2.0","id":6}"#, "", "   "]);
    assert_eq!(responses.len(), 1, "blank lines are skipped");
    assert_eq!(responses[0]["error"]["code"], json!(-32600));
    assert_eq!(responses[0]["id"], json!(6));
}

#[test]
fn crlf_line_endings_are_accepted() {
    let session = Session::new();
    let input = "{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"ping\"}\r\n";
    let mut output = Vec::new();
    serve(
        Cursor::new(input.as_bytes().to_vec()),
        &mut output,
        &session.store,
    )
    .unwrap();
    let line: Value = serde_json::from_slice(output.trim_ascii_end()).unwrap();
    assert_eq!(line["id"], json!(8));
}

mod personal {
    use super::*;
    use taste_core::storage::GlobalStore;

    /// A session whose store is attached to the personal rules in `home`.
    fn session_in(home: &std::path::Path) -> Session {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::for_root(dir.path());
        let store = RuleStore::open_ephemeral(paths.clone())
            .unwrap()
            .with_global(GlobalStore::open(home).unwrap());
        Session {
            _dir: dir,
            paths,
            store,
        }
    }

    fn record(session: &Session, instruction: &str, global: bool) -> Value {
        session.call(
            "record_coding_taste",
            json!({
                "category": "code-style",
                "scope": "**/*.ts",
                "instruction": instruction,
                "anti_pattern": "console.log(user)",
                "preferred_pattern": "logger.info(user)",
                "global": global,
            }),
        )
    }

    #[test]
    fn a_preference_recorded_as_global_is_served_in_another_project_marked_personal() {
        let home = tempfile::tempdir().unwrap();
        let origin = session_in(home.path());
        let recorded = record(
            &origin,
            "Log through the logger service, never the console.",
            true,
        );
        assert!(!is_error(&recorded), "{}", text(&recorded));
        assert!(
            text(&recorded).contains("personal rules now"),
            "{}",
            text(&recorded)
        );

        let other = session_in(home.path());
        let got = other.call("get_coding_taste", json!({ "file_path": "src/app.ts" }));
        let answer = text(&got);
        assert!(answer.contains("Personal preferences"), "{answer}");
        assert!(
            answer.contains("(personal) [code-style] Log through the logger service"),
            "{answer}"
        );
        assert!(
            !answer.contains("console.log(user)"),
            "snippets stay in their project: {answer}"
        );

        let none = other.call("get_coding_taste", json!({ "file_path": "src/app.py" }));
        assert!(
            text(&none).starts_with("No OpenTaste preferences"),
            "{}",
            text(&none)
        );
        assert!(
            !other.paths.claude_md().exists()
                || !std::fs::read_to_string(other.paths.claude_md())
                    .unwrap()
                    .contains("logger service")
        );
    }

    #[test]
    fn a_preference_recorded_without_global_stays_in_its_project() {
        let home = tempfile::tempdir().unwrap();
        let origin = session_in(home.path());
        record(
            &origin,
            "Log through the logger service, never the console.",
            false,
        );
        let other = session_in(home.path());
        let got = other.call("get_coding_taste", json!({ "file_path": "src/app.ts" }));
        assert!(
            text(&got).starts_with("No OpenTaste preferences"),
            "{}",
            text(&got)
        );
    }

    #[test]
    fn global_must_be_a_boolean() {
        let home = tempfile::tempdir().unwrap();
        let session = session_in(home.path());
        let result = session.call(
            "record_coding_taste",
            json!({ "category": "code-style", "scope": "**/*", "instruction": "x y z", "global": "yes" }),
        );
        assert!(is_error(&result));
    }
}

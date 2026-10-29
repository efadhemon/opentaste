//! JSON-RPC 2.0 message shapes.
//!
//! MCP's stdio transport carries one JSON object per line: no
//! `Content-Length` header, no blank-line separator. The framing lives in
//! [`crate::server`]; this module only describes a single message.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// Invalid JSON was received.
pub const PARSE_ERROR: i64 = -32700;
/// The JSON sent is not a valid request object.
pub const INVALID_REQUEST: i64 = -32600;
/// The method does not exist or is not available.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// Invalid method parameters.
pub const INVALID_PARAMS: i64 = -32602;
/// Internal JSON-RPC error.
pub const INTERNAL_ERROR: i64 = -32603;

/// The only JSON-RPC version this server speaks.
pub const JSONRPC_VERSION: &str = "2.0";

/// A request or, when `id` is absent, a notification.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    /// `None` only when the field is absent. An explicit `"id": null` is a
    /// request with a null id, which still gets a response.
    #[serde(default, deserialize_with = "present")]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

impl Request {
    /// A notification expects no response at all, not even an error.
    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }
}

/// Distinguish `"id": null` from a missing `id`. With `#[serde(default)]`,
/// this only runs when the field is present.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

/// A response to one request. Exactly one of `result` and `error` is present.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Response {
    jsonrpc: &'static str,
    pub id: Value,
    #[serde(flatten)]
    pub payload: Payload,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Payload {
    #[serde(rename = "result")]
    Result(Value),
    #[serde(rename = "error")]
    Error(ResponseError),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResponseError {
    pub code: i64,
    pub message: String,
}

impl Response {
    pub fn ok(id: Value, result: Value) -> Self {
        Response {
            jsonrpc: JSONRPC_VERSION,
            id,
            payload: Payload::Result(result),
        }
    }

    pub fn error(id: Value, code: i64, message: impl Into<String>) -> Self {
        Response {
            jsonrpc: JSONRPC_VERSION,
            id,
            payload: Payload::Error(ResponseError {
                code,
                message: message.into(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn a_success_carries_a_result_and_no_error() {
        let value = serde_json::to_value(Response::ok(json!(1), json!({"a": 1}))).unwrap();
        assert_eq!(
            value,
            json!({"jsonrpc": "2.0", "id": 1, "result": {"a": 1}})
        );
    }

    #[test]
    fn a_failure_carries_an_error_and_no_result() {
        let value =
            serde_json::to_value(Response::error(json!("x"), METHOD_NOT_FOUND, "nope")).unwrap();
        assert_eq!(
            value,
            json!({"jsonrpc": "2.0", "id": "x", "error": {"code": -32601, "message": "nope"}})
        );
    }

    #[test]
    fn a_missing_id_is_a_notification_and_a_null_id_is_not() {
        let note: Request =
            serde_json::from_str(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .unwrap();
        assert!(note.is_notification());

        let null: Request =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#).unwrap();
        assert!(!null.is_notification());
        assert_eq!(null.id, Some(Value::Null));
    }

    #[test]
    fn params_are_optional() {
        let request: Request =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#).unwrap();
        assert_eq!(request.params, None);
        assert_eq!(request.id, Some(json!(7)));
    }
}

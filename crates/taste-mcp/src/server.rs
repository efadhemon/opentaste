//! The stdio loop and method dispatch.
//!
//! The loop is generic over [`BufRead`] and [`Write`] so the protocol can be
//! driven from string buffers in tests rather than by spawning a process.

use std::io::{BufRead, Write};

use anyhow::Context;
use serde_json::{json, Value};
use taste_core::RuleStore;

use crate::protocol::{
    Request, Response, INVALID_PARAMS, INVALID_REQUEST, JSONRPC_VERSION, METHOD_NOT_FOUND,
    PARSE_ERROR,
};
use crate::{tools, MCP_PROTOCOL_VERSION, SUPPORTED_PROTOCOL_VERSIONS};

/// Answer newline-delimited JSON-RPC requests from `reader` on `writer` until
/// the reader is exhausted.
///
/// A line that cannot be parsed is answered with a parse error and the loop
/// carries on; only an I/O failure on either stream ends it early.
pub fn serve<R: BufRead, W: Write>(
    mut reader: R,
    mut writer: W,
    store: &RuleStore,
) -> anyhow::Result<()> {
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = reader
            .read_until(b'\n', &mut line)
            .context("reading a request from the client")?;
        if read == 0 {
            return Ok(());
        }
        let Some(response) = handle_line(&line, store) else {
            continue;
        };
        serde_json::to_writer(&mut writer, &response).context("encoding a response")?;
        // Unflushed, a reply sits in the buffer and the server looks hung.
        writer
            .write_all(b"\n")
            .and_then(|()| writer.flush())
            .context("writing a response to the client")?;
    }
}

/// The response to one raw line, or `None` when nothing must be sent: a blank
/// line, or a notification.
pub fn handle_line(line: &[u8], store: &RuleStore) -> Option<Response> {
    let Ok(text) = std::str::from_utf8(line) else {
        return Some(Response::error(
            Value::Null,
            PARSE_ERROR,
            "request is not valid UTF-8",
        ));
    };
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(err) => {
            return Some(Response::error(
                Value::Null,
                PARSE_ERROR,
                format!("parse error: {err}"),
            ))
        }
    };
    // Recover the id before validating the rest, so an invalid request is
    // still answered against the id the client is waiting on.
    let id = value.get("id").cloned().unwrap_or(Value::Null);
    let request: Request = match serde_json::from_value(value) {
        Ok(request) => request,
        Err(err) => {
            return Some(Response::error(
                id,
                INVALID_REQUEST,
                format!("invalid request: {err}"),
            ))
        }
    };
    if request.jsonrpc != JSONRPC_VERSION {
        return Some(Response::error(
            id,
            INVALID_REQUEST,
            format!("unsupported jsonrpc version {:?}", request.jsonrpc),
        ));
    }
    if request.is_notification() {
        // `notifications/initialized`, `notifications/cancelled` and anything
        // else: a reply to a notification is a protocol violation.
        return None;
    }
    let id = request.id.clone().unwrap_or(Value::Null);
    Some(match dispatch(&request, store) {
        Ok(result) => Response::ok(id, result),
        Err((code, message)) => Response::error(id, code, message),
    })
}

type Dispatched = Result<Value, (i64, String)>;

fn dispatch(request: &Request, store: &RuleStore) -> Dispatched {
    match request.method.as_str() {
        "initialize" => Ok(initialize(request.params.as_ref())),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::definitions() })),
        "tools/call" => call_tool(request.params.as_ref(), store),
        other => Err((METHOD_NOT_FOUND, format!("method not found: {other}"))),
    }
}

fn initialize(params: Option<&Value>) -> Value {
    let requested = params
        .and_then(|params| params.get("protocolVersion"))
        .and_then(Value::as_str);
    let version = match requested {
        Some(version) if SUPPORTED_PROTOCOL_VERSIONS.contains(&version) => version,
        _ => MCP_PROTOCOL_VERSION,
    };
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "opentaste", "version": env!("CARGO_PKG_VERSION") },
    })
}

fn call_tool(params: Option<&Value>, store: &RuleStore) -> Dispatched {
    let Some(name) = params
        .and_then(|params| params.get("name"))
        .and_then(Value::as_str)
    else {
        return Err((
            INVALID_PARAMS,
            "tools/call needs a string `name`".to_string(),
        ));
    };
    let empty = json!({});
    let arguments = params
        .and_then(|params| params.get("arguments"))
        .unwrap_or(&empty);
    // An unknown tool is a protocol error; anything that goes wrong inside a
    // known tool is reported in the result with `isError: true`.
    tools::call(name, arguments, store)
        .ok_or_else(|| (INVALID_PARAMS, format!("unknown tool: {name}")))
}

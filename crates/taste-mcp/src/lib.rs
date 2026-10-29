//! Local stdio Model Context Protocol server for OpenTaste.
//!
//! [`serve`] speaks newline-delimited JSON-RPC 2.0 over any reader and writer,
//! which is how the tests drive it with string buffers. stdout is the protocol
//! channel, so printing to it from anywhere in this crate is a compile error;
//! diagnostics go to stderr.

#![deny(clippy::print_stdout)]

pub mod protocol;
pub mod server;
mod tools;

pub use server::serve;

/// Protocol version advertised by the server during `initialize` when the
/// client asks for one this server does not know.
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Protocol versions whose request shapes this server answers correctly. A
/// client asking for one of these gets it echoed back.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18"];

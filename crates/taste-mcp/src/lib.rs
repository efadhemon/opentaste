//! Local stdio Model Context Protocol server for OpenTaste.
//!
//! Implemented in phase 4; this crate currently only reserves the workspace slot
//! so the whole workspace builds and tests cleanly.

/// Protocol version advertised by the server during `initialize`.
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

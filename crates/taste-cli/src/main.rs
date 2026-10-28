//! The `taste` binary.
//!
//! Command wiring and the terminal dashboard land in phase 5. For now the binary
//! only reports the engine version so the workspace has a buildable entry point.

fn main() {
    println!("taste {}", env!("CARGO_PKG_VERSION"));
}

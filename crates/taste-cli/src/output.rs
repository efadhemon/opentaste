//! Human-facing output, silenced by `--quiet`.
//!
//! Only informational lines go through here. `learn-delta` and `mcp` write
//! their machine-readable stdout directly, and errors always reach stderr.

use std::io::Write;

pub struct Output {
    quiet: bool,
}

impl Output {
    pub fn new(quiet: bool) -> Self {
        Output { quiet }
    }

    /// Print one line unless `--quiet` was given.
    ///
    /// A reader that closes early (`taste status | head`) is not an error:
    /// the command stops quietly instead of panicking the way `println!` does
    /// on a broken pipe.
    pub fn say(&self, line: impl AsRef<str>) {
        if self.quiet {
            return;
        }
        let mut stdout = std::io::stdout().lock();
        if let Err(err) = writeln!(stdout, "{}", line.as_ref()) {
            if err.kind() == std::io::ErrorKind::BrokenPipe {
                std::process::exit(0);
            }
        }
    }
}

/// Marks for the status table and the dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyphs {
    pub confirmed: &'static str,
    pub candidate: &'static str,
    pub off: &'static str,
    pub bar_full: &'static str,
    pub bar_empty: &'static str,
}

pub const UNICODE: Glyphs = Glyphs {
    confirmed: "[✓]",
    candidate: "[?]",
    off: "[ ]",
    bar_full: "█",
    bar_empty: "░",
};

pub const ASCII: Glyphs = Glyphs {
    confirmed: "[x]",
    candidate: "[?]",
    off: "[ ]",
    bar_full: "#",
    bar_empty: ".",
};

/// Unicode everywhere except the legacy Windows console, which renders `✓`
/// and `█` as mojibake under a non-UTF-8 codepage. Windows Terminal sets
/// `WT_SESSION` and is fine. `TASTE_ASCII=1` forces ASCII anywhere.
pub fn glyphs() -> Glyphs {
    let forced = std::env::var_os("TASTE_ASCII").is_some_and(|value| value != "0");
    let legacy_console = cfg!(windows) && std::env::var_os("WT_SESSION").is_none();
    if forced || legacy_console {
        ASCII
    } else {
        UNICODE
    }
}

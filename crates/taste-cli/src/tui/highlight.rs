//! Syntax highlighting for the dashboard's avoid and prefer snippets.
//!
//! The grammars the scanner already parses with, and each one's bundled
//! `highlights.scm`, through `tree-sitter-highlight`: no second highlighter
//! and no grammar the rest of OpenTaste does not use. The language comes from
//! the rule's scope; a scope that names none (`**/*`) gets no highlighting,
//! and neither does a terminal with `NO_COLOR` set.

use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use taste_core::ast::Family;
use taste_core::SupportedLanguage;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

/// The capture names styled, in the order [`style_of`] indexes them.
const NAMES: &[&str] = &[
    "keyword",
    "string",
    "comment",
    "number",
    "constant",
    "type",
    "function",
    "attribute",
];

fn style_of(index: usize) -> Style {
    match NAMES.get(index) {
        Some(&"keyword") => Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
        Some(&"string") => Style::default().fg(Color::Yellow),
        Some(&"comment") => Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
        Some(&"number" | &"constant") => Style::default().fg(Color::Cyan),
        Some(&"type") => Style::default().fg(Color::Blue),
        Some(&"function") => Style::default().fg(Color::LightBlue),
        Some(&"attribute") => Style::default().fg(Color::LightMagenta),
        _ => Style::default(),
    }
}

/// Whether the terminal wants colour: `NO_COLOR` set to anything non-empty
/// turns it off (<https://no-color.org>).
pub fn colours_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty())
}

/// One compiled configuration per family, built on first use.
fn configuration(lang: SupportedLanguage) -> Option<&'static HighlightConfiguration> {
    static CONFIGS: [OnceLock<Option<HighlightConfiguration>>; 4] = [
        OnceLock::new(),
        OnceLock::new(),
        OnceLock::new(),
        OnceLock::new(),
    ];
    let slot = match lang.family() {
        Family::Script => &CONFIGS[0],
        Family::Python => &CONFIGS[1],
        Family::Rust => &CONFIGS[2],
        Family::Go => &CONFIGS[3],
    };
    slot.get_or_init(|| {
        // TypeScript's grammar parses plain JavaScript snippets too.
        let lang = match lang.family() {
            Family::Script => SupportedLanguage::TypeScript,
            _ => lang,
        };
        let mut config = HighlightConfiguration::new(
            lang.language(),
            lang.name(),
            &lang.highlights_query(),
            "",
            "",
        )
        .ok()?;
        config.configure(NAMES);
        Some(config)
    })
    .as_ref()
}

/// `snippet` as lines of styled spans, or `None` when the scope names no
/// language or the snippet cannot be highlighted; the caller then falls back
/// to plain colouring.
pub fn highlight(snippet: &str, scope: &str) -> Option<Vec<Line<'static>>> {
    let lang = SupportedLanguage::from_scope(scope)?;
    let config = configuration(lang)?;
    let mut highlighter = Highlighter::new();
    let events = highlighter
        .highlight(config, snippet.as_bytes(), None, None, |_| None)
        .ok()?;
    let mut lines = vec![Line::default()];
    let mut styles: Vec<Style> = Vec::new();
    for event in events {
        match event.ok()? {
            HighlightEvent::HighlightStart(highlight) => styles.push(style_of(highlight.0)),
            HighlightEvent::HighlightEnd => {
                styles.pop();
            }
            HighlightEvent::Source { start, end } => {
                let style = styles.last().copied().unwrap_or_default();
                let text = snippet.get(start..end)?;
                for (index, part) in text.split('\n').enumerate() {
                    if index > 0 {
                        lines.push(Line::default());
                    }
                    if !part.is_empty() {
                        if let Some(line) = lines.last_mut() {
                            line.spans.push(Span::styled(part.to_string(), style));
                        }
                    }
                }
            }
        }
    }
    if snippet.ends_with('\n') && lines.last().is_some_and(|line| line.spans.is_empty()) {
        lines.pop();
    }
    Some(lines)
}

/// A snippet's lines as the detail pane shows them: highlighted when the
/// scope names a language, in `fallback` when it does not, and plain when
/// `colours` is off.
pub fn snippet_lines(
    text: &str,
    scope: &str,
    colours: bool,
    fallback: Color,
) -> Vec<Line<'static>> {
    if !colours {
        return text
            .lines()
            .map(|line| Line::raw(line.to_string()))
            .collect();
    }
    highlight(text, scope).unwrap_or_else(|| {
        text.lines()
            .map(|line| Line::styled(line.to_string(), Style::default().fg(fallback)))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styled(lines: &[Line<'_>], text: &str) -> Option<Style> {
        lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .find(|span| span.content == text)
            .map(|span| span.style)
    }

    #[test]
    fn a_typescript_snippet_gets_keyword_and_string_styles() {
        let lines = highlight(
            "const user = await load(\"users\");\nreturn user;",
            "src/**/*.ts",
        )
        .unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(styled(&lines, "const"), Some(style_of(0)));
        assert_eq!(styled(&lines, "\"users\""), Some(style_of(1)));
        assert_eq!(styled(&lines, "return"), Some(style_of(0)));
    }

    #[test]
    fn python_rust_and_go_snippets_are_highlighted_too() {
        for (snippet, scope, keyword) in [
            ("def f():\n    return \"x\"", "**/*.py", "def"),
            ("fn f() -> u8 { return 1; }", "**/*.rs", "fn"),
            ("func f() int { return 1 }", "**/*.go", "func"),
        ] {
            let lines = highlight(snippet, scope).unwrap();
            assert_eq!(styled(&lines, keyword), Some(style_of(0)), "{scope}");
        }
    }

    #[test]
    fn a_scope_that_names_no_language_falls_back() {
        assert!(highlight("console.log(x)", "**/*").is_none());
        assert!(highlight("console.log(x)", "**/*.md").is_none());
    }

    #[test]
    fn no_colour_means_no_styles_and_an_unknown_language_keeps_its_box_colour() {
        let plain = snippet_lines("const x = \"a\";", "**/*.ts", false, Color::Red);
        assert!(plain
            .iter()
            .flat_map(|line| line.spans.iter())
            .all(|span| span.style == Style::default()));
        assert!(plain.iter().all(|line| line.style == Style::default()));

        let fallback = snippet_lines("x = 1", "**/*", true, Color::Green);
        assert_eq!(fallback[0].style, Style::default().fg(Color::Green));
    }

    #[test]
    fn the_text_survives_highlighting_unchanged() {
        let snippet = "@Entity('users')\nexport class User {\n  id: string;\n}";
        let lines = highlight(snippet, "src/**/*.entity.ts").unwrap();
        let text: Vec<String> = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect()
            })
            .collect();
        assert_eq!(text.join("\n"), snippet);
    }
}

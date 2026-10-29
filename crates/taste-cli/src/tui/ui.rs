//! Draw the dashboard from an [`App`]. Nothing here changes state.
//!
//! Left pane 45%: confirmed `[✓]` and candidate `[?]` rules with a confidence
//! bar and category badge. Right pane 55%: scope, occurrences and
//! instruction, then the avoid/use pair. Bottom: the keys and the last status
//! message. Snippets are coloured red and green rather than fully syntax
//! highlighted; they are capped at a dozen lines, so a highlighter would add a
//! dependency for very little.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::output::Glyphs;
use crate::tui::app::{App, Entry, Mode};

const BAR_WIDTH: usize = 5;

pub fn draw(frame: &mut Frame, app: &App, glyphs: &Glyphs) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(2)])
        .split(frame.area());
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(rows[0]);

    draw_list(frame, panes[0], app, glyphs);
    draw_detail(frame, panes[1], app);
    draw_status(frame, rows[1], app);
}

pub fn confidence_bar(confidence: f32, glyphs: &Glyphs) -> String {
    let filled = ((confidence.clamp(0.0, 1.0) * BAR_WIDTH as f32).round() as usize).min(BAR_WIDTH);
    format!(
        "{}{} {:>3.0}%",
        glyphs.bar_full.repeat(filled),
        glyphs.bar_empty.repeat(BAR_WIDTH - filled),
        confidence * 100.0
    )
}

fn category_colour(entry: &Entry) -> Color {
    use taste_core::RuleCategory::*;
    match entry.category() {
        Architecture => Color::Magenta,
        CodeStyle => Color::Cyan,
        Imports => Color::Blue,
        Types => Color::Yellow,
        ErrorHandling => Color::Red,
        Naming => Color::Green,
        Testing => Color::LightBlue,
    }
}

fn draw_list(frame: &mut Frame, area: Rect, app: &App, glyphs: &Glyphs) {
    let items: Vec<ListItem> = app
        .visible()
        .map(|entry| {
            let mark = if entry.is_candidate() {
                glyphs.candidate
            } else if entry.is_enabled() {
                glyphs.confirmed
            } else {
                glyphs.off
            };
            let dim = if entry.is_enabled() {
                Style::default()
            } else {
                Style::default().add_modifier(Modifier::DIM)
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{mark} "), dim),
                Span::styled(
                    format!("{} ", confidence_bar(entry.confidence(), glyphs)),
                    dim,
                ),
                Span::styled(
                    format!("{:<14}", entry.category().slug()),
                    Style::default().fg(category_colour(entry)),
                ),
                Span::styled(entry.instruction().to_string(), dim),
            ]))
        })
        .collect();

    let (confirmed, candidates) = app.counts();
    let mut title = format!(
        " Rules · {confirmed} confirmed · {candidates} candidate{} ",
        if candidates == 1 { "" } else { "s" }
    );
    if !app.filter.is_empty() || app.mode == Mode::Filter {
        let cursor = if app.mode == Mode::Filter { "_" } else { "" };
        title.push_str(&format!("· filter: {}{cursor} ", app.filter));
    }
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("› ");
    let mut state = ListState::default();
    state.select(app.selected_index());
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_detail(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title(" Detail ");
    let Some(entry) = app.selected() else {
        let empty = Paragraph::new("Nothing selected.").block(block);
        frame.render_widget(empty, area);
        return;
    };
    let label = Style::default().add_modifier(Modifier::BOLD);
    let state = if entry.is_candidate() {
        "candidate (not applied to editors)"
    } else if entry.is_enabled() {
        "confirmed, enabled"
    } else {
        "confirmed, disabled"
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled("Scope        ", label),
            Span::raw(entry.scope().to_string()),
        ]),
        Line::from(vec![
            Span::styled("Occurrences  ", label),
            Span::raw(entry.occurrences().to_string()),
        ]),
        Line::from(vec![
            Span::styled("Confidence   ", label),
            Span::raw(format!("{:.0}%", entry.confidence() * 100.0)),
        ]),
        Line::from(vec![Span::styled("State        ", label), Span::raw(state)]),
        Line::from(vec![
            Span::styled("Source       ", label),
            Span::raw(entry.source()),
        ]),
        Line::from(vec![
            Span::styled("Id           ", label),
            Span::raw(entry.id().to_string()),
        ]),
        Line::raw(""),
        Line::styled("Instruction", label),
        Line::raw(entry.instruction().to_string()),
    ];
    let snippet = |title: &str, text: Option<&str>, colour: Color, lines: &mut Vec<Line>| {
        let Some(text) = text else {
            return;
        };
        lines.push(Line::raw(""));
        lines.push(Line::styled(title.to_string(), label.fg(colour)));
        for line in text.lines() {
            lines.push(Line::styled(
                format!("  {line}"),
                Style::default().fg(colour),
            ));
        }
    };
    snippet(
        "Anti-Pattern (Avoid)",
        entry.anti_pattern(),
        Color::Red,
        &mut lines,
    );
    snippet(
        "Preferred-Pattern (Use)",
        entry.preferred_pattern(),
        Color::Green,
        &mut lines,
    );

    let detail = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(detail, area);
}

pub const KEYS: &str =
    "[↑/↓] Navigate | [Space] Toggle Enable | [p] Promote Candidate | [d] Delete | [s] Sync to Editors | [/] Filter | [q] Quit";

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let keys = match &app.mode {
        Mode::ConfirmDelete(_) => "[y] Delete | any other key cancels".to_string(),
        Mode::Filter => "[Enter] Keep filter | [Esc] Clear filter | [Backspace] Edit".to_string(),
        Mode::Normal => KEYS.to_string(),
    };
    let status_style = if matches!(app.mode, Mode::ConfirmDelete(_)) {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Yellow)
    };
    let bar = Paragraph::new(vec![
        Line::styled(keys, Style::default().add_modifier(Modifier::DIM)),
        Line::styled(app.status.clone(), status_style),
    ]);
    frame.render_widget(bar, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::{ASCII, UNICODE};
    use crate::tui::app::Key;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use taste_core::config::Paths;
    use taste_core::models::RuleSource;
    use taste_core::{Observation, RuleCategory, RuleStore};

    fn screen(app: &App, glyphs: &Glyphs) -> String {
        let mut terminal = Terminal::new(TestBackend::new(140, 24)).unwrap();
        terminal.draw(|frame| draw(frame, app, glyphs)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let mut text = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn the_bar_fills_in_proportion_to_confidence() {
        assert_eq!(confidence_bar(0.88, &UNICODE), "████░  88%");
        assert_eq!(confidence_bar(1.0, &ASCII), "##### 100%");
        assert_eq!(confidence_bar(0.0, &ASCII), ".....   0%");
    }

    #[test]
    fn the_dashboard_shows_list_detail_and_keys() {
        let dir = tempfile::tempdir().unwrap();
        let store = RuleStore::open_ephemeral(Paths::for_root(dir.path())).unwrap();
        store
            .record_observation(
                Observation::new(
                    RuleCategory::Architecture,
                    "src/**/*.entity.ts",
                    RuleSource::Manual,
                    "Use @AutoEntity instead of @Entity.",
                )
                .with_patterns(
                    Some("@Entity(\"users\")".into()),
                    Some("@AutoEntity()".into()),
                ),
            )
            .unwrap();
        store
            .record_observation(Observation::new(
                RuleCategory::Types,
                "**/*.ts",
                RuleSource::AstDiff,
                "Prefer interface for object types.",
            ))
            .unwrap();
        let mut app = App::load(&store).unwrap();

        let text = screen(&app, &UNICODE);
        assert!(text.contains("[✓]") && text.contains("[?]"), "{text}");
        assert!(
            text.contains("Use @AutoEntity instead of @Entity."),
            "{text}"
        );
        assert!(text.contains("src/**/*.entity.ts"), "{text}");
        assert!(
            text.contains("Anti-Pattern (Avoid)") && text.contains("@Entity(\"users\")"),
            "{text}"
        );
        assert!(text.contains("Preferred-Pattern (Use)"), "{text}");
        assert!(text.contains("[Space] Toggle Enable"), "{text}");

        app.on_key(Key::Char('d'), &store);
        let confirming = screen(&app, &ASCII);
        assert!(confirming.contains("[y] Delete"), "{confirming}");
        assert!(confirming.contains("[x]"), "ascii glyphs: {confirming}");
    }
}

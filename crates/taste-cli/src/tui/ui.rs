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
use crate::tui::app::{App, EditState, Entry, Field, Mode};
use crate::tui::highlight;

const BAR_WIDTH: usize = 5;

pub fn draw(frame: &mut Frame, app: &App, glyphs: &Glyphs) {
    let area = frame.area();
    let bottom = status_lines(app, usize::from(area.width));
    // The bar grows to fit a narrow terminal, but never squeezes the panes
    // below five rows.
    let height = (bottom.len() as u16)
        .min(area.height.saturating_sub(5))
        .max(1);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(height)])
        .split(area);
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(rows[0]);

    draw_list(frame, panes[0], app, glyphs);
    draw_detail(frame, panes[1], app);
    frame.render_widget(Paragraph::new(bottom), rows[1]);
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
            let mark = if entry.is_personal_row() {
                glyphs.personal
            } else if entry.is_candidate() {
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
                Span::styled(
                    if app.is_personal(entry.id()) {
                        "  (personal)"
                    } else {
                        ""
                    },
                    Style::default().add_modifier(Modifier::ITALIC),
                ),
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

/// The two editable fields, the focused one with a `▏` at the cursor.
fn draw_edit(frame: &mut Frame, area: Rect, state: &EditState) {
    let label = Style::default().add_modifier(Modifier::BOLD);
    let field = |name: &str, text: &str, focused: bool| {
        let mut shown = text.to_string();
        if focused {
            let at = shown
                .char_indices()
                .nth(state.cursor)
                .map_or(shown.len(), |(i, _)| i);
            shown.insert(at, '▏');
        }
        let style = if focused {
            Style::default().add_modifier(Modifier::UNDERLINED)
        } else {
            Style::default()
        };
        vec![
            Line::styled(name.to_string(), label),
            Line::styled(shown, style),
            Line::raw(""),
        ]
    };
    let mut lines = vec![Line::raw(format!("Editing {}", state.id)), Line::raw("")];
    lines.extend(field(
        "Instruction",
        &state.instruction,
        state.field == Field::Instruction,
    ));
    lines.extend(field(
        "Scope (glob)",
        &state.scope,
        state.field == Field::Scope,
    ));
    let detail = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Edit "))
        .wrap(Wrap { trim: false });
    frame.render_widget(detail, area);
}

fn draw_detail(frame: &mut Frame, area: Rect, app: &App) {
    if let Mode::Edit(state) = &app.mode {
        draw_edit(frame, area, state);
        return;
    }
    let block = Block::default().borders(Borders::ALL).title(" Detail ");
    let Some(entry) = app.selected() else {
        let empty = Paragraph::new("Nothing selected.").block(block);
        frame.render_widget(empty, area);
        return;
    };
    let label = Style::default().add_modifier(Modifier::BOLD);
    let state = if entry.is_personal_row() {
        "personal, from another project (served where its scope matches)"
    } else if app.is_personal(entry.id()) {
        "confirmed here, and personal"
    } else if entry.is_candidate() {
        "candidate (not applied to editors)"
    } else if entry.is_enabled() {
        "confirmed, enabled"
    } else {
        "confirmed, disabled"
    };
    let lines = vec![
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
    // The avoid and prefer snippets get boxes of their own: the border and
    // title carry the red/green distinction, so the text can be coloured by
    // syntax.
    let colours = highlight::colours_enabled();
    let snippets: Vec<(&str, &str, Color)> = [
        (
            "Anti-Pattern (Avoid)",
            entry.anti_pattern(),
            highlight::AVOID,
        ),
        (
            "Preferred-Pattern (Use)",
            entry.preferred_pattern(),
            highlight::PREFER,
        ),
    ]
    .into_iter()
    .filter_map(|(title, text, colour)| text.map(|text| (title, text, colour)))
    .collect();
    let mut constraints = vec![Constraint::Min(3)];
    constraints.extend(
        snippets
            .iter()
            .map(|(_, text, _)| Constraint::Length(text.lines().count() as u16 + 2)),
    );
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let detail = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(detail, parts[0]);
    for ((title, text, colour), part) in snippets.into_iter().zip(parts.iter().skip(1)) {
        let border = if colours {
            Style::default().fg(colour)
        } else {
            Style::default()
        };
        let body = highlight::snippet_lines(text, entry.scope(), colours, colour);
        let snippet = Paragraph::new(body).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border)
                .title(format!(" {title} ")),
        );
        frame.render_widget(snippet, *part);
    }
}

pub const KEYS: &str =
    "[↑/↓] Navigate | [Space] Toggle Enable | [e] Edit | [g] Personal | [p] Promote Candidate | [d] Delete | [s] Sync to Editors | [/] Filter | [q] Quit";

/// The key bar and the status message, as the lines a terminal `width`
/// columns wide shows. Keys are wrapped between entries, never inside one, so
/// a narrow terminal shows every key on more lines instead of cutting the
/// last ones off; the message wraps between words.
fn status_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let keys = match &app.mode {
        Mode::ConfirmDelete(_) => "[y] Delete | any other key cancels".to_string(),
        Mode::Edit(_) => {
            "[Tab] Switch field | [←/→] Move | [Enter] Save and sync | [Esc] Cancel".to_string()
        }
        Mode::Filter => "[Enter] Keep filter | [Esc] Clear filter | [Backspace] Edit".to_string(),
        Mode::Normal => KEYS.to_string(),
    };
    let status_style = if matches!(app.mode, Mode::ConfirmDelete(_)) {
        Style::default()
            .fg(highlight::AVOID)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(highlight::STATUS)
    };
    let dim = Style::default().add_modifier(Modifier::DIM);
    let mut lines: Vec<Line<'static>> = wrap(keys.split(" | "), " | ", width)
        .into_iter()
        .map(|line| Line::styled(line, dim))
        .collect();
    if !app.status.is_empty() {
        lines.extend(
            wrap(app.status.split_whitespace(), " ", width)
                .into_iter()
                .map(|line| Line::styled(line, status_style)),
        );
    }
    lines
}

/// `items` joined by `separator` into lines of at most `width` columns,
/// breaking only between items. An item wider than a line gets one to itself.
fn wrap<'a>(items: impl Iterator<Item = &'a str>, separator: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for item in items {
        let joined = current.chars().count() + separator.chars().count() + item.chars().count();
        if current.is_empty() {
            current = item.to_string();
        } else if joined <= width {
            current.push_str(separator);
            current.push_str(item);
        } else {
            lines.push(std::mem::take(&mut current));
            current = item.to_string();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
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
        screen_sized(app, glyphs, 140, 24)
    }

    fn screen_sized(app: &App, glyphs: &Glyphs, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
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

    #[test]
    fn the_edit_pane_shows_both_fields_and_its_keys() {
        let dir = tempfile::tempdir().unwrap();
        let store = RuleStore::open_ephemeral(Paths::for_root(dir.path())).unwrap();
        store
            .record_observation(Observation::new(
                RuleCategory::Types,
                "**/*.ts",
                RuleSource::Manual,
                "Prefer interface for object types.",
            ))
            .unwrap();
        let mut app = App::load(&store).unwrap();
        app.on_key(Key::Char('e'), &store);
        let text = screen(&app, &UNICODE);
        assert!(text.contains(" Edit "), "{text}");
        assert!(
            text.contains("Scope (glob)") && text.contains("**/*.ts"),
            "{text}"
        );
        assert!(
            text.contains("object types.▏"),
            "the cursor sits at the end: {text}"
        );
        assert!(text.contains("[Enter] Save and sync"), "{text}");
        assert!(screen(&App::load(&store).unwrap(), &UNICODE).contains("[e] Edit"));
    }

    #[test]
    fn keys_wrap_between_entries_and_every_line_fits() {
        let lines = wrap(KEYS.split(" | "), " | ", 40);
        assert!(lines.len() > 1);
        for line in &lines {
            assert!(line.chars().count() <= 40, "{line:?}");
            assert!(!line.starts_with(" |") && !line.ends_with("| "), "{line:?}");
        }
        let rejoined = lines.join(" | ");
        assert_eq!(rejoined, KEYS, "nothing is lost or reordered");
        assert_eq!(wrap(KEYS.split(" | "), " | ", 1000), vec![KEYS.to_string()]);
        assert_eq!(
            wrap(["a-very-long-entry", "b"].into_iter(), " | ", 5),
            vec!["a-very-long-entry".to_string(), "b".to_string()],
            "an entry wider than the line gets one to itself"
        );
    }

    #[test]
    fn a_narrow_terminal_still_shows_every_key_and_the_whole_status() {
        let dir = tempfile::tempdir().unwrap();
        let store = RuleStore::open_ephemeral(Paths::for_root(dir.path())).unwrap();
        store
            .record_observation(Observation::new(
                RuleCategory::Types,
                "**/*.ts",
                RuleSource::Manual,
                "Prefer interface for object types.",
            ))
            .unwrap();
        let mut app = App::load(&store).unwrap();
        let narrow = screen_sized(&app, &UNICODE, 60, 24);
        for key in KEYS.split(" | ") {
            assert!(narrow.contains(key), "{key:?} is cut off:\n{narrow}");
        }

        app.on_key(Key::Char('d'), &store);
        let confirming = screen_sized(&app, &ASCII, 60, 24);
        let flat: String = confirming.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat.contains("anything else cancels."), "{confirming}");
    }
}

//! Dashboard state, with no terminal in sight.
//!
//! Every action is a method on [`App`] driven by [`Key`], this module's own
//! key type, so navigation, filtering, toggling, promotion and deletion are
//! tested without a PTY. Rendering lives in `ui.rs`; translating terminal
//! events into [`Key`]s lives in `mod.rs`.

use taste_core::{CandidateRule, RuleCategory, RuleStore, TasteRule};

/// The keys the dashboard understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Home,
    End,
    Enter,
    Esc,
    Backspace,
    Char(char),
}

/// A confirmed rule or a candidate, as one row of the list.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Confirmed(TasteRule),
    Candidate(CandidateRule),
}

impl Entry {
    pub fn id(&self) -> &str {
        match self {
            Entry::Confirmed(rule) => &rule.id,
            Entry::Candidate(candidate) => &candidate.id,
        }
    }

    pub fn category(&self) -> RuleCategory {
        match self {
            Entry::Confirmed(rule) => rule.category,
            Entry::Candidate(candidate) => candidate.category,
        }
    }

    pub fn scope(&self) -> &str {
        match self {
            Entry::Confirmed(rule) => &rule.scope,
            Entry::Candidate(candidate) => &candidate.scope,
        }
    }

    pub fn confidence(&self) -> f32 {
        match self {
            Entry::Confirmed(rule) => rule.confidence,
            Entry::Candidate(candidate) => candidate.confidence,
        }
    }

    pub fn occurrences(&self) -> u32 {
        match self {
            Entry::Confirmed(rule) => rule.occurrences,
            Entry::Candidate(candidate) => candidate.occurrences,
        }
    }

    pub fn instruction(&self) -> &str {
        match self {
            Entry::Confirmed(rule) => &rule.instruction,
            Entry::Candidate(candidate) => &candidate.instruction,
        }
    }

    pub fn anti_pattern(&self) -> Option<&str> {
        match self {
            Entry::Confirmed(rule) => rule.anti_pattern.as_deref(),
            Entry::Candidate(candidate) => candidate.anti_pattern.as_deref(),
        }
    }

    pub fn preferred_pattern(&self) -> Option<&str> {
        match self {
            Entry::Confirmed(rule) => rule.preferred_pattern.as_deref(),
            Entry::Candidate(candidate) => candidate.preferred_pattern.as_deref(),
        }
    }

    pub fn source(&self) -> &'static str {
        match self {
            Entry::Confirmed(rule) => rule.source.slug(),
            Entry::Candidate(candidate) => candidate.source.slug(),
        }
    }

    pub fn is_candidate(&self) -> bool {
        matches!(self, Entry::Candidate(_))
    }

    /// Enabled, for a confirmed rule. Candidates are never applied.
    pub fn is_enabled(&self) -> bool {
        matches!(self, Entry::Confirmed(rule) if rule.enabled)
    }

    fn matches(&self, filter: &str) -> bool {
        let filter = filter.to_lowercase();
        [
            self.instruction(),
            self.id(),
            self.scope(),
            self.category().slug(),
        ]
        .iter()
        .any(|field| field.to_lowercase().contains(&filter))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// Typing into the filter.
    Filter,
    /// `[d]` was pressed on this id; only `y` deletes.
    ConfirmDelete(String),
}

#[derive(Debug)]
pub struct App {
    entries: Vec<Entry>,
    /// Indices into `entries` that pass the filter, in display order.
    visible: Vec<usize>,
    /// Position within `visible`.
    selected: usize,
    pub filter: String,
    pub mode: Mode,
    pub status: String,
    pub quit: bool,
}

impl App {
    pub fn load(store: &RuleStore) -> anyhow::Result<Self> {
        let mut app = App {
            entries: Vec::new(),
            visible: Vec::new(),
            selected: 0,
            filter: String::new(),
            mode: Mode::Normal,
            status: String::new(),
            quit: false,
        };
        app.reload(store)?;
        if app.entries.is_empty() {
            app.status =
                "No rules yet. Run `taste scan`, or record a preference in chat.".to_string();
        }
        Ok(app)
    }

    /// Re-read the store, keeping the selection on the same rule when it
    /// still exists.
    pub fn reload(&mut self, store: &RuleStore) -> anyhow::Result<()> {
        let keep = self.selected().map(|entry| entry.id().to_string());
        self.entries = store
            .rules()?
            .rules
            .into_iter()
            .map(Entry::Confirmed)
            .chain(store.candidate_rules()?.into_iter().map(Entry::Candidate))
            .collect();
        self.refilter(keep.as_deref());
        Ok(())
    }

    fn refilter(&mut self, keep: Option<&str>) {
        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| self.filter.is_empty() || entry.matches(&self.filter))
            .map(|(index, _)| index)
            .collect();
        let kept = keep.and_then(|id| {
            self.visible
                .iter()
                .position(|index| self.entries[*index].id() == id)
        });
        self.selected = kept
            .unwrap_or(self.selected)
            .min(self.visible.len().saturating_sub(1));
    }

    pub fn visible(&self) -> impl Iterator<Item = &Entry> {
        self.visible.iter().map(|index| &self.entries[*index])
    }

    pub fn selected_index(&self) -> Option<usize> {
        (!self.visible.is_empty()).then_some(self.selected)
    }

    pub fn selected(&self) -> Option<&Entry> {
        self.visible
            .get(self.selected)
            .map(|index| &self.entries[*index])
    }

    pub fn counts(&self) -> (usize, usize) {
        let candidates = self
            .entries
            .iter()
            .filter(|entry| entry.is_candidate())
            .count();
        (self.entries.len() - candidates, candidates)
    }

    /// Apply one key. Store failures become a status message rather than
    /// ending the session.
    pub fn on_key(&mut self, key: Key, store: &RuleStore) {
        if let Err(err) = self.handle(key, store) {
            self.status = format!("Error: {err:#}");
        }
    }

    fn handle(&mut self, key: Key, store: &RuleStore) -> anyhow::Result<()> {
        match self.mode.clone() {
            Mode::ConfirmDelete(id) => {
                self.mode = Mode::Normal;
                if key == Key::Char('y') {
                    if store.delete_rule(&id)? {
                        self.status = format!("Deleted {id}. Press [s] to sync editors.");
                    } else {
                        self.status = format!("{id} no longer exists.");
                    }
                    self.reload(store)?;
                } else {
                    self.status = "Delete cancelled.".to_string();
                }
                Ok(())
            }
            Mode::Filter => {
                match key {
                    Key::Char(c) => self.filter.push(c),
                    Key::Backspace => {
                        self.filter.pop();
                    }
                    Key::Enter => self.mode = Mode::Normal,
                    Key::Esc => {
                        self.filter.clear();
                        self.mode = Mode::Normal;
                    }
                    Key::Up | Key::Down | Key::Home | Key::End => {
                        self.mode = Mode::Normal;
                        return self.handle(key, store);
                    }
                }
                let keep = self.selected().map(|entry| entry.id().to_string());
                self.refilter(keep.as_deref());
                Ok(())
            }
            Mode::Normal => self.normal(key, store),
        }
    }

    fn normal(&mut self, key: Key, store: &RuleStore) -> anyhow::Result<()> {
        match key {
            Key::Up | Key::Char('k') => self.selected = self.selected.saturating_sub(1),
            Key::Down | Key::Char('j') => {
                if self.selected + 1 < self.visible.len() {
                    self.selected += 1;
                }
            }
            Key::Home => self.selected = 0,
            Key::End => self.selected = self.visible.len().saturating_sub(1),
            Key::Char('/') => {
                self.mode = Mode::Filter;
                self.status = "Type to filter; [Enter] keeps it, [Esc] clears it.".to_string();
            }
            Key::Char(' ') => self.toggle(store)?,
            Key::Char('p') => self.promote(store)?,
            Key::Char('d') => {
                if let Some(entry) = self.selected() {
                    let id = entry.id().to_string();
                    self.status = format!("Delete {id}? This cannot be undone. Press [y] to confirm, anything else cancels.");
                    self.mode = Mode::ConfirmDelete(id);
                }
            }
            Key::Char('s') => {
                let report = store.sync()?;
                self.status = if report.is_empty() {
                    "Editor files were already up to date.".to_string()
                } else {
                    format!(
                        "Synced: {} updated, {} removed.",
                        report.written.len(),
                        report.removed.len()
                    )
                };
            }
            Key::Char('r') => {
                self.reload(store)?;
                self.status = "Reloaded.".to_string();
            }
            Key::Char('q') | Key::Esc => self.quit = true,
            _ => {}
        }
        Ok(())
    }

    fn toggle(&mut self, store: &RuleStore) -> anyhow::Result<()> {
        let Some(entry) = self.selected() else {
            return Ok(());
        };
        let Entry::Confirmed(rule) = entry else {
            self.status =
                "Candidates are not applied to editors yet; press [p] to promote.".to_string();
            return Ok(());
        };
        let (id, enable) = (rule.id.clone(), !rule.enabled);
        store.set_enabled(&id, enable)?;
        self.status = format!(
            "{} {id}. Press [s] to sync editors.",
            if enable { "Enabled" } else { "Disabled" }
        );
        self.reload(store)
    }

    fn promote(&mut self, store: &RuleStore) -> anyhow::Result<()> {
        let Some(entry) = self.selected() else {
            return Ok(());
        };
        if !entry.is_candidate() {
            self.status = "Already confirmed.".to_string();
            return Ok(());
        }
        let id = entry.id().to_string();
        self.status = match store.promote_candidate(&id)? {
            Some(rule) => format!("Promoted {}. Press [s] to sync editors.", rule.id),
            None => format!("{id} could not be promoted."),
        };
        self.reload(store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use taste_core::config::Paths;
    use taste_core::models::RuleSource;
    use taste_core::Observation;

    struct Fixture {
        _dir: tempfile::TempDir,
        store: RuleStore,
    }

    /// Two confirmed rules and one candidate.
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = RuleStore::open_ephemeral(Paths::for_root(dir.path())).unwrap();
        for (category, scope, instruction) in [
            (
                RuleCategory::Imports,
                "**/*.ts",
                "Prefer path aliases over deep relative imports.",
            ),
            (
                RuleCategory::Testing,
                "**/*.spec.ts",
                "Co-locate spec files beside the implementation.",
            ),
        ] {
            store
                .record_observation(Observation::new(
                    category,
                    scope,
                    RuleSource::Manual,
                    instruction,
                ))
                .unwrap();
        }
        store
            .record_observation(
                Observation::new(
                    RuleCategory::Architecture,
                    "src/**/*.entity.ts",
                    RuleSource::AstDiff,
                    "Use @AutoEntity instead of @Entity.",
                )
                .with_patterns(
                    Some("@Entity(\"users\")".into()),
                    Some("@AutoEntity()".into()),
                ),
            )
            .unwrap();
        Fixture { _dir: dir, store }
    }

    fn press(app: &mut App, store: &RuleStore, keys: &[Key]) {
        for key in keys {
            app.on_key(*key, store);
        }
    }

    fn selected_id(app: &App) -> String {
        app.selected().unwrap().id().to_string()
    }

    #[test]
    fn confirmed_rules_come_before_candidates() {
        let fx = fixture();
        let app = App::load(&fx.store).unwrap();
        let kinds: Vec<bool> = app.visible().map(Entry::is_candidate).collect();
        assert_eq!(kinds, vec![false, false, true]);
        assert_eq!(app.counts(), (2, 1));
    }

    #[test]
    fn navigation_stops_at_both_ends() {
        let fx = fixture();
        let mut app = App::load(&fx.store).unwrap();
        press(&mut app, &fx.store, &[Key::Up, Key::Up]);
        assert_eq!(app.selected_index(), Some(0));
        press(
            &mut app,
            &fx.store,
            &[Key::Down, Key::Down, Key::Down, Key::Down],
        );
        assert_eq!(app.selected_index(), Some(2));
        press(&mut app, &fx.store, &[Key::Home]);
        assert_eq!(app.selected_index(), Some(0));
        press(&mut app, &fx.store, &[Key::End]);
        assert_eq!(app.selected_index(), Some(2));
        press(&mut app, &fx.store, &[Key::Char('k')]);
        assert_eq!(app.selected_index(), Some(1));
    }

    #[test]
    fn an_empty_store_is_navigable_and_says_what_to_do() {
        let dir = tempfile::tempdir().unwrap();
        let store = RuleStore::open_ephemeral(Paths::for_root(dir.path())).unwrap();
        let mut app = App::load(&store).unwrap();
        press(
            &mut app,
            &store,
            &[
                Key::Down,
                Key::Up,
                Key::Char(' '),
                Key::Char('p'),
                Key::Char('d'),
            ],
        );
        assert_eq!(app.selected_index(), None);
        assert_eq!(app.mode, Mode::Normal, "nothing to delete");
        assert!(!app.quit);
    }

    #[test]
    fn filtering_narrows_the_list_and_escape_clears_it() {
        let fx = fixture();
        let mut app = App::load(&fx.store).unwrap();
        press(&mut app, &fx.store, &[Key::End, Key::Char('/')]);
        for c in "SPEC".chars() {
            app.on_key(Key::Char(c), &fx.store);
        }
        assert_eq!(app.filter, "SPEC");
        assert_eq!(
            app.visible().count(),
            1,
            "case-insensitive match on scope and text"
        );
        assert_eq!(
            app.selected_index(),
            Some(0),
            "selection clamps into the narrower list"
        );

        press(
            &mut app,
            &fx.store,
            &[
                Key::Backspace,
                Key::Backspace,
                Key::Backspace,
                Key::Backspace,
            ],
        );
        assert_eq!(app.visible().count(), 3);
        for c in "nothing-matches".chars() {
            app.on_key(Key::Char(c), &fx.store);
        }
        assert_eq!(app.selected(), None);

        press(&mut app, &fx.store, &[Key::Esc]);
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.filter.is_empty());
        assert_eq!(app.visible().count(), 3);
        assert!(!app.quit, "Esc in the filter does not quit");
    }

    #[test]
    fn letters_typed_into_the_filter_are_not_commands() {
        let fx = fixture();
        let mut app = App::load(&fx.store).unwrap();
        press(
            &mut app,
            &fx.store,
            &[
                Key::Char('/'),
                Key::Char('d'),
                Key::Char('q'),
                Key::Char(' '),
            ],
        );
        assert_eq!(app.filter, "dq ");
        assert!(!app.quit);
        assert_eq!(app.mode, Mode::Filter);
    }

    #[test]
    fn space_toggles_a_confirmed_rule_and_keeps_the_selection() {
        let fx = fixture();
        let mut app = App::load(&fx.store).unwrap();
        press(&mut app, &fx.store, &[Key::Down]);
        let id = selected_id(&app);
        press(&mut app, &fx.store, &[Key::Char(' ')]);
        assert!(!fx.store.rules().unwrap().find(&id).unwrap().enabled);
        assert_eq!(selected_id(&app), id);
        assert!(app.status.contains("Disabled"), "{}", app.status);
        press(&mut app, &fx.store, &[Key::Char(' ')]);
        assert!(fx.store.rules().unwrap().find(&id).unwrap().enabled);
    }

    #[test]
    fn space_on_a_candidate_explains_instead_of_acting() {
        let fx = fixture();
        let mut app = App::load(&fx.store).unwrap();
        press(&mut app, &fx.store, &[Key::End, Key::Char(' ')]);
        assert!(app.status.contains("promote"), "{}", app.status);
        assert_eq!(app.counts(), (2, 1));
    }

    #[test]
    fn p_promotes_a_candidate_into_the_confirmed_rules() {
        let fx = fixture();
        let mut app = App::load(&fx.store).unwrap();
        press(&mut app, &fx.store, &[Key::End]);
        let id = selected_id(&app);
        press(&mut app, &fx.store, &[Key::Char('p')]);
        assert!(fx.store.rules().unwrap().find(&id).is_some());
        assert_eq!(app.counts(), (3, 0));
        assert_eq!(
            selected_id(&app),
            id,
            "the selection follows the promoted rule"
        );
        assert!(!app.selected().unwrap().is_candidate());

        press(&mut app, &fx.store, &[Key::Char('p')]);
        assert!(app.status.contains("Already confirmed"), "{}", app.status);
    }

    #[test]
    fn d_deletes_only_after_y_is_pressed() {
        let fx = fixture();
        let mut app = App::load(&fx.store).unwrap();
        let id = selected_id(&app);

        press(&mut app, &fx.store, &[Key::Char('d')]);
        assert_eq!(app.mode, Mode::ConfirmDelete(id.clone()));
        press(&mut app, &fx.store, &[Key::Char('n')]);
        assert_eq!(app.mode, Mode::Normal);
        assert!(fx.store.rules().unwrap().find(&id).is_some(), "cancelled");

        press(&mut app, &fx.store, &[Key::Char('d'), Key::Char('d')]);
        assert!(
            fx.store.rules().unwrap().find(&id).is_some(),
            "a second d is not a yes"
        );

        press(&mut app, &fx.store, &[Key::Char('d'), Key::Char('y')]);
        assert!(fx.store.rules().unwrap().find(&id).is_none());
        assert_eq!(app.counts(), (1, 1));
        assert!(app.status.contains("Deleted"), "{}", app.status);
    }

    #[test]
    fn s_syncs_and_reports_what_changed() {
        let fx = fixture();
        let mut app = App::load(&fx.store).unwrap();
        press(&mut app, &fx.store, &[Key::Char('s')]);
        assert!(app.status.starts_with("Synced"), "{}", app.status);
        assert!(fx.store.paths().claude_md().exists());
        press(&mut app, &fx.store, &[Key::Char('s')]);
        assert!(app.status.contains("already up to date"), "{}", app.status);
    }

    #[test]
    fn q_and_escape_quit() {
        let fx = fixture();
        for key in [Key::Char('q'), Key::Esc] {
            let mut app = App::load(&fx.store).unwrap();
            app.on_key(key, &fx.store);
            assert!(app.quit, "{key:?}");
        }
    }
}

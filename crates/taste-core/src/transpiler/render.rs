//! Shared markdown rendering for every transpiler target.
//!
//! All four targets present the same rules, so the bullet shape lives here once
//! rather than being reimplemented (and drifting) per target.
//!
//! Scope globs are wrapped in backticks rather than bold. A glob like
//! `src/**/*.entity.ts` contains `**`, which markdown reads as a bold delimiter;
//! quoting it as code sidesteps the escaping entirely and renders correctly in
//! every editor that reads these files.

use std::collections::BTreeMap;

use crate::models::{RuleCategory, TasteRule};

/// Glob covering the whole project.
pub const MATCH_ALL_SCOPE: &str = "**/*";

/// Group rules by category, in the canonical category order.
///
/// Within a category, rules keep the identifier order the rule file imposes, so
/// generated documents are stable across runs.
pub fn group_by_category(rules: &[TasteRule]) -> BTreeMap<RuleCategory, Vec<&TasteRule>> {
    let mut grouped: BTreeMap<RuleCategory, Vec<&TasteRule>> = BTreeMap::new();
    for rule in rules {
        grouped.entry(rule.category).or_default().push(rule);
    }
    for group in grouped.values_mut() {
        group.sort_by(|a, b| a.id.cmp(&b.id));
    }
    grouped
}

/// The sorted, de-duplicated scopes covered by a set of rules.
pub fn scopes_of(rules: &[&TasteRule]) -> Vec<String> {
    let mut scopes: Vec<String> = rules.iter().map(|r| r.scope.clone()).collect();
    scopes.sort();
    scopes.dedup();
    scopes
}

/// Render one rule as a markdown list item, with its examples beneath it.
pub fn rule_bullet(rule: &TasteRule) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "- **[`{}`]** {}",
        rule.scope,
        rule.instruction.trim()
    ));

    if let Some(anti) = non_empty(rule.anti_pattern.as_deref()) {
        out.push('\n');
        out.push_str(&example("Avoid", anti));
    }
    if let Some(preferred) = non_empty(rule.preferred_pattern.as_deref()) {
        out.push('\n');
        out.push_str(&example("Prefer", preferred));
    }

    out
}

/// Render the rules of one category as a markdown list.
pub fn category_body(rules: &[&TasteRule]) -> String {
    rules
        .iter()
        .map(|rule| rule_bullet(rule))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Render every category as a document body with `###` headings.
///
/// This is the shape injected into `CLAUDE.md`, `.windsurfrules`, `AGENTS.md`
/// and `.github/copilot-instructions.md`. Returns an empty string when there is
/// nothing to say.
pub fn sections(rules: &[TasteRule]) -> String {
    sections_under(
        "## Coding Taste & Project Conventions (Auto-Learned)\n",
        rules,
    )
}

/// [`sections`] for a user-level file: the developer's personal rules, which
/// a project's own conventions override.
pub fn personal_sections(rules: &[TasteRule]) -> String {
    sections_under(
        "## Personal Coding Preferences (OpenTaste)\n\nYour own preferences, across projects. Where a project's own conventions say otherwise, follow the project.\n",
        rules,
    )
}

fn sections_under(heading: &str, rules: &[TasteRule]) -> String {
    let grouped = group_by_category(rules);
    if grouped.is_empty() {
        return String::new();
    }

    let mut out = String::from(heading);
    for (category, group) in &grouped {
        out.push_str(&format!("\n### {}\n\n", category.title()));
        out.push_str(&category_body(group));
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// Render an `Avoid:` or `Prefer:` example beneath a bullet.
///
/// Single-line snippets stay inline as code spans; anything multi-line becomes
/// an indented fenced block, because an inline span would mangle it.
fn example(label: &str, snippet: &str) -> String {
    let snippet = snippet.trim_end();
    if snippet.contains('\n') {
        let indented = snippet
            .lines()
            .map(|line| format!("    {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("  - {label}:\n\n    ```\n{indented}\n    ```\n")
    } else {
        format!("  - {label}: {}", inline_code(snippet))
    }
}

/// Wrap a snippet in a markdown inline code span, fences included.
///
/// A snippet containing backticks needs a fence longer than the longest
/// backtick run inside it, plus padding spaces so a leading or trailing
/// backtick is not absorbed by the fence itself.
fn inline_code(snippet: &str) -> String {
    let longest_run = snippet.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    if longest_run == 0 {
        return format!("`{snippet}`");
    }
    let fence = "`".repeat(longest_run + 1);
    format!("{fence} {snippet} {fence}")
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{RuleSource, TasteRule};
    use chrono::{DateTime, Utc};
    use pretty_assertions::assert_eq;

    fn rule(id: &str, category: RuleCategory, scope: &str, instruction: &str) -> TasteRule {
        TasteRule {
            id: id.to_string(),
            category,
            scope: scope.to_string(),
            confidence: 0.95,
            occurrences: 4,
            enabled: true,
            source: RuleSource::AstDiff,
            instruction: instruction.to_string(),
            anti_pattern: None,
            preferred_pattern: None,
            updated_at: DateTime::parse_from_rfc3339("2026-10-29T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    #[test]
    fn a_scope_glob_is_quoted_as_code_so_its_stars_survive() {
        let bullet = rule_bullet(&rule(
            "arch-entity",
            RuleCategory::Architecture,
            "src/**/*.entity.ts",
            "Use custom @AutoEntity() decorators",
        ));

        assert_eq!(
            bullet,
            "- **[`src/**/*.entity.ts`]** Use custom @AutoEntity() decorators"
        );
        assert!(
            !bullet.contains('\\'),
            "backticks should remove the need for escaping: {bullet}"
        );
    }

    #[test]
    fn single_line_examples_render_inline() {
        let mut r = rule(
            "arch-api",
            RuleCategory::Architecture,
            "src/**/*.controller.ts",
            "Return an ApiResponse wrapper",
        );
        r.anti_pattern = Some("return this.userService.findAll();".to_string());
        r.preferred_pattern = Some("return ApiResponse.ok(result);".to_string());

        assert_eq!(
            rule_bullet(&r),
            "- **[`src/**/*.controller.ts`]** Return an ApiResponse wrapper\n  \
             - Avoid: `return this.userService.findAll();`\n  \
             - Prefer: `return ApiResponse.ok(result);`"
        );
    }

    #[test]
    fn multi_line_examples_become_fenced_blocks() {
        let mut r = rule(
            "arch-entity",
            RuleCategory::Architecture,
            "src/**/*.entity.ts",
            "Use custom decorators",
        );
        r.anti_pattern = Some("@Entity('users')\nexport class User {}".to_string());

        let bullet = rule_bullet(&r);
        assert!(bullet.contains("  - Avoid:\n\n    ```\n"), "{bullet}");
        assert!(bullet.contains("    @Entity('users')\n"), "{bullet}");
        assert!(
            bullet.contains("    export class User {}\n    ```"),
            "{bullet}"
        );
    }

    #[test]
    fn a_snippet_containing_backticks_gets_a_longer_fence() {
        let mut r = rule(
            "style-template",
            RuleCategory::CodeStyle,
            "**/*",
            "Use template literals",
        );
        r.preferred_pattern = Some("const s = `hi ${name}`;".to_string());

        let bullet = rule_bullet(&r);
        assert!(
            bullet.contains("  - Prefer: `` const s = `hi ${name}`; ``"),
            "{bullet}"
        );
    }

    #[test]
    fn blank_examples_are_omitted_rather_than_rendered_empty() {
        let mut r = rule("naming", RuleCategory::Naming, "**/*", "Name things well");
        r.anti_pattern = Some("   ".to_string());
        r.preferred_pattern = Some(String::new());

        assert_eq!(
            rule_bullet(&r),
            "- **[`**/*`]** Name things well",
            "whitespace-only examples should not produce empty bullets"
        );
    }

    #[test]
    fn categories_are_grouped_in_canonical_order_regardless_of_input_order() {
        let rules = vec![
            rule("t", RuleCategory::Testing, "**/*", "Test things"),
            rule("a", RuleCategory::Architecture, "**/*", "Architect things"),
            rule("i", RuleCategory::Imports, "**/*", "Import things"),
        ];

        let order: Vec<_> = group_by_category(&rules).keys().copied().collect();
        assert_eq!(
            order,
            vec![
                RuleCategory::Architecture,
                RuleCategory::Imports,
                RuleCategory::Testing
            ]
        );
    }

    #[test]
    fn rules_within_a_category_are_ordered_by_id() {
        let rules = vec![
            rule("z", RuleCategory::Architecture, "**/*", "Later"),
            rule("a", RuleCategory::Architecture, "**/*", "Earlier"),
        ];

        let grouped = group_by_category(&rules);
        let ids: Vec<_> = grouped[&RuleCategory::Architecture]
            .iter()
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(ids, vec!["a", "z"]);
    }

    #[test]
    fn scopes_are_collected_sorted_and_deduplicated() {
        let rules = vec![
            rule("b", RuleCategory::Architecture, "src/**/*.service.ts", "B"),
            rule("a", RuleCategory::Architecture, "src/**/*.entity.ts", "A"),
            rule("c", RuleCategory::Architecture, "src/**/*.entity.ts", "C"),
        ];
        let grouped = group_by_category(&rules);

        assert_eq!(
            scopes_of(&grouped[&RuleCategory::Architecture]),
            vec!["src/**/*.entity.ts", "src/**/*.service.ts"]
        );
    }

    #[test]
    fn an_empty_rule_set_renders_nothing_at_all() {
        assert_eq!(sections(&[]), "");
    }

    #[test]
    fn sections_carry_a_heading_per_category_and_no_trailing_blank_lines() {
        let rules = vec![
            rule(
                "arch-entity",
                RuleCategory::Architecture,
                "src/**/*.entity.ts",
                "Use custom @AutoEntity() decorators",
            ),
            rule(
                "style-early-return",
                RuleCategory::CodeStyle,
                "**/*",
                "Prefer early returns over nested if-else blocks",
            ),
        ];

        let body = sections(&rules);
        assert!(
            body.starts_with("## Coding Taste & Project Conventions"),
            "{body}"
        );
        assert!(body.contains("\n### Architecture\n\n"), "{body}");
        assert!(body.contains("\n### Code Style\n\n"), "{body}");
        assert!(!body.ends_with('\n'), "{body:?}");
        assert!(
            body.find("### Architecture") < body.find("### Code Style"),
            "{body}"
        );
    }
}

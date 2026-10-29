//! Cold-start scan over a synthetic repository.

mod common;

use common::Project;
use taste_core::ast::stored_tally;
use taste_core::ast::{PREFER_PATH_ALIASES, PY_PREFER_ANNOTATIONS, PY_PREFER_LOGGING};
use taste_core::scan;

fn write_imports(project: &Project, alias: usize, relative: usize) {
    for index in 0..alias {
        project.write(
            &format!("src/alias{index}.ts"),
            "import { a } from \"@/lib/a\";\n",
        );
    }
    for index in 0..relative {
        project.write(
            &format!("src/relative{index}.ts"),
            "import { a } from \"../../lib/a\";\n",
        );
    }
}

#[test]
fn a_nine_of_ten_import_ratio_promotes_and_records_the_stack() {
    let project = Project::with_cache_db();
    write_imports(&project, 9, 1);
    project.write(
        "package.json",
        r#"{"dependencies":{"react":"18.0.0","next":"14.0.0","@nestjs/core":"10.0.0"}}"#,
    );

    let report = scan(&project.store).unwrap();
    assert_eq!(report.files_scanned, 10);
    assert_eq!(report.files_skipped, 0);
    assert_eq!(report.outcomes.len(), 1, "{:?}", report.outcomes);
    let rule = report.outcomes[0].promoted().expect("scan ratio promotes");
    assert_eq!(rule.instruction, PREFER_PATH_ALIASES);
    assert_eq!(rule.confidence, 0.90);

    let context = project.store.rules().unwrap().project_context;
    assert_eq!(context.total_files_scanned, 10);
    assert_eq!(
        context.detected_stack,
        vec![
            "nestjs".to_string(),
            "next".to_string(),
            "react".to_string()
        ]
    );
}

#[test]
fn a_six_of_ten_import_ratio_does_not_promote() {
    let project = Project::with_cache_db();
    write_imports(&project, 6, 4);

    let report = scan(&project.store).unwrap();
    assert!(report.outcomes.is_empty(), "{:?}", report.outcomes);
    assert!(project.store.rules().unwrap().rules.is_empty());
}

#[test]
fn rescanning_an_unchanged_repository_skips_every_file() {
    let project = Project::with_cache_db();
    write_imports(&project, 9, 1);
    let first = scan(&project.store).unwrap();
    assert_eq!(first.outcomes.len(), 1);
    let rules_before = project.store.rules().unwrap().rules.len();

    let second = scan(&project.store).unwrap();
    assert_eq!(second.files_scanned, 0);
    assert_eq!(second.files_skipped, 10);
    assert!(second.outcomes.is_empty(), "{:?}", second.outcomes);
    assert_eq!(project.store.rules().unwrap().rules.len(), rules_before);
}

#[test]
fn gitignored_files_do_not_count_toward_the_support_floor() {
    let project = Project::with_cache_db();
    project.write(".gitignore", "ignored/\n");
    write_imports(&project, 9, 0);
    project.write("ignored/extra.ts", "import { a } from \"@/lib/a\";\n");

    let report = scan(&project.store).unwrap();
    assert_eq!(report.files_scanned, 9);
    assert!(report.outcomes.is_empty(), "{:?}", report.outcomes);
}

#[test]
fn a_rule_the_developer_deleted_is_not_reinstalled_by_the_next_scan() {
    let project = Project::with_cache_db();
    write_imports(&project, 10, 0);
    let first = scan(&project.store).unwrap();
    let id = first.outcomes[0].promoted().unwrap().id.clone();
    assert!(project.store.delete_rule(&id).unwrap());

    // A new file forces a fresh parse, so the ratio is recomputed, not cached.
    project.write("src/alias_new.ts", "import { a } from \"@/lib/a\";\n");
    let second = scan(&project.store).unwrap();
    assert!(second.outcomes.is_empty(), "{:?}", second.outcomes);
    assert!(project.store.rules().unwrap().find(&id).is_none());
    assert!(project.store.candidate_rules().unwrap().is_empty());
}

#[test]
fn vendored_built_and_declaration_files_are_never_scanned() {
    let project = Project::with_cache_db();
    project.write("src/app.ts", "export const a = 1;\n");
    project.write("node_modules/pkg/index.ts", "export default 1;\n");
    project.write("packages/ui/dist/index.js", "export default 1;\n");
    project.write("build/out.js", "export default 1;\n");
    project.write("src/types.d.ts", "export interface A {}\n");
    project.write("public/vendor.min.js", "export default 1;\n");

    let report = scan(&project.store).unwrap();
    assert_eq!(report.files_scanned, 1, "{report:?}");
    assert_eq!(report.tallies.script.exports.files, 1);
}

#[test]
fn the_stored_tally_matches_the_scan_and_forgets_deleted_files() {
    let project = Project::with_cache_db();
    write_imports(&project, 9, 1);
    let report = scan(&project.store).unwrap();
    assert_eq!(stored_tally(&project.store).unwrap(), report.tallies);

    std::fs::remove_file(project.paths.project_root.join("src/relative0.ts")).unwrap();
    let after = stored_tally(&project.store).unwrap();
    assert_eq!(after.script.import_paths.right, 0);
    assert_eq!(after.script.import_paths.left, 9);
}

#[test]
fn a_workspace_package_contributes_its_frameworks_to_the_stack() {
    let project = Project::with_cache_db();
    project.write(
        "package.json",
        r#"{"private":true,"workspaces":["apps/*"]}"#,
    );
    project.write(
        "apps/web/package.json",
        r#"{"dependencies":{"react":"18.0.0"}}"#,
    );
    project.write(
        "apps/api/package.json",
        r#"{"dependencies":{"@nestjs/core":"10.0.0","pg":"8.0.0"}}"#,
    );
    project.write("apps/api/tsconfig.json", "{}");

    scan(&project.store).unwrap();
    let stack = project
        .store
        .rules()
        .unwrap()
        .project_context
        .detected_stack;
    assert_eq!(stack, vec!["nestjs", "postgresql", "react", "typescript"]);
}

#[test]
fn pnpm_workspaces_and_their_exclusions_are_read() {
    let project = Project::with_cache_db();
    project.write("package.json", r#"{"private":true}"#);
    project.write(
        "pnpm-workspace.yaml",
        "packages:\n  - 'packages/*'\n  - \"!packages/fixtures\"\n\ncatalog:\n  react: 18.0.0\n",
    );
    project.write(
        "packages/db/package.json",
        r#"{"dependencies":{"mongoose":"8.0.0"}}"#,
    );
    project.write(
        "packages/fixtures/package.json",
        r#"{"dependencies":{"vue":"3.0.0"}}"#,
    );

    scan(&project.store).unwrap();
    let stack = project
        .store
        .rules()
        .unwrap()
        .project_context
        .detected_stack;
    assert_eq!(stack, vec!["mongodb"]);
}

#[test]
fn typescript_and_databases_are_detected_in_a_single_package() {
    let project = Project::with_cache_db();
    project.write(
        "package.json",
        r#"{"dependencies":{"@prisma/client":"5.0.0","mysql2":"3.0.0"},"devDependencies":{"typescript":"5.0.0"}}"#,
    );

    scan(&project.store).unwrap();
    let stack = project
        .store
        .rules()
        .unwrap()
        .project_context
        .detected_stack;
    assert_eq!(stack, vec!["mysql", "prisma", "typescript"]);
}

#[test]
fn a_package_outside_the_workspace_globs_is_not_read() {
    let project = Project::with_cache_db();
    project.write("package.json", r#"{"workspaces":{"packages":["apps/*"]}}"#);
    project.write(
        "tools/scripts/package.json",
        r#"{"dependencies":{"express":"4.0.0"}}"#,
    );
    project.write(
        "node_modules/react/package.json",
        r#"{"dependencies":{"svelte":"4.0.0"}}"#,
    );

    scan(&project.store).unwrap();
    assert!(project
        .store
        .rules()
        .unwrap()
        .project_context
        .detected_stack
        .is_empty());
}

#[test]
fn a_deleted_file_loses_its_snapshot_on_the_next_scan() {
    let project = Project::with_cache_db();
    write_imports(&project, 2, 0);
    scan(&project.store).unwrap();
    assert!(project
        .store
        .candidates()
        .get_snapshot("src/alias1.ts")
        .unwrap()
        .is_some());

    std::fs::remove_file(project.dir.path().join("src/alias1.ts")).unwrap();
    scan(&project.store).unwrap();
    assert!(project
        .store
        .candidates()
        .get_snapshot("src/alias1.ts")
        .unwrap()
        .is_none());
    assert!(project
        .store
        .candidates()
        .get_snapshot("src/alias0.ts")
        .unwrap()
        .is_some());
}

#[test]
fn a_file_that_grows_past_the_size_limit_stops_counting() {
    let project = Project::with_cache_db();
    write_imports(&project, 1, 0);
    scan(&project.store).unwrap();
    assert_eq!(
        stored_tally(&project.store)
            .unwrap()
            .script
            .import_paths
            .left,
        1
    );

    let big = format!(
        "import {{ a }} from \"@/lib/a\";\n{}",
        "// padding\n".repeat(120_000)
    );
    project.write("src/alias0.ts", &big);
    scan(&project.store).unwrap();
    assert!(project
        .store
        .candidates()
        .get_snapshot("src/alias0.ts")
        .unwrap()
        .is_none());
    assert_eq!(
        stored_tally(&project.store)
            .unwrap()
            .script
            .import_paths
            .left,
        0
    );
}

#[test]
fn a_language_without_analyzers_is_neither_read_nor_snapshotted() {
    let project = Project::with_cache_db();
    write_imports(&project, 1, 0);
    project.write("src/main.rs", "fn main() {}\n");
    project.write("cmd/gen.go", "package main\nfunc main() {}\n");

    let report = scan(&project.store).unwrap();
    assert_eq!(report.files_scanned, 1);
    assert_eq!(report.files_skipped, 0);
    assert!(project
        .store
        .candidates()
        .get_snapshot("src/main.rs")
        .unwrap()
        .is_none());
    assert!(project
        .store
        .candidates()
        .get_snapshot("cmd/gen.go")
        .unwrap()
        .is_none());
}

/// Ten annotated, logging library modules under `src/myapp/`.
fn write_python_library(project: &Project) {
    for index in 0..10 {
        project.write(
            &format!("src/myapp/module{index}.py"),
            "import logging\n\nlogger = logging.getLogger(__name__)\n\n\n\
             def run(job: str) -> None:\n    logger.info(job)\n",
        );
    }
}

#[test]
fn a_python_codebase_promotes_python_rules_scoped_to_python_files() {
    let project = Project::with_cache_db();
    write_python_library(&project);
    let report = scan(&project.store).unwrap();
    let rules: Vec<_> = report
        .outcomes
        .iter()
        .filter_map(|outcome| outcome.promoted())
        .collect();
    let mut instructions: Vec<_> = rules.iter().map(|rule| rule.instruction.as_str()).collect();
    instructions.sort();
    let mut expected = vec![PY_PREFER_ANNOTATIONS, PY_PREFER_LOGGING];
    expected.sort();
    assert_eq!(instructions, expected, "{rules:?}");
    assert!(rules.iter().all(|rule| rule.scope == "**/*.py"));
}

#[test]
fn python_tests_and_scripts_do_not_count() {
    let project = Project::with_cache_db();
    write_python_library(&project);
    // Enough print-heavy tests and scripts to sink the logging ratio if read.
    for index in 0..10 {
        let printing = "def check() -> None:\n    print('a')\n    print('b')\n    print('c')\n";
        project.write(&format!("tests/test_module{index}.py"), printing);
        project.write(&format!("scripts/task{index}.py"), printing);
    }
    let report = scan(&project.store).unwrap();
    assert_eq!(report.files_scanned, 10, "tests and scripts are not read");
    assert_eq!(report.tallies.python.logging.right, 0);
    assert!(report
        .outcomes
        .iter()
        .filter_map(|outcome| outcome.promoted())
        .any(|rule| rule.instruction == PY_PREFER_LOGGING));
}

#[test]
fn a_mixed_repository_keeps_each_languages_rules_apart() {
    let project = Project::with_cache_db();
    write_python_library(&project);
    write_imports(&project, 10, 0);
    let report = scan(&project.store).unwrap();
    let rules: Vec<_> = report
        .outcomes
        .iter()
        .filter_map(|outcome| outcome.promoted())
        .collect();
    for rule in &rules {
        let python = rule.instruction.starts_with("In Python");
        assert_eq!(rule.scope == "**/*.py", python, "{rule:?}");
    }
    assert!(rules
        .iter()
        .any(|rule| rule.instruction == PREFER_PATH_ALIASES));
    assert!(rules
        .iter()
        .any(|rule| rule.instruction == PY_PREFER_LOGGING));
    // Nothing the TypeScript files counted leaked into the Python tally.
    assert_eq!(report.tallies.python.imports.files, 0);
    assert_eq!(report.tallies.script.import_paths.files, 10);
}

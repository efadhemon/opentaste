//! Cold-start scan over a synthetic repository.

mod common;

use common::Project;
use taste_core::ast::stored_tally;
use taste_core::ast::{
    COMPILER_CONFIDENCE, GO_PREFER_WRAPPED_ERRORS, PREFER_PATH_ALIASES, PY_PREFER_ANNOTATIONS,
    PY_PREFER_LOGGING, RS_PREFER_QUESTION_MARK, TS_STRICT_NULL, VALIDATE_WITH_ZOD,
};
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
fn an_unsupported_language_is_neither_read_nor_snapshotted() {
    let project = Project::with_cache_db();
    write_imports(&project, 1, 0);
    project.write("src/Gen.java", "class Gen {}\n");

    let report = scan(&project.store).unwrap();
    assert_eq!(report.files_scanned, 1);
    assert_eq!(report.files_skipped, 0);
    assert!(project
        .store
        .candidates()
        .get_snapshot("src/Gen.java")
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

#[test]
fn a_rust_library_promotes_question_marks_and_ignores_its_tests() {
    let project = Project::with_cache_db();
    project.write("Cargo.toml", "[package]\nname = \"lib\"\n");
    for index in 0..10 {
        project.write(
            &format!("src/module{index}.rs"),
            "pub fn load(path: &Path) -> Result<String, Error> {\n    let text = fs::read_to_string(path)?;\n    Ok(text)\n}\n\
             #[cfg(test)]\nmod tests {\n    #[test]\n    fn loads() {\n        load(p).unwrap();\n        load(q).unwrap();\n    }\n}\n",
        );
        project.write(
            &format!("tests/it{index}.rs"),
            "#[test]\nfn works() {\n    lib::load(p).unwrap();\n}\nfn helper() {\n    x().unwrap();\n    y().unwrap();\n}\n",
        );
    }
    let report = scan(&project.store).unwrap();
    assert_eq!(report.files_scanned, 10, "integration tests are not read");
    assert_eq!(
        report.tallies.rust.errors,
        taste_core::ast::Count {
            left: 10,
            right: 0,
            files: 10
        }
    );
    let rules: Vec<_> = report
        .outcomes
        .iter()
        .filter_map(|outcome| outcome.promoted())
        .collect();
    assert!(
        rules
            .iter()
            .any(|rule| rule.instruction == RS_PREFER_QUESTION_MARK && rule.scope == "**/*.rs"),
        "{rules:?}"
    );
}

#[test]
fn vendored_and_oversized_scripts_do_not_count() {
    let project = Project::with_cache_db();
    write_imports(&project, 1, 0);
    let arrows = "export const a = () => 1;\n".repeat(20);
    project.write("src/static/jdenticon-3.3.0.js", &arrows);
    let huge = "export const a = () => 1;\n".repeat(11_000);
    assert!(huge.len() > 256 * 1024);
    project.write("src/static/datatables.js", &huge);
    let report = scan(&project.store).unwrap();
    assert_eq!(report.files_scanned, 1);
    assert_eq!(report.tallies.script.function_syntax.files, 0);
}

const GO_WRAPPING: &str = "package store\n\nimport (\n\t\"fmt\"\n\t\"os\"\n)\n\n\
func Load(p string) ([]byte, error) {\n\tdata, err := os.ReadFile(p)\n\tif err != nil {\n\t\treturn nil, fmt.Errorf(\"load %s: %w\", p, err)\n\t}\n\treturn data, nil\n}\n";

fn write_go_package(project: &Project) {
    for index in 0..10 {
        project.write(&format!("internal/store/load{index}.go"), GO_WRAPPING);
        project.write(
            &format!("internal/store/load{index}_test.go"),
            "package store\n\nimport \"os\"\n\nfunc helper() error {\n\tif err := os.Remove(\"x\"); err != nil {\n\t\treturn err\n\t}\n\treturn nil\n}\n",
        );
    }
}

#[test]
fn a_go_codebase_promotes_wrapped_errors_and_ignores_its_tests() {
    let project = Project::with_cache_db();
    write_go_package(&project);
    let report = scan(&project.store).unwrap();
    assert_eq!(report.files_scanned, 10, "_test.go files are not read");
    assert_eq!(report.tallies.go.errors.right, 0);
    assert!(report
        .outcomes
        .iter()
        .filter_map(|outcome| outcome.promoted())
        .any(|rule| rule.instruction == GO_PREFER_WRAPPED_ERRORS && rule.scope == "**/*.go"));
}

#[test]
fn a_typescript_python_and_go_repository_keeps_every_languages_rules_apart() {
    let project = Project::with_cache_db();
    write_imports(&project, 10, 0);
    write_python_library(&project);
    write_go_package(&project);
    let report = scan(&project.store).unwrap();
    let rules: Vec<_> = report
        .outcomes
        .iter()
        .filter_map(|outcome| outcome.promoted())
        .collect();
    for rule in &rules {
        let expected = if rule.instruction.starts_with("In Python") {
            "**/*.py"
        } else if rule.instruction.starts_with("In Go") {
            "**/*.go"
        } else {
            "**/*.{ts,tsx,js,jsx,mjs,cjs}"
        };
        assert_eq!(rule.scope, expected, "{rule:?}");
    }
    for instruction in [
        PREFER_PATH_ALIASES,
        PY_PREFER_LOGGING,
        GO_PREFER_WRAPPED_ERRORS,
    ] {
        assert!(
            rules.iter().any(|rule| rule.instruction == instruction),
            "{instruction}"
        );
    }
}

#[test]
fn strict_null_checks_become_a_rule_per_strict_package() {
    let project = Project::with_cache_db();
    project.write(
        "tsconfig.base.json",
        "{\n  // shared\n  \"compilerOptions\": { \"strict\": true, },\n}\n",
    );
    project.write(
        "apps/web/tsconfig.json",
        "{ \"extends\": \"../../tsconfig.base.json\" }",
    );
    project.write(
        "apps/legacy/tsconfig.json",
        "{ \"extends\": \"../../tsconfig.base.json\", \"compilerOptions\": { \"strictNullChecks\": false } }",
    );
    project.write("apps/web/src/a.ts", "export const a = 1;\n");
    let report = scan(&project.store).unwrap();
    let rules: Vec<_> = report
        .outcomes
        .iter()
        .filter_map(|outcome| outcome.promoted())
        .filter(|rule| rule.instruction == TS_STRICT_NULL)
        .collect();
    assert_eq!(rules.len(), 1, "{rules:?}");
    assert_eq!(rules[0].scope, "apps/web/**/*.{ts,tsx}");
    assert_eq!(rules[0].confidence, COMPILER_CONFIDENCE);
    assert_eq!(
        report.files_scanned, 1,
        "tsconfig files are read, not counted"
    );
}

#[test]
fn turning_strict_null_checks_off_retracts_the_rule_and_turning_them_on_restores_it() {
    let project = Project::with_cache_db();
    let strict = "{ \"compilerOptions\": { \"strict\": true } }";
    project.write("tsconfig.json", strict);
    let strict_rules = |project: &Project| {
        project
            .store
            .rules()
            .unwrap()
            .rules
            .into_iter()
            .filter(|rule| rule.instruction == TS_STRICT_NULL)
            .map(|rule| rule.scope)
            .collect::<Vec<_>>()
    };
    scan(&project.store).unwrap();
    assert_eq!(strict_rules(&project), vec!["**/*.{ts,tsx}"]);

    project.write(
        "tsconfig.json",
        "{ \"compilerOptions\": { \"strict\": false } }",
    );
    let report = scan(&project.store).unwrap();
    assert_eq!(report.retracted.len(), 1);
    assert!(strict_rules(&project).is_empty());

    project.write("tsconfig.json", strict);
    scan(&project.store).unwrap();
    assert_eq!(
        strict_rules(&project),
        vec!["**/*.{ts,tsx}"],
        "retracting is not rejecting"
    );
}

#[test]
fn a_project_without_strict_null_checks_gets_no_rule() {
    let project = Project::with_cache_db();
    project.write(
        "tsconfig.json",
        "{ \"compilerOptions\": { \"target\": \"es2022\" } }",
    );
    let report = scan(&project.store).unwrap();
    assert!(report
        .outcomes
        .iter()
        .filter_map(|outcome| outcome.promoted())
        .all(|rule| rule.instruction != TS_STRICT_NULL));
}

fn write_validators(project: &Project, zod: usize, yup: usize) {
    for index in 0..zod {
        project.write(
            &format!("src/schemas/zod{index}.ts"),
            "import { z } from 'zod';\nexport const S = z.object({ a: z.string() });\n",
        );
    }
    for index in 0..yup {
        project.write(
            &format!("src/schemas/yup{index}.ts"),
            "import * as yup from 'yup';\nexport const S = yup.object({ a: yup.string() });\n",
        );
    }
}

fn promoted_instructions(report: &taste_core::ScanReport) -> Vec<String> {
    report
        .outcomes
        .iter()
        .filter_map(|outcome| outcome.promoted())
        .map(|rule| rule.instruction.clone())
        .collect()
}

#[test]
fn a_codebase_validating_with_zod_gets_a_zod_rule() {
    let project = Project::with_cache_db();
    write_validators(&project, 10, 0);
    // Files that validate nothing do not dilute the ratio.
    for index in 0..20 {
        project.write(&format!("src/plain{index}.ts"), "export const x = 1;\n");
    }
    let report = scan(&project.store).unwrap();
    assert!(promoted_instructions(&report).contains(&VALIDATE_WITH_ZOD.to_string()));
}

#[test]
fn mixed_or_too_few_validating_files_get_no_validation_rule() {
    for (zod, yup) in [(7, 3), (9, 0)] {
        let project = Project::with_cache_db();
        write_validators(&project, zod, yup);
        let report = scan(&project.store).unwrap();
        let instructions = promoted_instructions(&report);
        assert!(
            !instructions.iter().any(|i| i.starts_with("Validate input")),
            "{zod} zod, {yup} yup: {instructions:?}"
        );
    }
}

//! Cold-start scan over a synthetic repository.

mod common;

use common::Project;
use taste_core::ast::PREFER_PATH_ALIASES;
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

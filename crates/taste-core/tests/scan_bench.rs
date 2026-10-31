//! The scan benchmark behind SPEC.md §1's speed claim.
//!
//! Ignored by default; run it in release mode:
//!
//! ```text
//! cargo test --release -p taste-core --test scan_bench -- --ignored --nocapture
//! ```
//!
//! It writes a 1,000-file tree in four languages and times a cold scan (an
//! empty cache) and a warm one (nothing changed). Prefix it with
//! `taskset -c 0-3` to measure on four cores.

mod common;

use std::time::Instant;

use common::Project;
use taste_core::scan;

/// One file of each kind, around sixty lines, the shapes the analyzers count.
fn typescript(index: usize) -> String {
    let mut out = format!(
        "import {{ Injectable }} from \"@nestjs/common\";\nimport {{ Repo }} from \"@/lib/repo{index}\";\nimport {{ z }} from \"zod\";\n\nexport const Schema{index} = z.object({{ id: z.string() }});\nexport type Input{index} = z.infer<typeof Schema{index}>;\n\n"
    );
    for f in 0..6 {
        out.push_str(&format!(
            "export function handle{f}(input: Input{index}): number {{\n  if (!input.id) {{\n    return 0;\n  }}\n  const value = input.id.length * {f};\n  if (value > 10) {{\n    return value;\n  }} else {{\n    return value + 1;\n  }}\n}}\n\n"
        ));
    }
    out.push_str(&format!(
        "@Injectable()\nexport class Service{index} {{\n  constructor(private readonly repo: Repo) {{}}\n  find(id: string) {{\n    if (!id) throw new NotFoundError(id);\n    return this.repo.find(id);\n  }}\n}}\n"
    ));
    out
}

fn python(index: usize) -> String {
    let mut out = format!(
        "import logging\nfrom app.models import Model{index}\nfrom .errors import NotFound\n\nlogger = logging.getLogger(__name__)\n\n\n"
    );
    for f in 0..8 {
        out.push_str(&format!(
            "def handle{f}(item: Model{index}) -> int:\n    if item is None:\n        raise NotFound(\"missing\")\n    logger.info(\"handling %s\", item)\n    return item.value * {f}\n\n\n"
        ));
    }
    out
}

fn rust(index: usize) -> String {
    let mut out = format!("use crate::store::Store{index};\nuse tracing::info;\n\n");
    for f in 0..8 {
        out.push_str(&format!(
            "pub fn handle{f}(store: &Store{index}) -> Result<u32, Error> {{\n    let Some(item) = store.get({f})? else {{\n        return Ok(0);\n    }};\n    info!(\"handled\");\n    Ok(item.value)\n}}\n\n"
        ));
    }
    out
}

fn go(index: usize) -> String {
    let mut out =
        format!("package store{index}\n\nimport (\n\t\"fmt\"\n\t\"log/slog\"\n\t\"os\"\n)\n\n");
    for f in 0..8 {
        out.push_str(&format!(
            "func Handle{f}(path string) ([]byte, error) {{\n\tdata, err := os.ReadFile(path)\n\tif err != nil {{\n\t\treturn nil, fmt.Errorf(\"read %s: %w\", path, err)\n\t}}\n\tslog.Info(\"read\", \"path\", path)\n\treturn data, nil\n}}\n\n"
        ));
    }
    out
}

fn write_tree(project: &Project) {
    for index in 0..700 {
        project.write(
            &format!("web/src/m{}/f{index}.ts", index % 20),
            &typescript(index),
        );
    }
    for index in 0..100 {
        project.write(
            &format!("api/app/m{}/f{index}.py", index % 5),
            &python(index),
        );
        project.write(
            &format!("core/src/m{}/f{index}.rs", index % 5),
            &rust(index),
        );
        project.write(&format!("svc/m{}/f{index}.go", index % 5), &go(index));
    }
}

#[test]
#[ignore = "a benchmark: run with --release --ignored --nocapture"]
fn scan_a_thousand_files_cold_and_warm() {
    let project = Project::with_cache_db();
    write_tree(&project);

    let started = Instant::now();
    let cold = scan(&project.store).unwrap();
    let cold_time = started.elapsed();
    assert_eq!(cold.files_scanned, 1000);

    let started = Instant::now();
    let warm = scan(&project.store).unwrap();
    let warm_time = started.elapsed();
    assert_eq!(warm.files_scanned, 0);

    println!(
        "scan of 1,000 files: cold {:.0} ms, warm {:.0} ms",
        cold_time.as_secs_f64() * 1000.0,
        warm_time.as_secs_f64() * 1000.0
    );
}

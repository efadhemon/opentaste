//! What a file is for, read from its path alone.
//!
//! `.unwrap()` in a test, `print()` in a one-off script and `fmt.Println` in a
//! `_test.go` file are not anyone's taste, so the scanner leaves such files
//! out of the counts. TypeScript and JavaScript keep their existing behaviour:
//! their dimensions (imports, exports, types) mean the same thing in a spec
//! file as anywhere else.

use std::path::{Component, Path};

use super::{Family, SupportedLanguage};

/// Directories whose Python files are tests or scripts.
const PYTHON_DIRS: &[&str] = &["tests", "test", "scripts"];
/// Directories whose Rust files are integration tests, benchmarks or examples.
const RUST_DIRS: &[&str] = &["tests", "benches", "examples"];
/// Directories Go keeps test fixtures in.
const GO_DIRS: &[&str] = &["testdata"];

/// Whether the scanner should leave `path`, relative to the project root, out
/// of the counts because it is test code, a script or a build script.
pub fn is_test_or_script(path: &Path, lang: SupportedLanguage) -> bool {
    let Some(file) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let in_dir = |names: &[&str]| {
        path.parent().is_some_and(|parent| {
            parent.components().any(|part| match part {
                Component::Normal(name) => name.to_str().is_some_and(|name| names.contains(&name)),
                _ => false,
            })
        })
    };
    match lang.family() {
        Family::Script => false,
        Family::Python => {
            is_test_file_name(file, lang) || file == "conftest.py" || in_dir(PYTHON_DIRS)
        }
        Family::Rust => file == "build.rs" || in_dir(RUST_DIRS),
        Family::Go => is_test_file_name(file, lang) || in_dir(GO_DIRS),
    }
}

/// The file-name glob of a test file's role, such as `*_test.go` or
/// `test_*.py`, so a rule learned in a test stays with the tests.
pub(crate) fn test_file_pattern(file: &str, lang: SupportedLanguage) -> Option<String> {
    let (stem, ext) = file.rsplit_once('.')?;
    match lang.family() {
        Family::Python if stem.starts_with("test_") => Some(format!("test_*.{ext}")),
        Family::Python | Family::Go if stem.ends_with("_test") => Some(format!("*_test.{ext}")),
        _ => None,
    }
}

fn is_test_file_name(file: &str, lang: SupportedLanguage) -> bool {
    test_file_pattern(file, lang).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn excluded(path: &str) -> bool {
        let path = Path::new(path);
        is_test_or_script(path, SupportedLanguage::from_path(path).unwrap())
    }

    #[test]
    fn python_tests_and_scripts_are_left_out_of_the_counts() {
        for path in [
            "tests/test_users.py",
            "app/users_test.py",
            "app/test_users.py",
            "conftest.py",
            "scripts/seed.py",
            "services/api/tests/factories.py",
        ] {
            assert!(excluded(path), "{path}");
        }
        for path in ["app/users.py", "app/testing_utils.py", "app/contest.py"] {
            assert!(!excluded(path), "{path}");
        }
    }

    #[test]
    fn rust_integration_tests_benches_examples_and_build_scripts_are_left_out() {
        for path in [
            "tests/cli.rs",
            "crates/core/tests/scan.rs",
            "benches/parse.rs",
            "examples/demo.rs",
            "build.rs",
        ] {
            assert!(excluded(path), "{path}");
        }
        for path in ["src/lib.rs", "crates/core/src/tests_support.rs"] {
            assert!(!excluded(path), "{path}");
        }
    }

    #[test]
    fn go_test_files_and_fixtures_are_left_out() {
        assert!(excluded("pkg/users/users_test.go"));
        assert!(excluded("pkg/users/testdata/broken.go"));
        assert!(!excluded("pkg/users/users.go"));
        assert!(!excluded("pkg/users/latest.go"));
    }

    #[test]
    fn typescript_spec_files_still_count() {
        assert!(!excluded("src/users/users.spec.ts"));
        assert!(!excluded("tests/e2e/login.test.ts"));
    }

    #[test]
    fn test_files_get_a_role_pattern_per_language() {
        let pattern = |file: &str| {
            test_file_pattern(file, SupportedLanguage::from_path(Path::new(file)).unwrap())
        };
        assert_eq!(pattern("users_test.go").as_deref(), Some("*_test.go"));
        assert_eq!(pattern("test_users.py").as_deref(), Some("test_*.py"));
        assert_eq!(pattern("users_test.py").as_deref(), Some("*_test.py"));
        assert_eq!(pattern("users.go"), None);
        assert_eq!(pattern("users_test.rs"), None);
        assert_eq!(pattern("users_test.ts"), None);
    }
}

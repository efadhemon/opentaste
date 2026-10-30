//! Multi-language tree-sitter wrapper.
//!
//! `Parser` is neither `Sync` nor cheap to build, so each worker thread keeps
//! one parser per language in a `thread_local` and reuses it. A parse that
//! returns `None` (the parser gave up mid-callback) resumes on the next call
//! unless [`Parser::reset`] is called, which would attribute the leftover
//! state to the next file that thread handles. A tree that merely contains a
//! syntax error is still a finished parse and does not need a reset.

use std::cell::RefCell;
use std::path::Path;

use sha2::{Digest, Sha256};
use tree_sitter::{Language, Parser, Tree};

/// Languages that share one set of conventions, and therefore one set of
/// counts and one rule scope. TypeScript, TSX and JavaScript are one family:
/// the same import, export and function-syntax choices apply to all three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Family {
    Script,
    Python,
    Rust,
    Go,
}

impl Family {
    pub const ALL: [Family; 4] = [Family::Script, Family::Python, Family::Rust, Family::Go];

    /// Label shown beside this family's scan counts.
    pub fn name(self) -> &'static str {
        match self {
            Self::Script => "ts/js",
            Self::Python => "python",
            Self::Rust => "rust",
            Self::Go => "go",
        }
    }

    /// Glob every file of this family matches. Scan rules use it as their
    /// scope, which keeps one language's rule out of another's files and,
    /// because the scope is part of a rule's identifier, gives the same
    /// dimension in two languages two identifiers.
    pub const fn scope(self) -> &'static str {
        match self {
            Self::Script => "**/*.{ts,tsx,js,jsx,mjs,cjs}",
            Self::Python => "**/*.py",
            Self::Rust => "**/*.rs",
            Self::Go => "**/*.go",
        }
    }
}

/// Languages the scanner and the diff engine know how to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SupportedLanguage {
    TypeScript,
    Tsx,
    /// Plain JavaScript, including JSX. Distinct from [`Tsx`] so the type
    /// analyzer can ignore files that cannot contain `interface` or `type`.
    JavaScript,
    Python,
    Rust,
    Go,
}

impl SupportedLanguage {
    /// Detect a language from a file extension, case-insensitively.
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?;
        if ext.eq_ignore_ascii_case("ts")
            || ext.eq_ignore_ascii_case("mts")
            || ext.eq_ignore_ascii_case("cts")
        {
            Some(Self::TypeScript)
        } else if ext.eq_ignore_ascii_case("tsx") {
            Some(Self::Tsx)
        } else if ext.eq_ignore_ascii_case("js")
            || ext.eq_ignore_ascii_case("jsx")
            || ext.eq_ignore_ascii_case("mjs")
            || ext.eq_ignore_ascii_case("cjs")
        {
            Some(Self::JavaScript)
        } else if ext.eq_ignore_ascii_case("py") {
            Some(Self::Python)
        } else if ext.eq_ignore_ascii_case("rs") {
            Some(Self::Rust)
        } else if ext.eq_ignore_ascii_case("go") {
            Some(Self::Go)
        } else {
            None
        }
    }

    /// Short name used in diagnostics and snapshot signatures.
    pub fn name(self) -> &'static str {
        match self {
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::JavaScript => "javascript",
            Self::Python => "python",
            Self::Rust => "rust",
            Self::Go => "go",
        }
    }

    /// The family whose conventions and counts this language shares.
    pub fn family(self) -> Family {
        match self {
            Self::TypeScript | Self::Tsx | Self::JavaScript => Family::Script,
            Self::Python => Family::Python,
            Self::Rust => Family::Rust,
            Self::Go => Family::Go,
        }
    }

    /// TypeScript or TSX. JavaScript is deliberately excluded.
    pub fn is_typescript(self) -> bool {
        matches!(self, Self::TypeScript | Self::Tsx)
    }

    /// A language whose analyzers share the TypeScript/JavaScript queries.
    pub fn is_script(self) -> bool {
        matches!(self, Self::TypeScript | Self::Tsx | Self::JavaScript)
    }

    /// Whether OpenTaste learns anything from this language yet. The scanner
    /// and the post-commit hook skip other files before reading them. This is
    /// the one switch to flip when a language gains analyzers and detectors.
    pub fn has_analyzers(self) -> bool {
        matches!(
            self.family(),
            Family::Script | Family::Python | Family::Rust
        )
    }

    /// Grammar for this language.
    ///
    /// JavaScript borrows the TSX grammar: it parses plain JavaScript and JSX,
    /// and every analyzer looks at node shapes the two languages share. The
    /// [`SupportedLanguage::JavaScript`] variant stays separate so callers can
    /// still tell a `.js` file from a `.tsx` file.
    pub fn language(self) -> Language {
        match self {
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx | Self::JavaScript => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
        }
    }

    /// Grammar identity for query caches. JavaScript shares TSX's compiled queries.
    pub(crate) fn grammar(self) -> Self {
        match self {
            Self::JavaScript => Self::Tsx,
            other => other,
        }
    }
}

thread_local! {
    static PARSERS: RefCell<Vec<(SupportedLanguage, Parser)>> = const { RefCell::new(Vec::new()) };
}

/// Parse `source` with the grammar for `lang`.
///
/// Returns `None` when the grammar cannot be loaded. A source file with syntax
/// errors still returns a tree; callers that need a clean sample check
/// [`Tree::root_node`]`.has_error()`.
pub fn parse(lang: SupportedLanguage, source: &str) -> Option<Tree> {
    PARSERS.with(|cell| {
        let mut parsers = cell.borrow_mut();
        if let Some((_, parser)) = parsers.iter_mut().find(|(stored, _)| *stored == lang) {
            return finish(parser, source);
        }
        let mut parser = Parser::new();
        parser.set_language(&lang.language()).ok()?;
        let tree = finish(&mut parser, source);
        parsers.push((lang, parser));
        tree
    })
}

fn finish(parser: &mut Parser, source: &str) -> Option<Tree> {
    let tree = parser.parse(source, None);
    if tree.is_none() {
        parser.reset();
    }
    tree
}

/// SHA-256 of `bytes`, as a lowercase hex string.
///
/// This is the fingerprint stored in `file_snapshots.content_hash`.
pub fn content_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_select_the_language_a_file_is_written_in() {
        assert_eq!(
            SupportedLanguage::from_path(Path::new("src/app.ts")),
            Some(SupportedLanguage::TypeScript)
        );
        assert_eq!(
            SupportedLanguage::from_path(Path::new("src/App.TSX")),
            Some(SupportedLanguage::Tsx)
        );
        assert_eq!(
            SupportedLanguage::from_path(Path::new("src/old.jsx")),
            Some(SupportedLanguage::JavaScript)
        );
        assert_eq!(
            SupportedLanguage::from_path(Path::new("src/lib.rs")),
            Some(SupportedLanguage::Rust)
        );
        assert_eq!(SupportedLanguage::from_path(Path::new("README.md")), None);
    }

    #[test]
    fn every_supported_language_parses_a_minimal_sample() {
        let samples = [
            (
                SupportedLanguage::TypeScript,
                "export function n(x: number): number { return x; }\n",
            ),
            (
                SupportedLanguage::Tsx,
                "export const View = () => <div>hi</div>;\n",
            ),
            (
                SupportedLanguage::JavaScript,
                "export const View = () => <div>hi</div>;\n",
            ),
            (SupportedLanguage::Python, "def n(x):\n    return x\n"),
            (SupportedLanguage::Rust, "fn n(x: i32) -> i32 { x }\n"),
            (
                SupportedLanguage::Go,
                "package main\nfunc n(x int) int { return x }\n",
            ),
        ];
        for (lang, source) in samples {
            let tree =
                parse(lang, source).unwrap_or_else(|| panic!("{} produced no tree", lang.name()));
            assert!(
                !tree.root_node().has_error(),
                "{} failed to parse: {}",
                lang.name(),
                tree.root_node().to_sexp()
            );
        }
    }

    #[test]
    fn every_family_scope_matches_its_own_files_and_no_other_familys() {
        let files = [
            "src/a.ts",
            "src/b.tsx",
            "src/c.mjs",
            "app/d.py",
            "src/e.rs",
            "pkg/f.go",
        ];
        for family in Family::ALL {
            let glob = globset::GlobBuilder::new(family.scope())
                .literal_separator(true)
                .build()
                .unwrap()
                .compile_matcher();
            for file in files {
                let lang = SupportedLanguage::from_path(Path::new(file)).unwrap();
                assert_eq!(
                    glob.is_match(file),
                    lang.family() == family,
                    "{} against {file}",
                    family.scope()
                );
            }
        }
    }

    #[test]
    fn a_syntax_error_does_not_poison_the_next_file_on_this_thread() {
        let broken = parse(SupportedLanguage::TypeScript, "function (").unwrap();
        assert!(broken.root_node().has_error());
        let healthy = parse(
            SupportedLanguage::TypeScript,
            "function ok(): number { return 1; }\n",
        )
        .unwrap();
        assert!(
            !healthy.root_node().has_error(),
            "{}",
            healthy.root_node().to_sexp()
        );
    }

    #[test]
    fn content_hash_is_the_sha256_of_the_bytes() {
        assert_eq!(
            content_hash(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_ne!(content_hash(b"a"), content_hash(b"b"));
    }
}

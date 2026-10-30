//! Strict null checking, read from `tsconfig.json` rather than counted.
//!
//! The compiler enforces the setting, so it is the most certain signal
//! OpenTaste has. It is read per `tsconfig.json`, following `extends` inside
//! the repository: relative paths, and the repository's own workspace
//! packages by name (`@acme/tsconfig/base.json`, the usual monorepo setup).
//! A base config from an installed package (`@tsconfig/strictest`) is not
//! read: what it sets is unknown, and an unknown setting is never taken to
//! be on. Each option merges child over parent on its own, then
//! `strictNullChecks` decides if set and `strict` decides otherwise, as `tsc`
//! does.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::storage::validate_scope;

/// The file name a package's TypeScript settings live in.
pub const TSCONFIG: &str = "tsconfig.json";

/// How deep an `extends` chain is followed before giving up.
const MAX_EXTENDS: usize = 16;

/// Whether `tsconfig` (relative to `root`) turns on strict null checks.
/// `None` when the file cannot be read or a setting comes from outside the
/// repository.
pub fn strict_null_checks(root: &Path, tsconfig: &Path) -> Option<bool> {
    strict_null_checks_in(root, &workspace_names(root), tsconfig)
}

/// The repository's workspace packages, by the name in their `package.json`.
type Workspace = HashMap<String, PathBuf>;

fn workspace_names(root: &Path) -> Workspace {
    let mut names = Workspace::new();
    for dir in super::scan::workspace_packages(root) {
        let name = fs::read_to_string(dir.join("package.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|manifest| manifest.get("name")?.as_str().map(str::to_string));
        if let Some(name) = name {
            names.insert(name, dir);
        }
    }
    names
}

fn strict_null_checks_in(root: &Path, workspace: &Workspace, tsconfig: &Path) -> Option<bool> {
    let mut merged = Settings::default();
    read_chain(
        root,
        workspace,
        &root.join(tsconfig),
        &mut HashSet::new(),
        0,
        &mut merged,
    )?;
    match (merged.strict_null_checks, merged.strict) {
        (Setting::Set(on), _) => Some(on),
        (Setting::Unknown, _) => None,
        (Setting::Unset, Setting::Set(on)) => Some(on),
        (Setting::Unset, Setting::Unknown) => None,
        (Setting::Unset, Setting::Unset) => Some(false),
    }
}

/// One compiler option through an `extends` chain.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Setting {
    /// Nothing in the chain sets it.
    #[default]
    Unset,
    /// A base outside the repository might set it.
    Unknown,
    Set(bool),
}

impl Setting {
    fn of(options: Option<&Value>, key: &str) -> Self {
        match options.and_then(|o| o.get(key)).and_then(Value::as_bool) {
            Some(on) => Setting::Set(on),
            None => Setting::Unset,
        }
    }

    /// `self` laid over `base`: a child's own value wins.
    fn over(self, base: Setting) -> Setting {
        if self == Setting::Unset {
            base
        } else {
            self
        }
    }
}

#[derive(Debug, Default)]
struct Settings {
    strict: Setting,
    strict_null_checks: Setting,
}

/// Fold `file` and everything it extends into `merged`, parents first so the
/// child wins. A base from a package makes both options unknown until a
/// later file sets them. Returns `None` when a link inside the repository
/// cannot be read.
fn read_chain(
    root: &Path,
    workspace: &Workspace,
    file: &Path,
    seen: &mut HashSet<PathBuf>,
    depth: usize,
    merged: &mut Settings,
) -> Option<()> {
    if depth > MAX_EXTENDS || !seen.insert(file.to_path_buf()) || !inside(root, file) {
        return None;
    }
    let config: Value = serde_json::from_str(&strip_jsonc(&fs::read_to_string(file).ok()?)).ok()?;
    let parents: Vec<&str> = match config.get("extends") {
        Some(Value::String(one)) => vec![one.as_str()],
        Some(Value::Array(many)) => many.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    let dir = file.parent()?;
    // A later entry overrides an earlier one, and the file itself overrides
    // them all.
    for parent in parents {
        match resolve(workspace, dir, parent) {
            Some(path) => read_chain(root, workspace, &path, seen, depth + 1, merged)?,
            None => {
                merged.strict = Setting::Unknown;
                merged.strict_null_checks = Setting::Unknown;
            }
        }
    }
    let options = config.get("compilerOptions");
    merged.strict = Setting::of(options, "strict").over(merged.strict);
    merged.strict_null_checks =
        Setting::of(options, "strictNullChecks").over(merged.strict_null_checks);
    Some(())
}

/// A relative `extends`, or one naming a workspace package of this
/// repository (`@acme/tsconfig/base.json`, or the package alone for its
/// `tsconfig.json`), with `.json` added when it is missing. Any other bare
/// name is an installed package and is not followed.
fn resolve(workspace: &Workspace, dir: &Path, target: &str) -> Option<PathBuf> {
    let mut path = if target.starts_with("./") || target.starts_with("../") {
        dir.join(target)
    } else {
        let segments = if target.starts_with('@') { 2 } else { 1 };
        let name_len = target
            .match_indices('/')
            .nth(segments - 1)
            .map_or(target.len(), |(index, _)| index);
        let package = workspace.get(&target[..name_len])?;
        match target[name_len..].trim_start_matches('/') {
            "" => package.join(TSCONFIG),
            subpath => package.join(subpath),
        }
    };
    if path.extension().is_none_or(|ext| ext != "json") {
        path.as_mut_os_string().push(".json");
    }
    Some(path)
}

/// Whether `file` stays under `root` once `..` is applied, without touching
/// the filesystem.
fn inside(root: &Path, file: &Path) -> bool {
    let Ok(relative) = file.strip_prefix(root) else {
        return false;
    };
    let mut depth = 0i32;
    for part in relative.components() {
        match part {
            Component::ParentDir => depth -= 1,
            Component::Normal(_) => depth += 1,
            _ => {}
        }
        if depth < 0 {
            return false;
        }
    }
    true
}

/// `tsconfig.json` allows comments and trailing commas; JSON does not.
/// Strings are copied untouched, so `"//"` inside a path survives.
pub fn strip_jsonc(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                let start = index;
                index += 1;
                while index < bytes.len() && bytes[index] != b'"' {
                    index += if bytes[index] == b'\\' { 2 } else { 1 };
                }
                index = (index + 1).min(bytes.len());
                out.push_str(&text[start..index]);
            }
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
            }
            b',' => {
                let rest = text[index + 1..].trim_start();
                let next = strip_leading_comments(rest);
                if !(next.starts_with('}') || next.starts_with(']')) {
                    out.push(',');
                }
                index += 1;
            }
            _ => {
                let next = text[index..].chars().next().unwrap_or(' ');
                out.push(next);
                index += next.len_utf8();
            }
        }
    }
    out
}

fn strip_leading_comments(mut rest: &str) -> &str {
    loop {
        if let Some(after) = rest.strip_prefix("//") {
            rest = after
                .split_once('\n')
                .map_or("", |(_, tail)| tail)
                .trim_start();
        } else if let Some(after) = rest.strip_prefix("/*") {
            rest = after
                .split_once("*/")
                .map_or("", |(_, tail)| tail)
                .trim_start();
        } else {
            return rest;
        }
    }
}

/// The scope a `tsconfig.json` governs: the TypeScript files under its
/// directory. `None` for a directory that cannot be written as a glob.
pub fn scope_of(tsconfig: &Path) -> Option<String> {
    let dir = tsconfig
        .parent()
        .map(|dir| dir.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let scope = if dir.is_empty() {
        "**/*.{ts,tsx}".to_string()
    } else {
        if dir.contains(['*', '?', '[', ']', '{', '}']) {
            return None;
        }
        format!("{dir}/**/*.{{ts,tsx}}")
    };
    validate_scope(&scope).is_ok().then_some(scope)
}

/// The `tsconfig.json` files whose rule may be emitted: each one with strict
/// null checks on and no nested `tsconfig.json` that disagrees. A scope that
/// covers files compiled without the check would be a wrong rule for them.
pub fn strict_configs(root: &Path, tsconfigs: &[PathBuf]) -> Vec<PathBuf> {
    let workspace = workspace_names(root);
    let verdicts: Vec<(PathBuf, Option<bool>)> = tsconfigs
        .iter()
        .map(|config| {
            (
                config.clone(),
                strict_null_checks_in(root, &workspace, config),
            )
        })
        .collect();
    let dir = |config: &Path| config.parent().map(Path::to_path_buf).unwrap_or_default();
    verdicts
        .iter()
        .filter(|(_, verdict)| *verdict == Some(true))
        .filter(|(config, _)| {
            let outer = dir(config);
            !verdicts.iter().any(|(other, verdict)| {
                other != config && *verdict != Some(true) && dir(other).starts_with(&outer)
            })
        })
        .map(|(config, _)| config.clone())
        .collect()
}

/// The scopes the strict-null rule is recorded for. When every
/// `tsconfig.json` in the repository compiles with the check, that is one
/// project-wide rule rather than one per package repeating the same line in
/// every editor file; otherwise one per strict package.
pub fn strict_scopes(root: &Path, tsconfigs: &[PathBuf]) -> Vec<String> {
    let strict = strict_configs(root, tsconfigs);
    if !strict.is_empty() && strict.len() == tsconfigs.len() {
        return scope_of(Path::new(TSCONFIG)).into_iter().collect();
    }
    let mut scopes: Vec<String> = strict
        .iter()
        .filter_map(|config| scope_of(config))
        .collect();
    scopes.sort();
    scopes
}

/// The nearest `tsconfig.json` at or above `file`'s directory, relative to
/// `root`, if there is one inside the repository.
pub fn nearest_tsconfig(root: &Path, file: &Path) -> Option<PathBuf> {
    let mut dir = file.parent();
    while let Some(current) = dir {
        let candidate = current.join(TSCONFIG);
        if root.join(&candidate).is_file() {
            return Some(candidate);
        }
        dir = current.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, contents) in files {
            let path = dir.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        dir
    }

    fn verdict(files: &[(&str, &str)], config: &str) -> Option<bool> {
        let dir = project(files);
        strict_null_checks(dir.path(), Path::new(config))
    }

    #[test]
    fn strict_turns_null_checks_on_and_an_explicit_setting_wins() {
        assert_eq!(
            verdict(
                &[("tsconfig.json", r#"{"compilerOptions":{"strict":true}}"#)],
                TSCONFIG
            ),
            Some(true)
        );
        assert_eq!(
            verdict(
                &[(
                    "tsconfig.json",
                    r#"{"compilerOptions":{"strict":true,"strictNullChecks":false}}"#
                )],
                TSCONFIG
            ),
            Some(false)
        );
        assert_eq!(
            verdict(&[("tsconfig.json", r#"{"compilerOptions":{}}"#)], TSCONFIG),
            Some(false),
            "a missing setting is off"
        );
    }

    #[test]
    fn an_extends_chain_merges_each_option_child_over_parent() {
        let files = [
            (
                "tsconfig.base.json",
                r#"{"compilerOptions":{"strict":true}}"#,
            ),
            (
                "packages/a/tsconfig.json",
                r#"{"extends":"../../tsconfig.base"}"#,
            ),
            (
                "packages/b/tsconfig.json",
                r#"{"extends":"../../tsconfig.base.json","compilerOptions":{"strictNullChecks":false}}"#,
            ),
            (
                "packages/c/tsconfig.json",
                r#"{"extends":["../../tsconfig.base.json","./loose.json"]}"#,
            ),
            (
                "packages/c/loose.json",
                r#"{"compilerOptions":{"strict":false}}"#,
            ),
        ];
        assert_eq!(verdict(&files, "packages/a/tsconfig.json"), Some(true));
        assert_eq!(verdict(&files, "packages/b/tsconfig.json"), Some(false));
        assert_eq!(
            verdict(&files, "packages/c/tsconfig.json"),
            Some(false),
            "the later entry of an extends array wins"
        );
    }

    #[test]
    fn a_base_from_a_package_or_outside_the_repository_is_unknown() {
        assert_eq!(
            verdict(
                &[("tsconfig.json", r#"{"extends":"@tsconfig/strictest"}"#)],
                TSCONFIG
            ),
            None
        );
        assert_eq!(
            verdict(
                &[(
                    "tsconfig.json",
                    r#"{"extends":"@tsconfig/node20","compilerOptions":{"strict":true}}"#
                )],
                TSCONFIG
            ),
            None,
            "the package's base may still turn strictNullChecks off under strict"
        );
        assert_eq!(
            verdict(
                &[(
                    "tsconfig.json",
                    r#"{"extends":"@tsconfig/node20","compilerOptions":{"strictNullChecks":true}}"#
                )],
                TSCONFIG
            ),
            Some(true),
            "setting strictNullChecks itself decides"
        );
        assert_eq!(
            verdict(
                &[("tsconfig.json", r#"{"extends":"../outside.json"}"#)],
                TSCONFIG
            ),
            None
        );
        assert_eq!(
            verdict(
                &[("tsconfig.json", r#"{"extends":"./tsconfig.json"}"#)],
                TSCONFIG
            ),
            None,
            "a cycle is not followed forever"
        );
    }

    #[test]
    fn a_base_from_a_workspace_package_of_this_repository_is_followed() {
        let files = [
            (
                "pnpm-workspace.yaml",
                "packages:\n  - 'apps/*'\n  - 'tooling/*'\n",
            ),
            (
                "tooling/typescript/package.json",
                r#"{"name":"@acme/tsconfig"}"#,
            ),
            (
                "tooling/typescript/base.json",
                r#"{"compilerOptions":{"strict":true}}"#,
            ),
            (
                "tooling/typescript/tsconfig.json",
                r#"{"compilerOptions":{"strict":false}}"#,
            ),
            (
                "apps/web/tsconfig.json",
                r#"{"extends":"@acme/tsconfig/base.json"}"#,
            ),
            (
                "apps/docs/tsconfig.json",
                r#"{"extends":"@acme/tsconfig/base"}"#,
            ),
            ("apps/old/tsconfig.json", r#"{"extends":"@acme/tsconfig"}"#),
            (
                "apps/ext/tsconfig.json",
                r#"{"extends":"@other/tsconfig/base.json"}"#,
            ),
        ];
        assert_eq!(verdict(&files, "apps/web/tsconfig.json"), Some(true));
        assert_eq!(verdict(&files, "apps/docs/tsconfig.json"), Some(true));
        assert_eq!(
            verdict(&files, "apps/old/tsconfig.json"),
            Some(false),
            "the package alone means its tsconfig.json"
        );
        assert_eq!(verdict(&files, "apps/ext/tsconfig.json"), None);
    }

    #[test]
    fn comments_and_trailing_commas_are_allowed() {
        let config = r#"{
            // the compiler's settings
            "compilerOptions": {
                /* on */ "strict": true,
                "paths": { "@/*": ["./src/*"], },
                "baseUrl": "http://x//y",
            },
        }"#;
        assert_eq!(verdict(&[("tsconfig.json", config)], TSCONFIG), Some(true));
        let stripped = strip_jsonc(config);
        assert!(stripped.contains("http://x//y"), "{stripped}");
    }

    #[test]
    fn each_package_gets_its_own_scope_and_a_disagreeing_nested_config_blocks_its_parent() {
        assert_eq!(
            scope_of(Path::new(TSCONFIG)).as_deref(),
            Some("**/*.{ts,tsx}")
        );
        assert_eq!(
            scope_of(Path::new("apps/web/tsconfig.json")).as_deref(),
            Some("apps/web/**/*.{ts,tsx}")
        );
        let dir = project(&[
            ("tsconfig.json", r#"{"compilerOptions":{"strict":true}}"#),
            (
                "apps/web/tsconfig.json",
                r#"{"compilerOptions":{"strict":true}}"#,
            ),
            (
                "apps/legacy/tsconfig.json",
                r#"{"compilerOptions":{"strict":false}}"#,
            ),
        ]);
        let configs: Vec<PathBuf> = [
            TSCONFIG,
            "apps/web/tsconfig.json",
            "apps/legacy/tsconfig.json",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(
            strict_configs(dir.path(), &configs),
            vec![PathBuf::from("apps/web/tsconfig.json")],
            "the root's scope would cover apps/legacy, which is not strict"
        );
    }

    #[test]
    fn a_repository_strict_everywhere_gets_one_project_wide_scope() {
        let strict = r#"{"compilerOptions":{"strict":true}}"#;
        let configs = |names: &[&str]| names.iter().map(PathBuf::from).collect::<Vec<_>>();
        let all = project(&[
            ("apps/a/tsconfig.json", strict),
            ("apps/b/tsconfig.json", strict),
        ]);
        assert_eq!(
            strict_scopes(
                all.path(),
                &configs(&["apps/a/tsconfig.json", "apps/b/tsconfig.json"])
            ),
            vec!["**/*.{ts,tsx}".to_string()]
        );
        let some = project(&[
            ("apps/a/tsconfig.json", strict),
            ("apps/b/tsconfig.json", "{}"),
        ]);
        assert_eq!(
            strict_scopes(
                some.path(),
                &configs(&["apps/a/tsconfig.json", "apps/b/tsconfig.json"])
            ),
            vec!["apps/a/**/*.{ts,tsx}".to_string()]
        );
        assert!(strict_scopes(some.path(), &[]).is_empty());
    }

    #[test]
    fn the_nearest_config_governs_a_file() {
        let dir = project(&[("tsconfig.json", "{}"), ("apps/web/tsconfig.json", "{}")]);
        assert_eq!(
            nearest_tsconfig(dir.path(), Path::new("apps/web/src/a.ts")),
            Some(PathBuf::from("apps/web/tsconfig.json"))
        );
        assert_eq!(
            nearest_tsconfig(dir.path(), Path::new("tools/x.ts")),
            Some(PathBuf::from(TSCONFIG))
        );
    }
}

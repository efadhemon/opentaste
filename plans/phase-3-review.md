# Phase 3 review

**Reviewed:** 2026-10-29, against `main` at `6b103ae`
**Scope:** `crates/taste-core/src/ast/`, `crates/taste-core/src/diff/`, their tests and fixtures

## Verdict

The mechanical gates pass. The phase is **not free of bugs**, though. Every
item below was reproduced with a throwaway test against the real code (the
tests were removed afterwards and are not in the tree). Most of them are the
failure mode AGENTS.md warns about most: the code does not crash, it records a
**wrong rule** or **leaks a secret**.

What was checked and is fine:

| Gate | Result |
| --- | --- |
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo test --workspace` | 185 passed, 0 failed (148 baseline + 37 new) |
| Part 1: modules registered, no duplicate `tree-sitter` | yes |
| Analyzers count, `promote.rs` decides, `scan.rs` records only at the end | yes, as designed |
| Serial walk then `par_iter`, `thread_local!` parsers, `reset` only after `None` | yes |
| Every snippet passes through `scrub` inside `analyze` | yes, and tested |
| Every task in the Step 7 test list | present |

I could not re-check the manual gate (the `crisis-desk-ai` scan) or the CI run
on `48277d3`; they are taken from the plan file as written.

---

## Bugs to fix before phase 5 builds on this

All eleven were fixed on 2026-10-29, each with regression tests. The improvements section below is still open.

Ordered by how much damage each does. **S1** means a secret leaks or a wrong
rule gets recorded. **S2** means a rule is wrong in some projects, or a
detector misses common cases. **S3** is minor.

### B1 (S1). The scrubber misses common secret shapes

**Status: fixed.** Patterns and charset-aware entropy limits added in `diff/scrubber.rs`, with nine regression tests covering every shape below plus a false-positive guard. SPEC.md 5.3 updated.

`diff/scrubber.rs`. I fed `scrub` all five literals below. **None of them was
redacted:**

```ts
const k = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"; // 64-char hex secret
const w = "https://hooks.slack.com/services/T00000000/B00000000/XXXXXXXXXXXXXXXXXXXXXXXX";
const password = "hunter2hunter2hunter2";
const API_KEY = "sk_live_51Habcdefghijkl";
API_KEY="q8Zr2LmP0xT7vK3n"
```

There are four separate causes:

1. **Entropy cannot catch hex or short tokens.** Shannon entropy is capped at
   `log2(alphabet size)` and at `log2(token length)`. Hex has 16 symbols, so its
   maximum is exactly 4.0, and `ENTROPY_LIMIT` is 4.5 (line 17). Hex HMAC keys,
   API secrets and session secrets therefore can **never** be redacted. A
   17-character token also tops out at `log2(17) ≈ 4.09`, so in practice
   nothing under about 23 characters can pass 4.5 either. The 4.5 figure comes
   from SPEC.md, so the spec needs correcting along with the code. Use a
   threshold that depends on the character set (for example, about 3.0 for
   all-hex tokens of 32 characters or more), or scale it by token length.
2. **The URL allowlist is too broad.** `allowlisted` (line 131) skips every
   token that starts with `http://` or `https://`. Slack webhooks, presigned
   S3 URLs, and URLs with `?token=`/`?key=` query strings or `user:pass@`
   userinfo are all secrets. Allowlist only a URL's scheme and host, and keep
   checking its path and query.
3. **`env_assignment` misses quoted values** (line 186). The value group is
   `[^\s#'"]+`, so `API_KEY="..."` and `API_KEY='...'` never match, and those
   are the most common `.env` forms. It also matches only `SCREAMING_CASE`
   names, so `const password = "..."`, `apiKey: "..."` and `clientSecret = ...`
   in code are not covered. Both are cheap to fix.
4. **No vendor prefixes.** Only `AKIA` is matched. Add `ASIA` (temporary AWS
   keys), `ghp_`/`gho_`/`ghs_`/`github_pat_`, `xox[abposr]-`, `sk_live_`/`rk_live_`,
   `AIza`, `glpat-`, and JWTs (`eyJ…\.eyJ…\.…`). Prefix patterns are close to
   free of false positives.

Add a regression test with one case for each of the shapes above. This is the
gate SPEC.md section 5.3 cares most about, and the current test only covers the
`AKIA` example.

### B2 (S1). Decorator swap learns functional changes as taste

**Status: fixed.** `DecoratorSwap` now fires only from a framework decorator (known name, or imported from a bare package specifier) to one that is neither. Three tests in `tests/diff_detectors.rs`.

`diff/structural.rs:94`. Changing `@Get()` to `@Post()` on a controller method
yields the observation **"Use `@Post` instead of `@Get`."**, scoped to
`src/**/*.controller.ts`. The same happens for `@IsString` → `@IsNumber`,
`@ManyToOne` → `@OneToOne`, `@Injectable` → `@Controller`, and so on. These are
bug fixes or behaviour changes, not taste, and applying the resulting rule to
every controller is plainly wrong.

**Fix:** fire only when the swap goes from a framework decorator to a
project-specific one, which is the `@Entity` → `@AutoEntity` case the detector
exists for. Reuse `is_framework_decorator` from `ast/analyzers/arch.rs`. Never
fire when both names are framework decorators. Add `@Get` → `@Post` as a
negative fixture.

### B3 (S1). Contradiction detection decays rules because of untouched legacy code

**Status: fixed.** `contradictions(store, path, before, after)` counts structural occurrences on both sides and reports a rule only when `after` has more. PHASE-3 and PHASE-5 step 3 updated. Three new tests (legacy, moved, added beside legacy).

`diff/contradiction.rs:21`. `contradictions(store, path, after)` searches the
**whole** `after` file. Phase 5 step 3 will call it on every file a commit
touches. Take a rule such as "use `DomainError` instead of `throw new
Error(...)`" in a repository that still contains old `throw new Error`
lines: any commit that edits a single unrelated line in one of those files
decays the rule. Repeated commits demote it. I reproduced this: a file whose
only change was an added `const` line still reported the rule as
contradicted.

SPEC.md 5.1 says "if a developer **commits** code matching the anti_pattern".
That means new code, not code that was already there. **Fix:** add `before:
&str` to the signature and count a contradiction only when the anti-pattern
appears on lines that were added (reuse `changed_lines` and
`snippet_on_lines` from `structural.rs`), or only when `after` has more
occurrences than `before`. The function has no caller yet, so this is the
cheapest moment to change the signature. Update the PHASE-5 step 3 wording in
the same change.

### B4 (S1). `taste scan` brings back rules the developer deleted

**Status: fixed.** `cache.db` schema v2 adds `rejected_rules`; `delete_rule` records the id and `scan` skips it. Tests: v1→v2 migration, tombstone on delete, and a rescan that recomputes the ratio without reinstalling. Note: `cache.db` is not git-tracked, so a teammate's fresh clone can still re-infer the rule. A tracked tombstone would fix that, but it changes the `rules.json` schema, so it has been left for later.

`ast/scan.rs:167`. `already_recorded` checks only active rules and candidates.
`RuleStore::delete_rule` removes both and leaves no tombstone. The next scan
recomputes the same tally (cached snapshots included) and promotes the rule
again. I reproduced this: delete the promoted alias rule, rescan, and it comes
back as `Promoted` at confidence 1.0.

A developer who deletes a wrong rule has given the strongest negative signal
there is. **Fix:** record a rejection, for example a `rejected_rules` table in
`SqliteStore` or a disabled tombstone, and have `already_recorded` respect it.
The fix arguably belongs in `RuleStore::delete_rule` (phase 1). Phase 5's TUI
`[d]` key makes this visible to users.

### B5 (S2). The type ratio counts aliases that `interface` cannot express

**Status: fixed.** `types.rs` counts only `type_alias_declaration` whose value is an `object_type`. One test covers unions, function types, `keyof` and `Partial<…>`.

`ast/analyzers/types.rs:16`. Every `type_alias_declaration` counts on the
"type" side, including `type Status = 'a' | 'b'`, `type Fn = (x) => void`,
mapped types and `ReturnType<…>`. In my test, one interface plus one union
plus one function type tallied as 1 interface : 2 aliases. Typical TS
codebases have many unions, so the ratio leans toward **"Prefer `type`
aliases … for object types"** even in projects that always use `interface`
for objects. The emitted instruction says "for object types", so count only
aliases whose value is an `object_type`, which is what the instruction claims
to measure.

### B6 (S2). NestJS projects get wrong error and decorator rules

**Status: fixed.** `arch.rs` gained an `Origins` check that classifies a decorator or thrown class by where it was imported from, before falling back to names. JavaScript, DOM and NestJS built-in exceptions are ignored, and the framework decorator list now covers every name listed below. The diff `ErrorClass` detector uses the same `is_custom_error` and import check. Five tests.

- **Errors** (`ast/analyzers/arch.rs:72`, `diff/structural.rs:733`):
  `NotFoundException`, `BadRequestException`, `UnauthorizedException`,
  `ForbiddenException`, `ConflictException`, `InternalServerErrorException`
  and the rest are framework built-ins that end in `Exception`, so they count
  as **custom**. A stock NestJS service promotes "Prefer custom error classes
  over throwing bare `Error`" even though the project defines none. Rewriting
  `Error` as `NotFoundException` also fires the diff detector (reproduced).
  Add the `@nestjs/common` built-ins to `is_builtin_error` (or a separate
  framework list) and treat them as ignored, neither custom nor bare. Also
  ignore `AggregateError` and `DOMException`.
- **Decorators** (`ast/analyzers/arch.rs:148`): the framework list is missing
  many everyday names, so they count as custom. `@IsNotEmpty`,
  `@InjectRepository` and `@CreateDateColumn`, plus `@HttpCode`, all counted
  as custom (4 : 0 in my test). Missing:
  - NestJS: `Inject`, `Optional`, `Global`, `HttpCode`, `Header`, `Redirect`,
    `Render`, `SetMetadata`, `UseFilters`, `Version`, `Sse`, `All`, `Options`,
    `Head`, `Ip`, `Session`, `UploadedFile(s)`, `Next`
  - Scheduling, events, microservices: `Cron`, `Interval`, `OnEvent`,
    `MessagePattern`, `EventPattern`, `WebSocketGateway`, `SubscribeMessage`,
    `WebSocketServer`
  - GraphQL: `Resolver`, `Field`, `ObjectType`, `InputType`, `Mutation`,
    `Args`, `ResolveField`
  - Mongoose: `Prop`, `Schema`, `InjectModel`
  - TypeORM: `CreateDateColumn`, `UpdateDateColumn`, `DeleteDateColumn`,
    `VersionColumn`, `Unique`, `BeforeInsert`, `AfterLoad`, `RelationId`
  - class-validator: `IsNotEmpty`, `IsInt`, `IsEnum`, `IsArray`, `IsUUID`,
    `IsDate`, `IsDefined`, `Length`, `MinLength`, `MaxLength`, `Matches`,
    `IsIn`, `ArrayNotEmpty`
  - class-transformer: `Type`, `Expose`, `Exclude`, `Transform`
  - Angular: `Inject`, `Optional`, `ContentChild(ren)`, `ViewChildren`
  - Lit and MobX: `customElement`, `property`, `state`, `observable`,
    `action`, `computed`

  A denylist will always have gaps. A more robust signal is to classify a
  decorator as framework when its identifier is **imported from a package in
  `node_modules`** (a bare specifier), and as custom when it is imported from a
  relative or alias path. The import table is already parsed in the same file.
  This would make the dimension trustworthy.

### B7 (S2). The scan reads generated and vendored code when there is no `.gitignore`

**Status: fixed.** `scan.rs` filters the listed directories and suffixes through `WalkBuilder::filter_entry`, never the root itself. One test.

`ast/scan.rs:65`. Without a `.gitignore`, `node_modules/`, `dist/` and similar
directories are walked. `*.d.ts` files are always walked, and so are
`*.min.js` files under 1 MiB. In my test, `node_modules/pkg/index.ts`,
`dist/out.js` and `src/types.d.ts` were all parsed. Declaration files are
almost entirely `export interface`/`export declare`, which skews both the
exports and the type ratios. Always skip `node_modules`, `dist`, `build`,
`out`, `.next`, `coverage` and `vendor`, plus `*.d.ts`, `*.min.js` and
`*.bundle.js`, through `ignore::overrides` or a filter, whatever the
`.gitignore` says.

### B8 (S2). The import-rewrite detector and the scanner disagree on the "relative" rule text

**Status: fixed.** `ImportRewrite` fires only between alias and deep relative and uses `PREFER_PATH_ALIASES` / `PREFER_DEEP_RELATIVE`. `PREFER_LOGGER` is a `pub const`. `RuleStore::record_observation` now returns a new `Outcome::Conflicts` and records nothing when the observation is the opposite of a confirmed rule. Three tests.

`diff/structural.rs:156` emits the literal "Prefer relative imports over path
aliases (`@/` or `~/`).", while the scanner emits `PREFER_DEEP_RELATIVE` ("Prefer
deep relative imports (`../../`) …"). The two have different text and
therefore different rule ids, and `opposite_instruction` knows neither one as
the opposite of the other. So a diff-learned "relative" rule and a
scan-learned "alias" rule can both end up in `CLAUDE.md` at once,
contradicting each other. **Fix:** export one constant for each side, use
it in both places, and add it to `opposite_instruction`. The same applies to the
logger instruction, which should be a `pub const` as well.

More generally, nothing stops a diff observation from installing the opposite
of an existing rule. `already_recorded`'s opposite check exists only in
`scan.rs`. Consider moving it into `RuleStore::record_observation` so every
source respects it.

### B9 (S2). The control-flow diff detector ignores class methods and arrow functions

**Status: fixed.** `FLOWS` covers methods and `const` arrows. Two tests.

`diff/structural.rs:848`. `FLOWS` matches only `function_declaration`. The
scanner's `BODIES` query also matches `method_definition` and `const` arrows.
In a NestJS or Angular codebase, almost all logic lives in class methods, so
this detector never fires there. I confirmed that refactoring a method from
if/else to a guard clause yields nothing. Key methods as `Class.method` and
arrows by their binding name, the same way `DecoratorSwap` keys members.

### B10 (S3). The logger detector gives up after the first unmatched `console` call

**Status: fixed.** One test.

`diff/structural.rs:187`. The `?` on `.find(...)` returns from `detect` on the
first removed `console.*` call that has no matching logger call, so later
calls that *were* replaced are never checked. Reproduced: before has
`console.log('a'); console.log('b')`, after has `foo(); logger.info('b')`,
and the result is 0 observations. Use `let Some(logger) = … else { continue };`.

### B11 (S3). Throws and functions are keyed loosely

**Status: fixed.** `functions`, `type_decls`, `throws` and `flows` are keyed by enclosing scope (`qualified`), and keys that occur twice are dropped instead of letting the last one win (`unique`). Two tests, both checked to fail under the old keying.

- `throws()` (line 836) keys by the argument text alone, so every
  `throw new X()` with empty arguments collides on `"()"` and the last one
  wins. `grounded` hides most of the resulting noise, but key by enclosing
  function and arguments, or pair by diff hunk.
- `functions()`, `type_decls()` and `flows()` key by bare name, so nested or
  same-named functions in different scopes overwrite each other.

---

## Improvements (not bugs)

1. **Snippet size cap.** `FunctionSyntax`, `TypeSwap` and `ControlFlow` store
   the *entire* function, interface or if/else node as `anti_pattern` and
   `preferred_pattern`. A 200-line function ends up in `.taste/rules.json` and
   possibly in the editor targets. Truncate to the signature plus a few lines,
   or around 400 characters, before scrubbing. Short snippets are also more
   useful to `contradictions`.
   **Status: done.** Snippets are capped at `MAX_SNIPPET_LINES` (12) and
   `MAX_SNIPPET_BYTES` (400), cut at a line boundary, and end in a `// …`
   comment. Contradiction matching skips comments, so a truncated
   anti-pattern can never match on its first few lines alone. Scrubbing runs
   *before* the cut, so a secret straddling the cut is redacted rather than
   left as an unrecognisable prefix. The scrubbing order is checked by a test
   that fails if the two steps are swapped.
2. **Skip trees with errors in `diff::analyze`.** A half-typed buffer (common
   with the phase 6 extension) parses into error-recovery trees whose node
   shapes may be wrong. Return early when `root_node().has_error()` on either
   side, or at least when the changed region overlaps an `ERROR` node.
   **Status: done**, the narrower way. An edit is skipped when an `ERROR` or
   `MISSING` node covers any removed line of `before` or added line of
   `after`. An error elsewhere in the file does not block it, so a file
   that uses syntax the grammar lacks can still teach. The half-typed test
   uses two buffers that do produce observations when the guard is
   removed.
3. **Analyzer parses before checking the language.** `analyze_source`
   (`ast/analyzers/mod.rs:133`) parses Rust, Python and Go files and then throws
   the tree away. Check `is_script()` first. Better still, skip non-script files
   in `scan` before reading them, since today they are read, hashed, parsed,
   and snapshotted for no counts at all.
4. **Read and hash in parallel.** `scan` reads and SHA-256-hashes every file in
   the serial loop, and only the parse runs on rayon. Moving the read, hash and
   snapshot lookup into `par_iter` (the snapshot lookup is a cheap SQLite read,
   or load all snapshots into a `HashMap` first) should help the measured 321 ms
   release time on the 991-file tree get closer to the spec.
5. **Stale snapshots.** Snapshot rows for deleted files are never removed.
   Prune paths the walk did not see.
6. **Monorepo scope.** `scope_for_path("apps/api/src/x.service.ts")` gives
   `apps/**/*.service.ts`, which covers every app. For monorepos, consider
   scoping to the nearest directory that has a `package.json`.
7. **Monorepo stack detection.** `detect_stack` reads only the root
   `package.json`. Workspaces (`apps/*/package.json`) are not detected.
8. **Contradiction checks are probably too strict for decorators.** A decorator
   anti-pattern like `@Get()` is shorter than `MIN_NEEDLE` (8), so it can never
   contradict. That is safe, but rules from `DecoratorSwap` will rarely decay.
   Once B2 is fixed, match decorators by name structurally instead.
9. **Doc mismatch.** `MAX_FILE_BYTES` says files "at or above" the size are
   skipped, but the code uses `>`.
10. **Exports counting.** Re-export barrels (`export * from`, `export { x } from`)
    and `export type {…}` count as named exports. That is defensible, but it
    inflates the "named" side in barrel-heavy projects. Consider excluding
    re-exports.
11. **Test gaps worth adding with the fixes:** a scrubber case per secret shape
    in B1; negative fixtures for `@Get`→`@Post` and `Error`→`NotFoundException`;
    a scan test with a `node_modules` directory and no `.gitignore`; a
    union-type file that must not count toward the alias side; a test that a
    deleted rule stays deleted after a rescan; and a contradiction test where
    the anti-pattern is unchanged legacy code.

12. **Vendor patterns need a word boundary** (found while testing 1). The
    `\b` in front of `AKIA`, `ghp_` and the rest means a key glued to the
    preceding text (`xxxAKIA…`) is not matched, and a long run of
    low-entropy text around it hides it from the entropy check too. That is
    rare in real code, but `(?:^|[^A-Za-z0-9])` in place of `\b` would close
    it for the prefixes that cannot start an English word.

## Suggested order

All of B1 to B11 have landed, and so have improvements 1 and 2. The rest are
optional and none of them blocks phase 5.

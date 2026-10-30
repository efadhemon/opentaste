# Phase 9: The missing TypeScript detectors

**Status:** ongoing

| | |
| --- | --- |
| **Blocked by** | [Phase 7](PHASE-7.md): package scopes (a `tsconfig.json` belongs to one package), versioned tallies |
| **Blocks** | [Phase 10](PHASE-10.md) |
| **Rough size** | 2–3 sessions |
| **Rows** | [V1-AUDIT.md](V1-AUDIT.md) M3, M4, M5 |

SPEC.md §2 lists three detectors the first release left out: strict null
checks and zod schemas in `types.rs`, and dependency-injection patterns in
`arch.rs`. AGENTS.md recorded why they were deferred; each step below answers
that reason instead of ignoring it.

---

## Step 1: Strict null checks (M3)

*Deferred because:* "a `tsconfig.json` setting, not a per-file statistical
signal." True, which makes it the most certain signal OpenTaste can have: the
compiler enforces it. It is read from configuration, not counted.

- [x] Read `compilerOptions.strict` and `compilerOptions.strictNullChecks`
      from each package's `tsconfig.json`, following `extends` within the
      repository (not into `node_modules`). `strictNullChecks` overrides
      `strict`. (`ast/tsconfig.rs`. "Within the repository" includes the
      repository's own workspace packages by name, `"extends":
      "@acme/tsconfig/base.json"`, which is how create-t3-turbo and most
      Turborepo setups share a base; without it they got no rule. A base from
      an installed package makes the answer unknown unless the file sets
      `strictNullChecks` itself, because the base could turn it off under
      `strict`.)
- [x] When it is on, record a scan observation, category `types`, scoped to
      that package's TypeScript files: "This code compiles with
      `strictNullChecks`: handle `null` and `undefined` explicitly; do not
      silence them with the non-null assertion `!`." Confidence 0.95, the
      same trust as an explicit statement, because the compiler states it.
      (When every `tsconfig.json` in the repository is strict, one
      project-wide rule replaces a line per package; a package is skipped when
      a nested `tsconfig.json` under it disagrees, since its scope would
      cover those files.)
- [x] When it is off, record nothing. A missing setting is not a preference.
      *Added:* when it is turned off, `taste scan` retracts the rule it
      recorded (`RuleStore::retract_rule`, which removes without rejecting,
      so turning it on again restores the rule). A ratio never retracts a
      rule; a compiler setting is a fact.
- [x] Diff detector: a non-null assertion `x!.y` replaced by `x?.y` or an
      explicit check, in a project with strict null checks on. (`analyze_in`
      reads the file's nearest `tsconfig.json`; plain `analyze`, which has no
      project, never reports it.)
- [x] Tests: `strict: true`; `strict: true` with `strictNullChecks: false`
      (off); an `extends` chain; two packages with different settings get
      two scopes; JSON with comments (tsconfig allows them); the diff
      detector's positive and negative.
- [x] `taste watch` rescans when a `tsconfig.json` changes.

Checked on create-t3-turbo (one project-wide rule, from eleven packages that
all extend the workspace's `@acme/tsconfig`), full-stack-fastapi-template
(two package scopes) and OpenTaste.

**Found while checking:** running `taste init` on this repository showed
that the marker splicer took the mention of `<!-- OPENTASTE:START -->` in
AGENTS.md's own prose for a block and replaced the text up to the next
mention. Markers now count only on their own line outside fenced code
(`transpiler/marker.rs`).

## Step 2: Zod schemas (M4)

*Deferred because:* "plausible but not worth a detector in the first
release." The risk is mistaking one schema in a utility file for a project
convention.

- [ ] Scan dimension "validation library": per file, which of zod, yup,
      joi, class-validator, valibot and io-ts it imports and uses. Files
      that validate nothing count for no side.
- [ ] A rule only when one library is used in at least 80% of the files
      that validate anything, across at least 10 such files: "Validate input
      with zod schemas" (or whichever library wins).
- [ ] Secondary rule, same thresholds, zod only: types derived with
      `z.infer<typeof Schema>` versus a hand-written interface next to the
      schema.
- [ ] Diff detector: a yup/joi schema replaced by the zod equivalent in one
      edit (import source swap plus schema call).
- [ ] Tests: unanimous zod; mixed zod/yup below 80%; 9 files (below the
      floor); `z.infer` counting; a file that only imports types from zod
      counts nothing.

## Step 3: Dependency injection (M5)

*Deferred because:* "DI shape varies too much between frameworks to count
reliably." So it is counted per framework, only where the framework makes
the choice visible, and nothing is emitted outside those frameworks.

- [ ] NestJS (`@Injectable`, `@Controller`) and Angular (`@Injectable`,
      `@Component`): constructor-parameter injection versus the `inject()`
      function (Angular 14+) versus property injection (`@Inject()` on a
      field).
- [ ] Any of those classes: a dependency created with `new SomeService(…)`
      inside the class versus injected. Rule: "Inject services instead of
      constructing them with `new`."
- [ ] The framework is taken from `detected_stack` and the decorator's
      import source; a class with a look-alike decorator from elsewhere does
      not count.
- [ ] Diff detector: `private readonly x = new XService()` replaced by a
      constructor parameter or `inject(XService)`.
- [ ] Tests: Nest constructor injection; Angular `inject()`; a mixed
      Angular project under 80%; `new Date()` and `new Map()` are not
      services; a class without a DI decorator counts nothing.
- [ ] **Precision gate:** a real NestJS project and a real Angular project;
      every rule emitted is agreed with by a human and recorded here.

## Step 4: The full test set

- [ ] Strict null: config reading, `extends`, comments in JSON, per-package
      scope, the diff detector.
- [ ] Zod: thresholds, `z.infer`, type-only imports, the diff detector.
- [ ] DI: each framework's styles, `new` of non-services ignored, look-alike
      decorators ignored, the diff detector.

---

## Definition of done

- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
      warnings` and `cargo test --workspace` clean.
- [ ] Both DI precision gates and a zod precision gate (one real project that
      uses zod) recorded.
- [ ] AGENTS.md's table rows for strict null, zod and DI removed.
- [ ] V1-AUDIT M3, M4 and M5 marked fixed.
- [ ] Status at the top of this file set to `completed`.

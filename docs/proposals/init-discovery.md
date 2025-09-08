Centralizing **init** and **init**.py Handling in Cribo

Overview

- Goal: eliminate scattered, inconsistent checks for "**init**" and "**init**.py" by introducing a single, well‑typed API for package/module identification and naming.
- Benefits: correctness (PEP 420 namespace packages), fewer bugs, simplified reasoning, and easier maintenance.

Alignment with ModuleId (entry = 0)

- This proposal explicitly extends docs/proposals/module-id-usage.md: ModuleResolver owns module identity and ModuleId(0) is the entry.
- Identity and behavior checks should use ModuleId and resolver‑backed metadata (not string heuristics):
  - Entry checks: `module_id.is_entry()` (never path/name comparisons).
  - Package init checks: query resolver for package/init semantics instead of filename tests.
  - Path/name utilities remain for pre‑registration classification only; after registration, Resolver is the single source of truth.

Current Findings (inventory)

- orchestrator.rs
  - Directory entry detection uses string equality checks for "**main**.py" and "**init**.py" (e.g., lines around 377–420).
  - Special-cases entry name "**init**.py" to force module name "**init**" (around 945–951).
  - Namespace package skipping uses is_dir checks without shared helpers.
- resolver.rs
  - Repeated checks for packages via `path.file_name() == "__init__.py"` (e.g., ~133, ~782–784).
  - Resolution search prefers `foo/__init__.py` over `foo.py` via repeated inlined logic (~600–665, ~720+).
  - Relative import resolution infers “package-ness” from `name.ends_with(".__init__")` heuristics (179–182).
- code_generator/import_deduplicator.rs
  - Detects `__init__.py` via filename equality (around 165–172) to relax trimming rules.
- code_generator/module_transformer.rs and import_transformer.rs
  - Heuristics and comments refer to package `__init__` behaviors but lack shared predicates.
- analyzers/dependency_analyzer.rs
  - Uses `contains("__init__")` in multiple heuristics for cycle classification (e.g., 86, 113, 141, 175).
- util.rs
  - Has pieces of the solution: `module_name_from_relative`, `is_init_module("pkg.__init__")`, and `is_special_module_file` (works on extension-stripped filenames). Useful but incomplete for path-based checks.
- ast_builder/module_wrapper.rs
  - Defines `MODULE_INIT_ATTR: "__init__"` constant for AST generation; should come from a shared constants module.

Problems

- String/partial matching: `contains("__init__")` and ad-hoc `ends_with("__init__.py")` appear across modules, risking subtle bugs.
- Duplication: path → module name conversions and package checks are reimplemented in multiple places.
- Incomplete handling: namespace packages (PEP 420) supported in some places, but not uniformly enforced via shared helpers.
- Inconsistent naming: resolver and orchestrator special-case naming in different ways.

Design Principles

- Single source of truth: centralize filename literals and predicates.
- Path-segment based decisions: never rely on substring/contains; inspect `Path` segments or dotted module segments.
- Canonicalization: store and compare canonical module names consistently (e.g., `pkg`, not `pkg.__init__`). Expose kind metadata for when the distinction matters.
- PEP 420 correctness: clean helpers for package dir with and without `__init__.py`.
- Ergonomic API: simple, explicit functions with clear names and types.

Proposed Architecture

1. Constants module

- File: `crates/cribo/src/python/constants.rs`
- Purpose: define all magic names once.
- API:
  - `pub const INIT_FILE: &str = "__init__.py";`
  - `pub const INIT_STEM: &str = "__init__";`
  - `pub const MAIN_FILE: &str = "__main__.py";`
  - `pub const MAIN_STEM: &str = "__main__";`

2. Module path + naming utilities

- File: `crates/cribo/src/python/module_path.rs`
- Purpose: canonical conversion and identification of module/package roles.
- Types:
  - `pub enum ModuleKind { RegularModule, PackageInit, NamespacePackageDir, Main }`
  - `pub struct ModuleIdInfo { pub name: String, pub kind: ModuleKind }`
- API (path-focused):
  - `pub fn is_init_file_name(name: &str) -> bool` — exact match against `INIT_FILE`.
  - `pub fn is_init_stem(stem: &str) -> bool` — exact match against `INIT_STEM`.
  - `pub fn is_main_file_name(name: &str) -> bool` — exact match against `MAIN_FILE`.
  - `pub fn is_special_entry_file_name(name: &str) -> bool` — `is_init_file_name || is_main_file_name`.
  - `pub fn is_init_path(path: &Path) -> bool` — `path.file_name() == INIT_FILE`.
  - `pub fn is_package_dir_with_init(dir: &Path) -> bool` — checks `dir.join(INIT_FILE).is_file()`.
  - `pub fn is_namespace_package_dir(dir: &Path) -> bool` — `dir.is_dir() && !is_package_dir_with_init(dir)` (PEP 420).
  - `pub fn module_name_from_relative(path: &Path) -> Option<String>` — move existing logic from `util.rs` here and make it the canonical implementation.
  - `pub fn classify_path(path: &Path) -> ModuleKind` — inspects path to classify kind.
  - `pub fn parent_package_dir(path: &Path) -> Option<&Path>` — for `__init__.py`, returns parent; for `foo.py`, returns parent dir.
- API (name-focused):
  - `pub fn is_init_module_name(name: &str) -> bool` — replaces all `ends_with(".__init__")` and `contains("__init__")` heuristics.
  - `pub fn canonical_module_name(name: &str) -> Cow<'_, str>` — returns `pkg` for `pkg.__init__`, otherwise returns original.
  - `pub fn parent_package_name(name: &str) -> Option<String>` — useful for relative import resolution.

Note on ownership with ModuleId

- Before a module is registered (e.g., while probing filesystem), use `module_path` helpers.
- After registration (ModuleId allocated), use Resolver metadata exclusively for kind/semantics.

3. Public facade for consumers

- Re-export a minimal, consistent surface from `crates/cribo/src/python/mod.rs`.
- Deprecate direct string literals in production code (see CI guardrail).

4. ModuleId integration in Resolver

- Evolve resolver metadata to be the authoritative source for “init-ness” and package kind:
  - Today: `ModuleMetadata { is_package: bool }` means path was an `__init__.py`.
  - Proposed: replace the boolean with an enum to capture full semantics from this proposal:
    - `pub enum ModuleKind { RegularModule, PackageInit, NamespacePackageDir, Main }`
    - `pub struct ModuleMetadata { id: ModuleId, name: String, canonical_path: PathBuf, kind: ModuleKind }`
- Public resolver queries (post‑registration):
  - `fn kind(&self, id: ModuleId) -> Option<ModuleKind>`
  - `fn is_package_init(&self, id: ModuleId) -> bool` (sugar over kind)
  - Existing `get_module_name`, `get_module_path` remain.
  - Keep `ModuleId::ENTRY` for entry logic.

Unified Invariants

- Canonical identity: in registries, a package module’s canonical name is its package name (e.g., `mypkg`), not `mypkg.__init__`.
- Kind preserved: where behavior depends on being an init module (e.g., import trimming rules, wrapper generation), use `ModuleKind::PackageInit` instead of string checks.
- Path comparisons use `Path`/`OsStr`, not string `contains`/`ends_with` on the whole path.

Callsite Integration Map

- orchestrator.rs
  - Pre‑registration: replace `filename == "__init__.py" || filename == "__main__.py"` with `is_special_entry_file_name(filename)`.
  - Pre‑registration: replace direct `__init__.py` fallback logic with `is_init_path(entry_path)` and `classify_path`.
  - Registration: once entry is registered, rely on `ModuleId::ENTRY` and `resolver.kind(ModuleId::ENTRY)` for behavior.
  - Use `module_path::module_name_from_relative` for module name resolution; remove duplicate helper.
  - Namespace package checks use `is_namespace_package_dir` for clarity (pre‑registration); after registration, kind is read from resolver.
- resolver.rs
  - Replace all direct `"__init__.py"` filename checks with `is_init_path` / `is_package_dir_with_init` (internal usage only).
  - In `ModuleRegistry`, set `kind` via `classify_path` at registration time.
  - In resolution search, use `is_package_dir_with_init`/`is_namespace_package_dir` instead of manual `join("__init__.py")` and `is_dir` chains.
  - In relative import name resolution, replace `ends_with(".__init__")` with `is_init_module_name` (pre‑registration name math only). Prefer using `ModuleId` and registry kind when context is available.
- code_generator/import_deduplicator.rs
  - Prefer `resolver.is_package_init(module_id)` over path/filename checks (post‑registration everywhere in codegen).
  - Where behavior requires “package init semantics,” gate on `resolver.kind(module_id) == ModuleKind::PackageInit`.
- analyzers/dependency_analyzer.rs
  - Replace all `contains("__init__")` heuristics with `resolver.is_package_init(id)` where IDs are available; fallback to `is_init_module_name` only where names exist without IDs.
- ast_builder/module_wrapper.rs
  - Replace the local `MODULE_INIT_ATTR` with `python::constants::INIT_STEM`.
- Any other `__init__` usages
  - Prefer `canonical_module_name` for graph keys, map lookups, and display when deduplication matters.

Data Ownership and Flow (ModuleId‑first)

- Single source of truth: ModuleResolver owns module identity and kind after registration.
- Utilities are for discovery/classification before registration only (filesystem probing, entry selection, etc.).
- After `register_module`, all components should flow ModuleId and query the resolver for name, path, and kind.

Tests and Validation

- Unit tests
  - Add focused tests in `python/module_path.rs` for:
    - `is_init_file_name`, `is_init_stem`, `is_init_path`.
    - `is_package_dir_with_init`, `is_namespace_package_dir`.
    - `module_name_from_relative` behavior for `pkg/__init__.py`, `pkg/__main__.py`, `pkg/mod.py`, nested packages, and root `__init__.py`.
    - `canonical_module_name` and `is_init_module_name` correctness.
- Snapshot fixtures (reuse mandatory framework in `tests/test_bundling_snapshots.rs`)
  - `namespace_package_basic/` (no `__init__.py`): runtime lookup and import rewriting remain correct.
  - `init_vs_module_preference/` with both `foo/__init__.py` and `foo.py` (already present: module_first_resolution); ensure behavior unchanged.
  - `relative_imports_in_init/` testing `from . import x` and re-exports from package `__init__.py`.
  - `package_dir_entry/` when entry is a directory: `__main__.py` preferred, fallback to `__init__.py`.
- End-to-end
  - Run `cargo nextest run --workspace` and accept snapshots for fixtures touched.

CI Guardrail (prevent regressions)

- Add a lightweight check script (e.g., `scripts/lint-init-usage.sh`) that fails CI if forbidden literals are used outside the allowlist:
  - Forbid raw occurrences of `"__init__.py"` and `".__init__"` in `crates/cribo/src/**` except within `python/constants.rs`, `python/module_path.rs`, and tests.
  - Implementation can be a simple `rg` with path filters; integrate into CI or `make lint`.
- Rationale: absent a custom Clippy lint for string patterns, a repo-level check keeps usage centralized.

Migration Plan (incremental, low risk)

1. Add `python/constants.rs` and `python/module_path.rs` with unit tests.
2. Update `util.rs` to re-export or delegate to `python::module_path::module_name_from_relative`; deprecate duplicate helpers in code comments and migrate callsites.
3. Replace callsites in orchestrator, resolver, codegen, and analyzers with the new helpers (mechanical changes):
   - Filename equality → `is_init_file_name` / `is_special_entry_file_name`.
   - Path equality → `is_init_path`.
   - Directory classification → `is_package_dir_with_init` / `is_namespace_package_dir`.
   - Name heuristics → `is_init_module_name` / `canonical_module_name`.
4. Run tests and snapshot fixtures; use `cargo insta accept` where behavior is unchanged but formatting of logs/snapshots is stabilized.
5. Add CI guardrail script; fix or whitelist legitimate occurrences inside the new modules.
6. Remove dead/duplicate code per project policy (no deprecations; immediate cleanup).

Behavioral Invariants to Preserve

- `module_name_from_relative("pkg/__init__.py") == Some("pkg")` and `== Some("pkg")` for `pkg/__main__.py`.
- Resolver still prefers `foo/__init__.py` over `foo.py`.
- Canonical graph identity is by package name, not by `.__init__` suffix; `ModuleKind` (stored in resolver metadata) retains the distinction when needed by codegen/analysis.
- Namespace packages remain discoverable without requiring `__init__.py`.

Edge Cases and Notes

- Windows/macOS/Linux: comparisons should use `OsStr`/`Path` equality; do not lowercase or normalize names beyond what Python import semantics require.
- Symlinks: keep using canonicalized paths for identity when possible; the helpers should accept paths as-is and leave canonicalization to callers that need it.
- Logging: when referring to packages, prefer canonical names (no `.__init__` suffix) in user-facing messages.

Optional Future Enhancements

- Consider a small `PythonModuleRef { path, name, kind }` helper passed through pipelines where both name and kind are needed, reducing re-computation.
- If needed, add a custom `xtask` to run the guardrail and more static checks locally.

Summary

- Introduce a `python::{constants, module_path}` subsystem to own all `__init__`/`__init__.py` knowledge.
- Replace substring/equality checks with explicit, shared predicates and a small `ModuleKind` enum; store `ModuleKind` in resolver metadata tied to `ModuleId`.
- Migrate callsites to ModuleId‑centric checks; preserve behavior and simplify code.
- Add tests and a CI guardrail to keep logic centralized and correct going forward.

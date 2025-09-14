# Split Import Transformer, Part 2 — Bottom‑Up Refactor Plan

Objective

- Continue splitting the 30k‑token `import_transformer` monolith into small, focused handler files by moving real logic (not wrappers), keeping behavior identical and tests green.
- Use a strict bottom‑up extraction order to avoid line shifting during edits.
- After each extraction: compile (`cargo check`), run tests (`cargo nextest run`), then commit.

Ground Rules

- Move code verbatim into handler files; adjust only call signatures/receivers and `use` imports as needed. Do not reformat or “clean up” logic during moves.
- Prefer free functions in handler modules that take `&Bundler` and typed args rather than methods on `RecursiveImportTransformer`.
- Keep Bundler’s helper functions as the “source of truth” while moving call sites. Only change helper visibility with the minimal scope (`pub(in crate::code_generator)`), no behavior changes.
- One extraction per commit; run tests after each.

Current Anchors (commit baseline: post-wrapper wildcard move)

- File: `crates/cribo/src/code_generator/import_transformer/mod.rs`
  - `fn handle_import_from` body: lines 1296–1415
    - Resolve relative module + dedup precheck: 1324–1367
    - Submodule handler hook: 1369–1375
    - Resolved handlers (inlined/wrapper): 1377–1391
    - Standard rewrite fallback: 1393–1415
  - `fn has_bundled_submodules`: 1840–1860
  - `fn rewrite_import_from`: 1876–2248
    - Resolve module name block: 1910–1937
    - Unresolved absolute fallback (“keep original”): 1954–1962
    - Not-in-bundled triage (absolute or relative, non-wrapper): 1965–2075
      - Transform if submodules bundled: 1978–2001
      - Inlined module fast-path: 2004–2023
      - Wrapper module fast-path (dispatch to handler): 2025–2041
      - Relative special cases (entry/**main**/panic): 2044–2136
      - Absolute non-bundled keep original: 2137–2139
    - Bundled module path: 2141–2248
      - Wrapper module branch (rel→abs + handler call): 2149–2186
      - Inlined/bundled-submodules path: 2188–2201
      - Inlined module assignments block: 2204–2247
- File: `crates/cribo/src/code_generator/bundler.rs`
  - `pub(super) fn handle_symbol_imports_from_multiple(..) -> Vec<Stmt>`: 656–1210

Naming and Destinations (handlers)

- `crates/cribo/src/code_generator/import_transformer/handlers/fallback.rs` — trivial keep/panic paths.
- `handlers/relative.rs` — relative import special cases.
- `handlers/inlined.rs` — inlined module from‑import transformations (already exists; we’ll add new entry points).
- `handlers/wrapper.rs` — wrapper module from‑import transformations (already exists; we’ll add new entry points).

Bottom‑Up Extraction Plan (precise ranges and steps)

Step 1 — Extract absolute non‑bundled fallback (keep original)

- Source range: `import_transformer/mod.rs:2137–2139`.
- New function: `handlers::fallback::keep_original_from_import(import_from: &StmtImportFrom) -> Vec<Stmt>`.
- Dest file: `crates/cribo/src/code_generator/import_transformer/handlers/fallback.rs`.
- Call site change: replace the 3‑line `return vec![Stmt::ImportFrom(import_from)];` with `return FallbackHandler::keep_original_from_import(import_from);`.
- Validation: `cargo check && cargo nextest run`.
- Commit: `refactor(import_transformer): extract non-bundled absolute fallback to handlers/fallback.rs`

Step 2 — Extract wrapper bundled branch (rel→abs conversion + handler dispatch)

- Source range: `import_transformer/mod.rs:2149–2186`.
- New function:
  - `handlers::wrapper::WrapperHandler::handle_wrapper_from_import_absolute_context(
        bundler: &Bundler,
        import_from: &StmtImportFrom,
        module_name: &str,
        inside_wrapper_init: bool,
        at_module_level: bool,
        current_module: Option<&str>,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        function_body: Option<&[Stmt]>,
     ) -> Vec<Stmt>`
- Notes: move the rel→abs conversion (2156–2175) verbatim into the new function before calling existing `rewrite_from_import_for_wrapper_module_with_context`.
- Call site change: replace the full `if` body with a single `return` to the new function.
- Validation + Commit:
  - `cargo check && cargo nextest run`
  - `refactor(import_transformer): extract wrapper absolute-context branch to handlers/wrapper.rs`

Step 3 — Extract inlined bundled branch (namespace/submodules + assignments)

- Source ranges:
  - Submodules transform within this branch: `2188–2201`.
  - Inlined assignments block: `2204–2247`.
- New function:
  - `handlers::inlined::InlinedHandler::handle_inlined_from_import_absolute_context(
        bundler: &Bundler,
        import_from: &StmtImportFrom,
        module_name: &str,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        inside_wrapper_init: bool,
     ) -> Vec<Stmt>`
- Implementation: move 2188–2247 verbatim; keep the inner call to `namespace_manager::transform_namespace_package_imports` and the `create_assignments_for_inlined_imports` invocation as-is.
- Call site: replace entire `else { ... }` (starting at 2188) with a single `return InlinedHandler::handle_inlined_from_import_absolute_context(..);`.
- Validation + Commit:
  - `cargo check && cargo nextest run`
  - `refactor(import_transformer): extract inlined absolute-context branch to handlers/inlined.rs`

Step 4 — Extract relative import special cases (entry/**main**/panic)

- Source range: `import_transformer/mod.rs:2091–2136`.
- New function:
  - `handlers::relative::RelativeHandler::handle_unbundled_relative_import(
        bundler: &Bundler,
        import_from: &StmtImportFrom,
        module_name: &str,
        current_module: &str,
     ) -> Vec<Stmt>`
- Implementation: move the entire block verbatim; function either returns `vec![Stmt::ImportFrom(import_from)]` for the `__main__` case or panics identically for others.
- Call site: replace 2091–2136 with a single `return RelativeHandler::handle_unbundled_relative_import(..);`.
- Validation + Commit:
  - `cargo check && cargo nextest run`
  - `refactor(import_transformer): extract relative special-cases to handlers/relative.rs`

Step 5 — Extract entry‑module resolution to inlined fast‑path

- Source range: `import_transformer/mod.rs:2073–2088`.
- New function: `handlers::inlined::InlinedHandler::handle_entry_relative_as_inlined(
      bundler: &Bundler,
      import_from: &StmtImportFrom,
      module_name: &str,
      symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
      inside_wrapper_init: bool,
      current_module: &str,
   ) -> Vec<Stmt>`
- Call site: wrap the `if let Some(module_id) = entry_module_id { ... }` in a single `return` to the new function.
- Validation + Commit:
  - `cargo check && cargo nextest run`
  - `refactor(import_transformer): extract entry relative inlined fast-path to handlers/inlined.rs`

Step 6 — Extract unresolved absolute import fallback (top of rewrite)

- Source range: `import_transformer/mod.rs:1954–1962`.
- New function: reuse `FallbackHandler::keep_original_from_import` from Step 1.
- Call site: replace with `return FallbackHandler::keep_original_from_import(&import_from);`.
- Validation + Commit:
  - `cargo check && cargo nextest run`
  - `refactor(import_transformer): extract unresolved absolute fallback to handlers/fallback.rs`

Step 7 — Extract “transform if submodules bundled” fast‑path

- Source range: `import_transformer/mod.rs:1978–2001`.
- New function: `handlers::inlined::InlinedHandler::transform_if_has_bundled_submodules(
      bundler: &Bundler,
      import_from: &StmtImportFrom,
      module_name: &str,
      symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
   ) -> Option<Vec<Stmt>>` (returns `Some(stmts)` if transformed, else `None`).
- Call site: replace the whole block with an early return using `if let Some(stmts) = ... { return stmts }`.
- Validation + Commit:
  - `cargo check && cargo nextest run`
  - `refactor(import_transformer): extract submodule bundled fast-path to handlers/inlined.rs`

Step 8 — Extract inlined module fast‑path (absolute non‑bundled)

- Source range: `import_transformer/mod.rs:2004–2023`.
- New function: `handlers::inlined::InlinedHandler::handle_imports_from_inlined_module_with_context` already exists; only lift the conditional into a helper:
  - `handlers::inlined::InlinedHandler::maybe_handle_inlined_absolute(
        bundler: &Bundler,
        import_from: &StmtImportFrom,
        module_name: &str,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        inside_wrapper_init: bool,
        current_module: &str,
     ) -> Option<Vec<Stmt>>`
- Call site: early return on `Some(stmts)`.
- Validation + Commit:
  - `cargo check && cargo nextest run`
  - `refactor(import_transformer): extract inlined absolute fast-path to handlers/inlined.rs`

Step 9 — Extract wrapper absolute fast‑path (non‑resolved branch)

- Source range: `import_transformer/mod.rs:2025–2041`.
- New function: `handlers::wrapper::WrapperHandler::maybe_handle_wrapper_absolute(
      bundler: &Bundler,
      import_from: &StmtImportFrom,
      module_name: &str,
      inside_wrapper_init: bool,
      at_module_level: bool,
      current_module: &str,
      symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
      function_body: Option<&[Stmt]>,
   ) -> Option<Vec<Stmt>>`
- Call site: early return on `Some(stmts)`.
- Validation + Commit:
  - `cargo check && cargo nextest run`
  - `refactor(import_transformer): extract wrapper absolute fast-path to handlers/wrapper.rs`

Step 10 — Move Bundler’s non‑wildcard wrapper from‑import logic into handler

- Source range: `code_generator/bundler.rs:656–1210` (`handle_symbol_imports_from_multiple`).
- New function (moved verbatim; receiver becomes `&Bundler` parameter):
  - `handlers::wrapper::handle_symbol_imports_from_multiple(
        bundler: &Bundler,
        import_from: &StmtImportFrom,
        module_name: &str,
        context: BundledImportContext<'_>,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        function_body: Option<&[Stmt]>,
     ) -> Vec<Stmt>`
- Call site updates:
  - `import_transformer/mod.rs::transform_wrapper_symbol_imports(..)` calls the moved function directly.
  - Remove the method from `impl Bundler` and its now‑unused helpers if any (only after all call sites are updated).
- Visibility: keep helper visibilities `pub(in crate::code_generator)` only where the moved function needs them.
- Validation + Commit:
  - `cargo check && cargo nextest run`
  - `refactor(wrapper): move non-wildcard from-import handler from Bundler to handlers/wrapper.rs`

Step 11 — Optional cleanup (post‑green)

- Remove any dead code created by the moves (unused imports, unused helper stubs) in the touched files only.
- If bottom sections of `rewrite_import_from` are fully extracted, consider splitting `rewrite_import_from` itself into a module (`handlers/rewrite.rs`) in a follow‑up PR.

Implementation Notes

- Keep logs, warnings, and panic messages identical.
- When copying code into handlers, switch from `self` to `bundler` explicitly and import the needed items at the top of the file.
- Maintain exact order of statements inside the moved blocks to preserve subtle init/merge behavior.
- Use the bottom‑up order above to prevent earlier line numbers from drifting between steps.

Test & Commit Checklist (per step)

- `cargo check`
- `cargo nextest run`
- Commit message from the step above.

Rollback Strategy

- If any step causes snapshot drift or runtime failures, revert that single commit and split the move into a smaller unit (e.g., extract only the `__main__` case first, then the panic case).
- Avoid partial logic rewrites; prefer moving smaller contiguous blocks verbatim.

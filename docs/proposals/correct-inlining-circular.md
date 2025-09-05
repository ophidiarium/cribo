# Correct Inlining and Circular Imports (Design + Implementation Spec)

Status: Implementation Resulted in Regressions, Abandoned

Owner: Cribo Team

Related fixtures: `mixed_import_patterns` (primary), `cross_module_attribute_import`, `cross_module_inheritance`, `ast_rewriting_globals_collision`, `alias_transformation_test`, `ast_rewriting_happy_path`

## 1. Background & Goals

The `mixed_import_patterns` fixture exposed a class of issues where a package’s modules import each other at mixed scopes (module-level and function-level) to avoid circular imports. Our bundler inlines some modules while emitting wrappers for others. The original approach failed to correctly:

- Pre-create wrapper namespaces before inlined code referenced them.
- Initialize wrapper modules in a sequence that respects strongly connected components (SCCs).
- Avoid name shadowing in function scope when rewriting `from X import Y`.
- Keep entry-module aliasing stable and non-duplicative without erasing needed rebinding semantics.

This document describes the design and implementation for correct inlining with circular imports, as well as the refinements made while fixing regressions surfaced by other fixtures.

## 2. Design Overview

Key principles:

- Classify modules into inlinable vs wrapper (side-effects/circulars) and honor Tarjan SCCs.
- Two-phase emission for SCCs: (A) predeclare wrapper namespaces; (B) define wrapper init functions, then merge.
- Pre-create any wrapper namespaces (and their init functions) that inlined modules directly or transitively depend on.
- Pass a safe "self" to wrapper init functions:
  - Inside wrapper init: pass the module variable (never `globals()`), because `globals()` can be rewritten to `__dict__` by the globals lifter.
  - In non-wrapper contexts: prefer the module variable for top-level modules; use `globals()["mod"]` only for function-scope aliasing where local shadowing must be avoided (e.g., `from X import Y` inside a function).
- Avoid duplicate init calls and alias statements; never dedupe init-result assignments.

## 3. Module Classification and Namespace Strategy

1. Classify modules via existing analyzer into inlinable vs wrapper. Wrapper modules receive a synthetic name and an init function.

2. Build SCC groups for wrapper modules and process in two phases:
   - Phase A: Create wrapper namespaces (SimpleNamespace stubs) and attach parent-child attributes (e.g., `pkg.sub` references).
   - Phase B: Emit wrapper init functions after all stubs exist.

3. Pre-create needed wrappers for inlined modules:
   - Scan inlined modules’ imports and build a set of required wrapper modules (including transitive dependencies). Pre-create their namespaces and init functions before inlining code that references them.

## 4. Wrapper Initialization Semantics (self argument)

Problem: Using `globals()["mod"]` as `self` inside wrapper init causes the globals lifter to rewrite to `module.__dict__`, which is not equivalent and resulted in KeyErrors (e.g., `'mypackage'`).

Solution:

- Inside wrapper init: always pass the module variable directly (no `globals()`), e.g., `mypackage.__init__(mypackage)`.
- Outside wrapper init and at function level (to avoid local shadowing when rewriting `from X import Y`): use `globals()["mod"].Y` for aliasing (entry and function scope), but never as the wrapper-init `self`.

Implementation points:

- `Bundler::create_module_initialization_for_import_with_current_module(module_id, current_module, inside_wrapper_init)` computes the correct `self` arg and emits the init call.
- `Bundler::transform_bundled_import_from_multiple_with_current_module` requests initialization with the correct `inside_wrapper_init` flag.

## 5. Import Rewrite Strategy (recursive transformer)

Goals:

- Normalize imports across scopes, while preventing function-scope name shadowing (e.g., later local assignment to `config` after `from config import Config`).
- Use module namespace variables for inlined modules.

Highlights:

- When inlining, `from pkg import submodule` becomes proper namespace-based references (e.g., `pkg_submodule` variable) and we avoid `globals()` unless necessary in function-scope aliasing.
- For wrapper modules, the recursive transformer defers to bundler to ensure the init function exists before accessing attributes.
- For inlined modules, alias assignments prefer namespace variable attribute access (stable, avoids symbol reordering issues).

## 6. Entry Module Handling

The entry module receives a post-pass that:

- Applies symbol renames to top-level definitions.
- Deduplicates import alias assignments that are truly redundant (same target and same source).
- Always keeps wrapper init result assignments (`__cribo_init_result = ...`) and emits the merge loops immediately after. We protect these assignments from deduplication to prevent attribute merges from using stale init results.
- Allows later alias to rebind the same name (matching Python semantics). For example, `Logger = core.utils.Logger` later overridden by `Logger = models.user.Logger` is allowed and expected.

## 7. Import Source Mapping for Cross-Module Renames (inliner)

To correctly rewrite cross-module inheritance and references, the inliner needs to know where a local symbol came from. We build this mapping from:

- `from X import Y [as Z]` statements, and
- alias assignments synthesized by the import transformer, e.g.,
  - `Name = globals()["mod"].Symbol`
  - `Name = sanitized_mod.Symbol`

This enables the inliner’s `rewrite_class_arg_expr` to:

- Prefer renames from the source module when rewriting base classes and keyword values, while preserving local alias names when appropriate.

## 8. Lessons Learned (Regressions & Fixes)

This effort surfaced valuable regressions that we addressed:

1. `cross_module_attribute_import`:
   - Issue: Using `globals()` as wrapper-init `self` got rewritten to `module.__dict__`, causing KeyError during circular setup.
   - Fix: Inside wrapper init, always pass the module variable directly.

2. `cross_module_inheritance`:
   - Issue: Duplicate alias lines and wrong target alias caused class bases to resolve to the wrong `HTTPBasicAuth` implementation.
   - Fix: Let legitimate later aliasing override earlier aliasing in entry (rebind), only dedupe exact duplicates; preserve local alias in class base rewrites.

3. `ast_rewriting_globals_collision`:
   - Issue: Attribute merges used a stale init result or collided due to dedup, wiping attributes (e.g., `services.auth.process`).
   - Fix: Protect init-result assignments from dedupe and emit each init immediately before its merge loop.

4. `alias_transformation_test` and `ast_rewriting_happy_path`:
   - Observation: More explicit alias statements surfaced in bundled output (e.g., helper function aliases). These are semantically correct and deterministic. Snapshot updates may be warranted.

5. General:
   - Avoid mixing `globals()` inside wrapper init contexts; prefer module variables.
   - Be careful when deduplicating alias assignments: only skip truly identical re-statements and never those for init results.

## 9. Implementation Details (Pointers)

Primary code paths (by file):

- `crates/cribo/src/code_generator/bundler.rs`
  - Pre-creation of wrapper namespaces and their init functions for inlined dependencies.
  - SCC (cycle) two-phase emission.
  - `create_module_initialization_for_import_with_current_module`: safe `self` argument and init emission.
  - Entry processing: alias dedupe, protected init-result assignments, merges.
  - `build_import_source_map`: tracks both `from X import` and alias assignments (`globals()["X"].Y`, `sanitized_X.Y`).

- `crates/cribo/src/code_generator/import_transformer.rs`
  - Wrapper-aware import initialization (passes `inside_wrapper_init`).
  - Namespace-based aliasing for inlined modules to avoid unstable globals use where not needed.

- `crates/cribo/src/code_generator/inliner.rs`
  - Cross-module base class rewrite via `rewrite_class_arg_expr`, preserving local alias when needed.
  - Recognizes alias assignments generated by the transformer as import sources.
  - Side-effect free inliner’s loop handling now robust against init-result var naming.

## 10. Correctness & Determinism

- Deterministic ordering: SCC processing logs and emits in a consistent order; merges anchored immediately after init calls.
- No duplicate init calls: local dedupe within a transform scope.
- No accidental symbol overwrite in `__all__` or via merges: merges rely on fresh init result per operation and skip undesired targets.

## 11. Risk & Edge Cases

- Deep alias chains: Resolved by capturing import sources from both `from` statements and synthesized alias assignments.
- Over-aggressive dedupe: Limited to exact duplicate alias assignments; init results are never deduped.
- Globals lifter side-effects in wrapper init: Avoided by using module variables internally.

## 12. Testing & Rollout

- Fixtures used to drive the changes:
  - Primary: `mixed_import_patterns` (circular between config/logger with mixed-scope imports)
  - Guardrails: `cross_module_attribute_import`, `cross_module_inheritance`, `ast_rewriting_globals_collision`, `alias_transformation_test`, `ast_rewriting_happy_path`

- Action:
  - Run `cargo test --test test_bundling_snapshots` with selective `INSTA_GLOB_FILTER` for rapid iteration, then full.
  - Review and accept snapshots where behavior is semantically equivalent but output is now explicit/stable: `cargo insta review`.

## 13. Examples (Pseudocode)

Two‑phase SCC emission:

```text
for group in SCCs:
  # Phase A: predeclare
  for mod in group:
    create_namespace(mod)
    attach_parent_chain(mod)
  # Phase B: define init
  for mod in group:
    define_init_function(mod)
```

Wrapper init (safe self):

```text
if inside_wrapper_init or mod has dot:
  self_arg = Name(sanitized_mod)
else:
  # non-wrapper function-scope aliasing may still use globals()[mod] for symbol fetches
  self_arg = Name(sanitized_mod)
emit: sanitized_mod = _cribo_init__... (self_arg)
```

Entry alias dedupe (simplified):

```text
if Assign(target=Name, value=Attribute(base, attr)):
  if is_init_result_target(Name): keep
  elif identical_alias_already_seen(target, base, attr): skip
  else: keep (allow later rebinds)
```

## 14. Conclusion

The updated bundling pipeline correctly supports mixed-scope imports with circular dependencies, preserves runtime semantics across fixtures, and produces more stable output. The design focuses on correctness first (wrapper pre-creation, safe init calls, robust merges), then minimizes diffs through targeted dedupe. Snapshot updates should be reviewed and accepted where output is now more explicit but semantically equivalent.

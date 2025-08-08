# Final Proposal: Centralized Namespace Management via Pre-Discovery and Deferred Population

**Author**: Gemini
**Date**: 2025-08-08
**Status**: Final

## 1. Overview

This document provides the definitive technical specification for refactoring Cribo's namespace management. The current implementation distributes the creation of `types.SimpleNamespace` AST nodes across multiple modules, leading to code duplication, high complexity, and regressions.

This proposal outlines a robust, multi-phase architecture that is both technologically correct and respects Rust's borrowing constraints:

1. **Pre-Discovery**: A read-only pass identifies all required namespaces upfront.
2. **Centralized Empty Generation**: All required namespaces are created as empty objects at the top of the bundle.
3. **Deferred Population**: Namespaces are populated with their symbols via attribute assignments immediately after the corresponding module's code has been inlined and its symbols defined.

This approach centralizes namespace creation, ensures correct parent-child ordering, and allows for the safe, incremental removal of the legacy `deduplicate_namespace_creation_statements` function.

## 2. Problem Analysis

The core issue is uncoordinated, direct AST generation. Multiple transformers (`import_transformer`, `module_transformer`) create `types.SimpleNamespace` objects independently. This decentralized approach is fragile and produces duplicate AST nodes, previously masked by a post-processing deduplication step.

Crucially, two fundamental constraints were identified during implementation analysis:

1. **Borrowing Constraints**: The `RecursiveImportTransformer` is invoked from contexts (in `inliner.rs` and `module_transformer.rs`) where the `Bundler` is already immutably borrowed, making it impossible for the transformer to mutate a central collection of namespace requirements.
2. **Forward References**: It is impossible to populate a namespace with its symbols at creation time. The namespace objects are created at the top of the bundled file, but the symbols they need to contain are defined much later in the code. Referencing these symbols before they are defined would result in a `NameError` at runtime.

The existing `populate_namespace_with_module_symbols` function is a remnant of a previous attempt to solve this. It is currently called reactively from deep within the `import_transformer.rs` module, which is a fragile and incomplete solution.

Therefore, the only correct approach is to discover requirements upfront, create empty namespaces first, and then centrally manage the deferred population after symbols are defined.

## 3. Proposed Architecture

The architecture is a three-phase process integrated into the main bundling workflow.

### Phase 1: Pre-Discovery (Read-Only)

A new, lightweight `NamespaceDiscoverer` will perform a read-only traversal of all module ASTs *before* any transformations begin. It will identify all import patterns that necessitate the creation of a namespace and will not perform any code transformation.

### Phase 2: Centralized Empty Namespace Generation

The `Bundler` will process the requirements collected during the discovery phase. It will resolve any context conflicts and then, at the very beginning of the bundling process, call a centralized generation function. This function will create all required namespaces as **empty** `types.SimpleNamespace` objects, ensuring they are available in the global scope for the rest of the bundled code.

### Phase 3: Inlining with Deferred Population

This is the crucial phase that correctly handles symbol population.

- The `inliner` remains responsible for processing a module's AST and adding its statements to the final bundle.
- **Immediately after** inlining a module's code, the `inliner` will call a new, centralized `namespace_manager` function.
- This new function will generate the necessary `Stmt::Assign` nodes to populate the now-existing empty namespace with the symbols from the module that was just inlined (e.g., `pkg_utils.my_func = pkg_utils_my_func`).
- These population statements are appended to the bundle right after the symbol definitions, guaranteeing correctness.

### 3.1. Data Structures

#### 3.1.1. `NamespaceRequirement` Struct

This struct will represent the need for a namespace's existence.

```rust
// To be defined in `crates/cribo/src/code_generator/namespace_manager.rs`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NamespaceRequirement {
    /// The original, fully-qualified module path (e.g., "pkg.compat").
    pub path: String,
    /// The context in which the namespace is required.
    pub context: NamespaceContext,
    /// The local alias for the namespace, if one exists (e.g., `compat` for `pkg.compat`).
    pub alias: Option<String>,
}
```

#### 3.1.2. `NamespaceInfo` Struct

The `deferred_symbols` field will be removed and replaced with a simple flag.

```rust
// in crates/cribo/src/code_generator/namespace_manager.rs
pub struct NamespaceInfo {
    // ... existing fields ...
    // pub deferred_symbols: Vec<(String, Expr)>, // This will be removed
    /// A flag to indicate that this namespace should be populated from an inlined module.
    pub needs_population: bool,
}
```

## 4. Detailed Implementation Plan

### Phase 1: Implement Namespace Discovery

1. **Create `namespace_discovery.rs`**:
   - Define a new struct `NamespaceDiscoverer<'a>`. It will hold immutable references to `Bundler` fields required for discovery (e.g., `resolver`, `inlined_modules`).
   - Implement `pub fn discover(ast: &ModModule, module_path: &Path) -> Vec<NamespaceRequirement>`. This method will use the `ruff_python_ast::visitor::Visitor` trait to walk the AST, identify import statements that require namespaces, and return a vector of `NamespaceRequirement` structs.

### Phase 2: Integrate Discovery and Empty Generation

This phase wires the new discovery process into the main bundling workflow.

1. **Update `bundler.rs` Workflow**:
   - In the `bundle_modules` function, immediately after the `prepare_modules` step, add a new **Discovery Stage**:
     - Iterate over all modules (inlinable and wrapper).
     - For each module, instantiate `NamespaceDiscoverer` and call its `discover` method.
     - Collect all returned requirements into `self.collected_namespace_requirements`.
   - Implement a new private method: `bundler.process_collected_requirements()`. This function will deduplicate the collected requirements by path (respecting context priority) and populate the `bundler.namespace_registry`.
   - Call `self.process_collected_requirements()` and then `namespace_manager::generate_required_namespaces(self)` at the top of `bundle_modules` to create all necessary empty namespaces upfront.

2. **Modify `namespace_manager.rs`**:
   - Update the `generate_required_namespaces()` function to ensure it only generates **empty** namespaces (e.g., `pkg_utils = types.SimpleNamespace(__name__='pkg.utils')`), as population is now handled later.

### Phase 3: Implement Deferred Population and Remove Old Call Sites

This phase implements the new deferred population mechanism and atomically removes the old, decentralized calls.

1. **Adapt Existing `namespace_manager.populate_namespace_with_module_symbols()`**:
   - Review and adapt the existing `populate_namespace_with_module_symbols` function in `namespace_manager.rs` to ensure it works correctly when called from the inliner. Its signature is likely sufficient, but its internal logic may need minor adjustments for the new, centralized workflow.

2. **Update `inliner.rs` to Call Population Function**:
   - In the `inline_module` function, after a module's statements have been processed and added to `ctx.inlined_stmts`, add a call to the `namespace_manager::populate_namespace_with_module_symbols()` function.
   - Extend `ctx.inlined_stmts` with the returned population statements. This ensures population happens immediately after symbol definition.

3. **Atomically Remove Old Call Sites**:
   - In `import_transformer.rs`, delete the now-redundant calls to `populate_namespace_with_module_symbols`. This is done in the same phase to prevent a non-compilable state.
   - Remove all other namespace creation logic from `RecursiveImportTransformer` and `module_transformer.rs`. The transformers will now operate on the assumption that namespace variables are created by the bundler and populated by the inliner.

# Phase 4: Validation

Make sure to run the full snapshot test suite (`cargo test --workspace`) after each phase to ensure that the new system behaves as expected and that no regressions are introduced.

### Phase 5: Final Cleanup

With the new system fully in place, this phase removes the last pieces of legacy code.

1. **Remove Redundant State**:
   - Delete the `created_namespace_objects` flag from `RecursiveImportTransformer` as it is no longer needed. The `types` import is now handled centrally by the `Bundler`.

### Phase 6: Incremental Removal of `deduplicate_namespace_creation_statements`

This phase ensures a safe, verifiable removal of the legacy function.
MANDATORY: run whole test suite after each step to ensure correctness!

1. **Step 6.1: Remove Alias Tracking Logic**
   - **Action**: In `deduplicate_namespace_creation_statements`, remove the code block that handles alias assignments (`alias = var`).
   - **Justification**: The new pre-discovery phase correctly identifies alias requirements, and the centralized generator creates them. This logic is now redundant.
   - **Validation**: Run the full snapshot test suite (`cargo test --workspace`). All tests must pass.

2. **Step 6.2: Remove Attribute Assignment Warning**
   - **Action**: Remove the logic that checks for `parent.child` assignments and logs a warning if the parent namespace hasn't been created.
   - **Justification**: The new system's depth-based sorting of namespace creation guarantees that parent namespaces are always created before their children. This check is obsolete.
   - **Validation**: Run the full snapshot test suite.

3. **Step 6.3: Remove Core `SimpleNamespace` Deduplication**
   - **Action**: Remove the primary `if/else` block that checks if a namespace is already in the `created_namespaces` set.
   - **Justification**: The pre-discovery and centralized generation phases guarantee that exactly one creation statement is generated for each required namespace. There will be no duplicates to remove.
   - **Validation**: Run the full snapshot test suite.

4. **Step 6.4: Final Removal**
   - **Action**: The function should now be an empty shell that passes its input through. Remove the function definition entirely and delete the call to it in `bundle_modules`.
   - **Justification**: The function no longer serves any purpose.
   - **Validation**: Run the full snapshot test suite.

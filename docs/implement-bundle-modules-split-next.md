# Implementation Plan: Phased and Modular Refactoring of `bundle_modules`

## 1. Executive Summary

This document provides a detailed, step-by-step plan to refactor the monolithic `bundle_modules` function in `crates/cribo/src/code_generator/bundler.rs`. This plan improves upon the previous version by not only breaking down the function but also relocating logic to more appropriate modules, enhancing the overall project architecture.

The current `bundle_modules` function is approximately 900 lines long (lines 1458-2359). This refactoring will make the code more modular, maintainable, and easier to debug.

## 2. Goal

To refactor `bundle_modules` into smaller, focused functions, and to move self-contained logic into other, more suitable modules within the `code_generator`. The final `bundle_modules` function will serve as a high-level orchestrator.

## 3. Phased Implementation Plan

Follow these phases sequentially. After each phase, **run all workspace tests (`cargo test --workspace`)** to ensure no regressions have been introduced before committing.

---

### **Phase 1: Initialization and Preparation (in `bundler.rs`)**

**Goal:** Extract the initial setup logic into private methods within `bundler.rs`, as this logic is tightly coupled to the `Bundler`'s state.

**Step 1.1: Create `initialize_bundler`**

- **Action:** Move lines `1461-1538` into a new private method `fn initialize_bundler(&mut self, params: &BundleParams<'_>)`.
- **Benefit:** Isolates the setup of the bundler's initial state (tree-shaking, entry point info, etc.).

**Step 1.2: Create `prepare_modules`**

- **Action:** Move lines `1541-1621` into a new private method `fn prepare_modules(...) -> Result<Vec<...>>`. This includes the `needs_types_for_entry_imports` check.
- **Benefit:** Encapsulates the trimming of unused imports and the indexing of module ASTs, which are preparatory steps for code generation.

**Step 1.3: Test and Commit**

- Run `cargo test --workspace`.
- Commit with message: `refactor(bundler): extract initialize_bundler and prepare_modules`.

---

### **Phase 2: Module Classification (in `bundler.rs`)**

**Goal:** Extract the complex logic of classifying modules into a single, focused method. This remains in `bundler.rs` as it's a core strategic decision-making part of the bundling process.

**Step 2.1: Create `classify_modules`**

- **Action:**
  - Create a new struct `ClassificationResult { ... }`.
  - Move lines `1639-1811` into a new private method `fn classify_modules(...) -> ClassificationResult`.
- **Benefit:** Creates a clear separation between analyzing module characteristics and the main bundling flow. It produces a structured result that describes how to handle each module.

**Step 2.2: Test and Commit**

- Run `cargo test --workspace`.
- Commit with message: `refactor(bundler): extract classify_modules method`.

---

### **Phase 3: Semantic and Circular Dependency Analysis**

**Goal:** Isolate semantic analysis and move the complex circular dependency logic to its own dedicated module.

**Step 3.1: Create `collect_symbol_renames` (in `bundler.rs`)**

- **Action:** Move lines `1960-1970` into a new private method `fn collect_symbol_renames(...) -> FxIndexMap<...>`.
- **Benefit:** Encapsulates the collection of symbol renames.
- **Note on Location:** This stays in `bundler.rs` because its logic depends on code-generation decisions (i.e., which modules are inlined), preventing a clean move to the `analyzers` crate.

**Step 3.2: Move Circular Pre-declaration Logic (to `circular_deps.rs`)**

- **Action:**
  - Create a new `pub(super)` function in `crates/cribo/src/code_generator/circular_deps.rs` named `generate_predeclarations`.
  - The function signature should be `pub(super) fn generate_predeclarations(bundler: &mut Bundler, ... ) -> Result<Vec<Stmt>>`.
  - Move the logic from lines `1984-2109` of `bundler.rs` into this new function.
  - Update `code_generator/mod.rs` if necessary to ensure visibility.
  - In `bundler.rs`, replace the old logic with a call to `circular_deps::generate_predeclarations(self, ...)?`.
- **Benefit:** This is the key modular improvement. It moves a large, complex, and self-contained piece of logic into the module that is already responsible for circular dependency data structures (`SymbolDependencyGraph`), significantly cleaning up `bundler.rs`.

**Step 3.3: Test and Commit**

- Run `cargo test --workspace`.
- Commit with message: `refactor: extract symbol rename logic and move circular dependency handling`.

---

### **Phase 4: Finalizing the `bundle_modules` Orchestrator**

After the above phases, the `bundle_modules` function will be dramatically smaller and cleaner. Its role will be to orchestrate the calls to the new private methods and the functions in other modules. The final structure will clearly show the major phases of the bundling process:

```rust
// in bundler.rs
pub fn bundle_modules(&mut self, params: &BundleParams<'_>) -> Result<ModModule> {
    // 1. Initialization
    self.initialize_bundler(params);
    let mut modules = self.prepare_modules(params)?;

    // 2. Classification
    // ... setup before classification ...
    let classification = self.classify_modules(&modules, params.entry_module_name);
    // ... logic to handle classification results ...

    // 3. Semantic Analysis & Circular Deps
    let semantic_ctx = /* ... */;
    let mut symbol_renames = self.collect_symbol_renames(&modules, &semantic_ctx);
    let predeclarations = circular_deps::generate_predeclarations(
        self,
        &modules,
        &classification.inlinable_modules,
        &symbol_renames,
        params,
    )?;

    // 4. Code Generation
    let mut final_body = Vec::new();
    final_body.extend(predeclarations);
    // ... (Calls to namespace_manager, module_transformer, inliner) ...

    // 5. Finalization
    self.finalize_module(final_body, params)
}
```

This updated plan provides a more robust and architecturally sound path for the refactoring, directly addressing the concern about modularity.

# Centralized Namespace Management Specification

**Author**: Gemini
**Date**: 2025-08-07
**Status**: Final

## 1. Overview

This document specifies a plan to refactor the creation and management of `types.SimpleNamespace` AST nodes within the Cribo bundler. The current implementation distributes this logic across multiple modules, necessitating a `deduplicate_namespace_creation_statements` function and making the system difficult to maintain.

This plan addresses these issues by evolving the existing `namespace_registry` infrastructure into a centralized authority for the entire namespace lifecycle. This refined approach will handle creation, deferred population, aliasing, and complex edge cases like circular dependencies, ultimately leading to a more robust and maintainable codebase.

## 2. Problem Statement

The logic for creating `types.SimpleNamespace` objects is currently scattered across:

- `code_generator::bundler`
- `code_generator::import_transformer`
- `code_generator::module_transformer`
- `code_generator::namespace_manager`

This fragmentation leads to code duplication, high complexity, and fragile workarounds like `deduplicate_namespace_creation_statements`. A centralized system is required to manage the complex graph of module relationships, handle circular dependencies, and ensure symbols are available when needed.

## 3. Proposed Solution: A Centralized Namespace Authority

We will enhance the existing `namespace_registry: FxIndexMap<String, NamespaceInfo>` within the `Bundler` to serve as the single source of truth for namespace management.

### 3.1. Data Structures

#### 3.1.1. Enhanced `NamespaceInfo` Struct

The `NamespaceInfo` struct in `bundler.rs` will be extended to track the full state of each namespace.

```rust
// in crates/cribo/src/code_generator/bundler.rs

#[derive(Debug, Clone)]
pub struct NamespaceInfo {
    // --- Existing Fields (Retained for metadata) ---
    pub original_path: String,
    pub needs_alias: bool,
    pub alias_name: Option<String>,
    pub attributes: Vec<(String, String)>,
    pub parent_module: Option<String>,

    // --- New/Updated Fields ---
    /// Tracks if the `var = types.SimpleNamespace()` statement has been generated.
    pub is_created: bool,
    /// The context in which this namespace was required, with priority.
    pub context: NamespaceContext,
    /// Symbols that need to be assigned to this namespace after its creation.
    pub deferred_symbols: Vec<(String, Expr)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamespaceContext {
    TopLevel,
    Attribute { parent: String },
    InlinedModule,
    CircularDependencyWrapper,
}

impl NamespaceContext {
    // Defines the priority for overriding contexts. Higher value wins.
    fn priority(&self) -> u8 {
        match self {
            Self::TopLevel => 0,
            Self::Attribute { .. } => 1,
            Self::InlinedModule => 2,
            Self::CircularDependencyWrapper => 3,
        }
    }
}
```

#### 3.1.2. Sanitization Mapping in `Bundler`

To handle the mapping between original module paths and sanitized identifiers, the `Bundler` will maintain a reverse lookup map.

```rust
// in crates/cribo/src/code_generator/bundler.rs

pub struct Bundler<'a> {
    // ...
    // Primary registry: Maps SANITIZED name to NamespaceInfo
    pub(crate) namespace_registry: FxIndexMap<String, NamespaceInfo>,
    // Reverse lookup: Maps ORIGINAL path to SANITIZED name
    pub(crate) path_to_sanitized_name: FxIndexMap<String, String>,
    // ...
}
```

### 3.2. Centralized API in `Bundler`

The `Bundler` will expose a clear, two-phase API for namespace management.

#### Phase 1: Registration

The `require_namespace` method will be the sole entry point for requesting a namespace.

```rust
// in crates/cribo/src/code_generator/bundler.rs

impl<'a> Bundler<'a> {
    /// Registers a request for a namespace, creating or updating its info.
    /// This is the ONLY function that should be called to request a namespace.
    /// It is idempotent and handles parent registration recursively.
    pub fn require_namespace(&mut self, path: &str, context: NamespaceContext) {
        // 1. Recursively require parent namespaces if `path` is dotted.
        // 2. Get or create the sanitized name for `path`, handling potential collisions.
        // 3. Use the sanitized name to look up the NamespaceInfo in `self.namespace_registry`.
        // 4. If it doesn't exist, create a new `NamespaceInfo`.
        // 5. If it exists, update its context only if the new context has a higher priority.
    }
}
```

#### Phase 2: Generation

The `generate_required_namespaces` method will be called once to generate all necessary AST statements in the correct order.

```rust
// in crates/cribo/src/code_generator/bundler.rs

impl<'a> Bundler<'a> {
    /// Generates all required namespace creation and population statements.
    /// This function guarantees correct, dependency-aware ordering.
    pub fn generate_required_namespaces(&mut self) -> Vec<Stmt> {
        let mut statements = Vec::new();

        // 1. Get all keys (sanitized names) from `self.namespace_registry`.
        // 2. Create a list of (sanitized_name, original_path) tuples.
        // 3. Sort this list based on the depth of the original_path (number of '.'). This ensures
        //    parent namespaces are created before their children.
        // 4. Iterate through the sorted list.
        // 5. For each namespace, if `is_created` is false: a. Generate the `sanitized_name =
        //    types.SimpleNamespace()` statement. b. Add it to the `statements` vector. c. Mark
        //    `is_created = true` in its `NamespaceInfo`. d. Generate and add any deferred symbol
        //    population statements. e. Generate and add any required alias statements (`alias =
        //    sanitized_name`).

        statements
    }
}
```

## 4. Refactoring and Implementation Plan

1. **Phase 1: Extend Infrastructure**
   - Update the `NamespaceInfo` and `Bundler` structs as defined above.
   - Implement the `require_namespace` and `generate_required_namespaces` methods, including the depth-based sorting logic.
   - Define the `NamespaceContext` enum with its priority logic.

2. **Phase 2: Gradual Migration**
   - Replace each direct creation of `types.SimpleNamespace` with a call to `bundler.require_namespace()`. This will be done one module at a time, with full test runs after each migration to ensure stability.
   - The mutable borrow of `bundler` will be managed by passing `&mut self` down the call stack where needed. In cases where this is difficult, we can collect required namespaces into a temporary `Vec` and register them in a batch once the immutable borrow is released.
   - **Key Migration Targets**:
     - **`code_generator::namespace_manager`**: Refactor `create_namespace_for_inlined_module_static`, `create_namespace_with_name`, and `create_namespace_attribute` to use the new registry.
     - **`code_generator::import_transformer`**: Modify `RecursiveImportTransformer` to accept a mutable reference to the `Bundler` and call `require_namespace` instead of creating namespaces directly.
     - **`code_generator::module_transformer`**: Update `transform_module_to_init_function` to use the registry for creating namespaces for inlined submodules.
     - **`code_generator::bundler`**: Refactor `bundle_modules` and other internal methods that currently create namespaces ad-hoc.

3. **Phase 3: Centralize Generation**
   - In `bundler.rs`, replace scattered namespace creation logic with a single, well-placed call to `self.generate_required_namespaces()`.

4. **Phase 4: Cleanup and Validation**
   - After verifying that the new system correctly handles all cases, remove the `deduplicate_namespace_creation_statements` function.
   - Remove the now-redundant fields from `Bundler` (`required_namespaces`, `created_namespaces`).

## 5. Handling Key Concerns

This design addresses the specific concerns raised during the review:

- **Sanitization and Lookups**: The `path_to_sanitized_name` map provides a clear and efficient way to manage the relationship between original paths and their sanitized identifiers.
- **Ordering**: The generation phase will use depth-based sorting of original paths, which is a robust method for ensuring parent namespaces are created before their children.
- **Mutable Borrows**: The migration will be handled carefully, passing mutable references where possible and using temporary collections to batch registrations where necessary.
- **Context Priority**: The `NamespaceContext::priority()` method provides explicit rules for resolving context conflicts.
- **Symbol Population Timing**: The `deferred_symbols` field allows for a clean separation between namespace creation and population. The generation logic will ensure population happens after creation, and the `Expr` for a symbol's value will be captured at a point when it is valid.
- **Testing**: During the migration, a debug assertion can be added to compare the output of the old and new systems, ensuring that the same set of namespaces is created. Snapshot tests will be updated incrementally with each migrated module.

## 6. Conclusion

By evolving the existing `namespace_registry`, we can build a centralized, robust, and maintainable system for namespace management. This approach respects the existing architecture's complexity while achieving the goals of code simplification and the removal of brittle workarounds. The phased implementation plan provides a clear path forward with minimal risk.

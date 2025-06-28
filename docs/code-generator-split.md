# Technical Proposal: Refactoring `code_generator.rs`

## 1. Motivation

The `crates/cribo/src/code_generator.rs` file has grown significantly and currently exceeds 14,000 lines of code. Its large size makes it difficult to navigate, understand, and maintain. The file contains multiple distinct responsibilities, including:

- The main `HybridStaticBundler` struct and its implementation.
- Several helper and context-related structs.
- Complex AST transformers like `RecursiveImportTransformer` and `GlobalsLifter`.
- Logic for handling circular dependencies at the symbol level (`SymbolDependencyGraph`).
- Various utility functions for AST manipulation.

To improve code organization, readability, and maintainability, this proposal outlines a plan to split `code_generator.rs` into smaller, more focused modules.

## 2. Proposed Structure

The current `code_generator.rs` file will be replaced by a `code_generator` module with the following structure:

```
crates/cribo/src/
└── code_generator/
    ├── mod.rs                 # Main module file, defines HybridStaticBundler
    ├── bundler.rs             # Core implementation of HybridStaticBundler
    ├── circular_deps.rs       # SymbolDependencyGraph for circular dependency logic
    ├── context.rs             # Context/parameter structs for bundling operations
    ├── globals.rs             # GlobalsLifter and related utility functions
    └── import_transformer.rs  # RecursiveImportTransformer implementation
```

### File Responsibilities:

- **`mod.rs`**:
  - Declares the other files as sub-modules.
  - Contains the definition of the main `HybridStaticBundler` struct.
  - Will re-export necessary components for other parts of the crate.

- **`bundler.rs`**:
  - Contains the `impl HybridStaticBundler` block.
  - Houses the primary methods for bundling, such as `bundle_modules`.
  - May delegate to helpers in other files for specific tasks.

- **`circular_deps.rs`**:
  - Contains the `SymbolDependencyGraph` struct and its implementation. This isolates the logic for handling symbol-level circular dependencies.

- **`context.rs`**:
  - A central place for all the small helper structs used as parameters or context for various methods. This includes:
    - `HardDependency`
    - `ModuleTransformContext`
    - `InlineContext`
    - `SemanticContext`
    - `ProcessGlobalsParams`
    - `DirectImportContext`
    - `TransformFunctionParams`
    - `BundleParams`
    - `RecursiveImportTransformerParams`

- **`globals.rs`**:
  - Contains the `GlobalsLifter` struct and its implementation.
  - Includes the `transform_globals_in_expr` and `transform_globals_in_stmt` utility functions.

- **`import_transformer.rs`**:
  - Contains the `RecursiveImportTransformer` struct and its implementation. This isolates the complex logic of rewriting imports within the AST.

## 3. Refactoring Steps

The refactoring process will be as follows:

1. **Create New Directory and Files**:
   - Create the directory `crates/cribo/src/code_generator/`.
   - Create the empty files: `mod.rs`, `bundler.rs`, `circular_deps.rs`, `context.rs`, `globals.rs`, and `import_transformer.rs`.

2. **Migrate `SymbolDependencyGraph`**:
   - Move the `SymbolDependencyGraph` struct and its `impl` block from `code_generator.rs` to `code_generator/circular_deps.rs`.

3. **Migrate `GlobalsLifter` and Utilities**:
   - Move the `GlobalsLifter` struct and its `impl` block to `code_generator/globals.rs`.
   - Move the `transform_globals_in_expr` and `transform_globals_in_stmt` functions to the same file.

4. **Migrate `RecursiveImportTransformer`**:
   - Move the `RecursiveImportTransformer` struct and its `impl` block to `code_generator/import_transformer.rs`.

5. **Migrate Context Structs**:
   - Move all the small context and parameter structs to `code_generator/context.rs`.

6. **Set Up `mod.rs` and `bundler.rs`**:
   - In `code_generator/mod.rs`, declare the new modules (`pub mod bundler;`, etc.).
   - Move the `HybridStaticBundler` struct definition to `code_generator/mod.rs`.
   - Move the `impl HybridStaticBundler` block and its methods to `code_generator/bundler.rs`.

7. **Update `use` Statements**:
   - Go through each new file and add the necessary `use` statements to resolve paths to types now in other files (e.g., `use super::context::BundleParams;`).
   - Update `crates/cribo/src/lib.rs` and other files that use `code_generator` to point to the new module structure. The public interface should remain unchanged.

8. **Final Cleanup**:
   - Once all code has been moved and all compilation errors are resolved, delete the original `crates/cribo/src/code_generator.rs` file.
   - Rename `crates/cribo/src/code_generator/mod.rs` to `crates/cribo/src/code_generator.rs` to make it the module root, or adjust `lib.rs` to use `mod code_generator;`. The former is often cleaner.

This refactoring will result in a more organized and maintainable `code_generator` module, with each file having a clear and focused responsibility, and all files being well under the desired token limit.

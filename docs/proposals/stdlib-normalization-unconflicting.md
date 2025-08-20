# System Design: Semantically-Aware Stdlib Normalization

## 1. Overview

This document outlines the final, approved strategy for handling standard library (stdlib) imports in Cribo. The primary goal is to resolve name conflicts where local variables or function arguments shadow stdlib module names, ensuring correct symbol resolution in all cases.

This design is grounded in a detailed analysis of the existing codebase. It leverages established architectural patterns, particularly the `RecursiveImportTransformer` and the `namespace_manager`, to deliver a robust solution with minimal new complexity. The core principle is to **defer all stdlib normalization to the code generation phase**, where a complete `SemanticModel` is available to accurately distinguish between user-defined symbols and stdlib module references.

## 2. The Core Problem

The current architecture performs semantic analysis on the original, unmodified Abstract Syntax Tree (AST), which is correct. However, a subsequent `stdlib_normalization` step operates *after* this analysis but *before* the final code generation, and it does so without semantic context. It blindly rewrites any name that matches a known stdlib alias.

This leads to bugs in common Python patterns where a local symbol shadows a stdlib module, such as this example from the project's test suite (`crates/cribo/tests/fixtures/stdlib_shadowing/main.py`):

```python
import os


def get_files(os):  # The 'os' parameter shadows the 'os' module.
    return os.listdir(".")  # This line would be incorrectly rewritten.


# This call should be correctly rewritten to use the hoisted stdlib module.
print(os.path.join("a", "b"))
```

The existing normalization logic cannot distinguish the `os` parameter from the `os` module, leading to incorrect code generation.

## 3. Proposed Architecture: A Minimal, Semantically-Aware Approach

The solution is to integrate stdlib normalization directly into the existing code generation machinery, making it fully context-aware.

### 3.1. High-Level Pipeline

1. **Analysis Phase (Simplified)**:
   - **Parse Module**: The source is parsed into an AST.
   - **Semantic Analysis**: `SemanticBundler` runs on the raw AST, building an accurate `SemanticModel` for each module.
   - **Remove Premature Normalization**: The premature and context-free `stdlib_normalization::normalize_stdlib_imports` function call in `orchestrator.rs` will be **completely removed**. The AST will remain pristine throughout the analysis phase.

2. **Code Generation Phase (Enhanced)**:
   - **Centralized Transformation**: All stdlib import processing will be consolidated within the `code_generator::import_transformer::RecursiveImportTransformer`. This component is the designated place for all complex import and symbol rewriting.
   - **Semantic Context**: The `RecursiveImportTransformer` will be provided with a reference to the complete `SemanticBundler` instance. This gives it access to the `SemanticModel` for any module it needs to inspect, allowing for precise, context-aware decisions.
   - **Hoisting via Namespace Manager**: All identified stdlib modules will be registered with the existing `code_generator::namespace_manager`. This central registry will handle the creation of a conflict-free `_cribo` namespace, ensuring all stdlib modules are imported and assigned correctly at the top of the bundle.

### 3.2. Mechanism for Dynamic and Static Stdlib Imports

The new architecture will unify the handling of both stdlib imports found in user code and those dynamically required by the bundler itself (e.g., `import types` for namespaces).

1. **Centralized Request API on `Bundler`**: A new public method on `code_generator::bundler::Bundler` will serve as the single entry point for requesting a stdlib module.
   ```rust
   // In Bundler
   pub fn require_stdlib_module(&mut self, module_name: &str) {
       self.namespace_manager.require_namespace(
           module_name.to_string(),
           NamespaceContext::StdlibModule,
           NamespaceParams::immediate(), // Ensures it's created
       );
   }
   ```

2. **Centralized Path Rewriting API**: A corresponding helper will provide the correct, rewritten path for use in generated code.
   ```rust
   // In Bundler or a shared utility module
   pub fn get_rewritten_stdlib_path(path: &str) -> String {
       format!("_cribo.{}", path)
   }
   ```

This unified mechanism ensures that whether an import comes from user code or is injected by Cribo, it is handled identically, guaranteeing consistency and preventing conflicts.

### 3.3. Detailed Implementation Plan

#### Phase 1: Decouple and Remove Old Logic

1. **Modify `orchestrator.rs`**:
   - In the `process_module` function, the entire block responsible for stdlib normalization will be deleted.
   - The `ProcessedModule` struct will be simplified, removing the now-obsolete `normalized_imports` and `normalized_modules` fields.

2. **Delete `stdlib_normalization.rs`**:
   - The file `crates/cribo/src/stdlib_normalization.rs` will be removed from the project.

#### Phase 2: Enhance the `RecursiveImportTransformer`

1. **Provide Semantic Context**:
   - The `RecursiveImportTransformerParams` struct will be updated to include a reference to the `SemanticBundler`.
   - The `Bundler` will pass its `SemanticBundler` instance when creating the transformer.

2. **Implement Stdlib Import Handling**:
   - The `transform_statement` method will be enhanced to detect stdlib imports.
   - When a stdlib import is found, it will call `bundler.require_stdlib_module()` for each required module.
   - It will then build a local "rename map" for the current scope (e.g., `from json import dumps as json_dumps` maps `"json_dumps"` to `"_cribo.json.dumps"`).
   - Finally, it will mark the original import statement for removal.

3. **Implement Semantically-Aware Rewriting**:
   - The `transform_expr` method will use the `SemanticModel` to resolve any symbol it encounters.
   - By inspecting the symbol's `BindingKind`, it will differentiate between stdlib module imports and locally-defined variables (like parameters or assignments).
   - Only symbols that resolve to a stdlib module import will be rewritten using the rename map.

#### Phase 3: Leverage the `NamespaceManager`

1. **Add New Namespace Context**:
   - A new variant will be added to `code_generator::namespace_manager::NamespaceContext`:
     ```rust
     pub enum NamespaceContext {
         // ... existing variants
         StdlibModule,
     }
     ```

2. **Generate Hoisted Imports**:
   - No changes are needed here. The `Bundler`'s existing call to `namespace_manager.generate_required_namespaces()` will automatically handle the generation of the `_cribo` namespace and all necessary `import os as _cribo_os` statements, triggered by the `require_stdlib_module` calls.

## 4. Test Strategy

The existing snapshot testing framework at `crates/cribo/tests/test_bundling_snapshots.rs` will be used for all validation.

### New Test Fixtures

The following fixtures will be added to `crates/cribo/tests/fixtures/` to ensure correctness and prevent regressions.

1. **`stdlib_shadowing_arg/main.py`**:
   - **Purpose**: Validates that a function argument shadowing a stdlib module is handled correctly.
   - **Content**:
     ```python
     import json


     def process_data(json):
         if isinstance(json, str):
             return len(json)
         return -1


     data_str = '{"key": "value"}'
     data_dict = json.loads(data_str)

     print(f"Module loads: {data_dict['key']}")
     print(f"Function processes: {process_data(data_str)}")
     ```

2. **`stdlib_shadowing_local_var/main.py`**:
   - **Purpose**: Validates that a local variable assignment shadowing a stdlib module is handled correctly.
   - **Content**:
     ```python
     import os


     def find_files():
         os = ["file1.txt", "file2.txt"]
         for f in os:
             print(f"Found local file: {f}")


     print(f"Module path: {os.path.join('a', 'b')}")
     find_files()
     ```

3. **`stdlib_complex_alias/main.py`**:
   - **Purpose**: Validates that aliased `from` imports are correctly hoisted and rewritten.
   - **Content**:
     ```python
     from collections import abc as collections_abc
     from sys import version_info as py_version

     print(isinstance([], collections_abc.Sequence))
     if py_version.major > 2:
         print("Python 3+")
     ```

This revised plan presents a low-risk, high-value solution that corrects a fundamental flaw in the bundling logic by cleanly integrating semantically-aware processing into the existing architecture while providing a clear path for handling all stdlib dependencies.

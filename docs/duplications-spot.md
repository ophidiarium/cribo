# Code Duplication Analysis

This document outlines areas of duplicated or semantically similar code in the Cribo codebase, identified for potential refactoring.

### 1. Duplicated Test Setup Logic

**Files:**

- `crates/cribo/tests/test_pythonpath_support.rs`
- `crates/cribo/tests/test_virtualenv_support.rs`

**Description:**
Both files contain similar setup code for creating temporary directories and test files. This logic can be extracted into a shared test helper function to reduce redundancy and improve maintainability.

### 2. Similar Logic for `PYTHONPATH` and `VIRTUAL_ENV` Handling

**File:** `crates/cribo/src/resolver.rs`

**Description:**
The `ModuleResolver` has separate but very similar logic for handling `PYTHONPATH` and `VIRTUAL_ENV`.

- `PythonPathGuard` and `VirtualEnvGuard` are nearly identical and could be replaced by a single generic `EnvVarGuard`.
- The constructor functions `new_with_pythonpath`, `new_with_virtualenv`, and `new_with_overrides` can be simplified.

### 3. Redundant Test File

**File:** `crates/cribo/tests/test_python_version_config.rs`

**Description:**
The file `test_python_version_config.rs` appears twice in the file list with identical content. If this is not a listing error, the duplicate file should be removed.

### 4. Duplicated `run_cribo` Test Helper

**Files:**

- `crates/cribo/tests/test_cli_stdout.rs`
- `crates/cribo/tests/test_directory_entry_simple.rs`

**Description:**
The `run_cribo` test helper function is defined identically in two different test files. It should be moved to a shared test utility module.

### 5. Similar AST Traversal Logic

**Files:**

- `crates/cribo/src/graph_builder.rs`
- `crates/cribo/src/visitors/import_discovery.rs`

**Description:**
Both `GraphBuilder` and `ImportDiscoveryVisitor` traverse the AST. While their specific tasks differ, the core traversal logic could be shared to avoid duplication. A common visitor pattern could be used where different components can hook into the traversal.

### 6. Duplicated `get_python_executable` Logic

**Files:**

- `crates/cribo/src/util.rs`
- `crates/cribo/tests/test_bundling_snapshots.rs`

**Description:**
The logic for determining the Python executable path is duplicated. The test in `test_bundling_snapshots.rs` should reuse the function from `util.rs`.

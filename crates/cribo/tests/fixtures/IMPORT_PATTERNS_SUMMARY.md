# Python Fixture Import Pattern Summary

This document summarizes the import patterns tested by the Python fixtures located in `crates/cribo/tests/fixtures`. Each section describes a group of fixtures and the specific import nuances they aim to verify.

## Alias Transformation (`alias_transformation_test`)

- **Description**: Tests the bundling tool's ability to correctly handle various forms of import aliasing. This includes transforming or removing original import statements after processing aliases to avoid redundancy.
- **Patterns Tested**:
  - Regular import aliases for standard library modules:
    - `import json as j`
    - `import os as operating_system`
    - `import sys as system_info`
  - From-import aliases for local modules/packages:
    - `from utils.data_processor import process_data as process_a`
    - `from utils.data_processor import format_output as format_a`
    - `from utils.config_manager import load_config as config_a`
  - Mixed from-imports (aliased and non-aliased items from the same module):
    - `from utils.helpers import helper_func, debug_print as debug_a`
  - Non-aliased imports (should remain unchanged by alias-specific logic):
    - `import math`
    - `import hashlib`
- **Key Nuances**: Ensures that the bundler correctly resolves aliased names and manages the original import statements (e.g., by removing them if fully aliased or partially modifying them for mixed imports).

## AST Rewriting for Regular Import Aliases (`ast_rewriting_regular_import_aliases`)

- **Description**: Focuses specifically on the AST rewriting capabilities for `import ... as ...` statements, covering standard library (including deeply nested modules) and local modules.
- **Patterns Tested**:
  - Standard library aliases:
    - `import os as operating_system`
    - `import json as j`
    - `import sys as system_module`
  - Dotted module aliases:
    - `import collections.abc as abc_collections`
  - Nested dotted module aliases:
    - `import urllib.parse as url_parser`
  - Deeply nested module aliases:
    - `import xml.etree.ElementTree as xml_tree`
  - Local module aliases:
    - `import utils.helpers as helper_utils`
    - `import utils.config as config_module`
  - Non-aliased imports (should be ignored by this specific rewriting logic):
    - `import math`, `import random`, `import datetime`
- **Key Nuances**: Verifies that the AST rewriter correctly identifies and processes `import ... as ...` statements for various module types while leaving standard `import ...` statements untouched.

## Function-Level Module Imports (`function_level_module_import`)

- **Description**: Tests the scenario where an entire module (not just specific names) is imported using a `from ... import ...` statement from within a function's scope.
- **Patterns Tested**:
  - `from utils import calculator` (imported inside the `process_data` function)
- **Key Nuances**: This pattern is crucial for handling deferred imports, conditional imports, or imports localized to specific execution paths. The bundler must correctly identify and include `utils.calculator` even though the import is not at the top level of the module.

## Mixed Top-Level and Function-Level Imports (`mixed_import_patterns`)

- **Description**: Simulates a more complex application structure where import statements are found at both the module's top level and within functions, often to manage or break potential circular dependencies.
- **Patterns Tested**:
  - Top-level `from ... import ...`:
    - `from utils import format_message`
  - Function-level `from ... import ...` (used to defer imports):
    - `from config import Config` (inside `main` function)
    - `from app import Application` (inside `main` function)
- **Key Nuances**: Stresses the bundler's dependency resolution capabilities when import graphs are not solely defined by top-level statements. It must correctly trace dependencies even when imports are deferred to function scopes.

## Explicit Relative Imports (`stickytape_explicit_relative_import`)

- **Description**: Tests the handling of explicit relative imports within a package structure.
- **Patterns Tested**:
  - Main script imports a module from a sub-package: `import greetings.greeting`
  - Module within the sub-package uses an explicit relative import: `from .messages import message` (in `greetings/greeting.py`)
- **Key Nuances**: Verifies correct resolution of `.` in relative imports, indicating the current package.

## `from ... import ... as ...` (Module Alias) (`stickytape_import_from_as_module`)

- **Description**: Tests importing an entire module from a package and aliasing the module itself.
- **Patterns Tested**:
  - `from greetings import greeting as g`
- **Key Nuances**: The bundler must recognize that `g` is an alias for the `greetings.greeting` module and bundle accordingly.

## `from ... import ... as ...` (Value Alias) (`stickytape_import_from_as_value`)

- **Description**: Tests importing a specific name (variable, function, class) from a module and aliasing that name.
- **Patterns Tested**:
  - `from greeting import message as m`
- **Key Nuances**: Ensures the bundler correctly links `m` to the `message` object within the `greeting` module.

## Basic Local `from ... import ...` (`stickytape_script_with_single_local_from_import`)

- **Description**: Tests a fundamental `from module import name` statement for a local module.
- **Patterns Tested**:
  - `from greeting import message`
- **Key Nuances**: Basic test for resolving and including a specific named entity from a sibling module.

## Basic Local `import ...` (`stickytape_script_with_single_local_import`)

- **Description**: Tests a fundamental `import module` statement for a local module.
- **Patterns Tested**:
  - `import greeting`
- **Key Nuances**: Basic test for resolving and including an entire sibling module.

## Critical Regression: Module vs Value Import Detection

**IMPORTANT**: A critical regression has been identified where the bundler fails to distinguish between importing a module (`from package import module`) and importing a value from a module (`from module import value`). This causes incorrect bundling behavior.

### The Problem

When encountering `from X import Y`, the bundler currently doesn't check whether `Y` is:

1. A submodule of package `X` (should be treated as a module import)
2. A value (function, class, variable) defined in module `X` (should be treated as a value import)

### Affected Fixtures

The following fixtures currently fail due to this issue:

- `stickytape_explicit_relative_import_single_dot` - Module uses relative imports internally
- `stickytape_script_using_from_to_import_module` - Imports module from package
- `function_level_module_import` - Function-scoped module import

### Why It Works "By Accident" Sometimes

The bundler incorrectly creates a `SimpleNamespace` object for module imports, which only works when:

- The module contains only simple values (no functions referencing other imports)
- The module has no internal imports or dependencies
- No module-specific attributes are accessed (like `__name__`, `__file__`)

It fails when:

- The module has relative imports (`from . import config`)
- Functions in the module reference other modules
- The module needs proper module initialization

### Example Demonstration

```python
# greetings/config.py
DEFAULT_NAME = "User"

# greetings/greeting.py
from . import config
message = "Hello"
def get_default_greeting():
    return f"{message}, {config.DEFAULT_NAME}!"

# main.py
from greetings import greeting  # This is a MODULE import, not a value import
```

The bundler incorrectly generates:

```python
# Incorrect: treats greeting as values to inline
greeting = types.SimpleNamespace(message=message_greetings_greeting, get_default_greeting=get_default_greeting_greetings_greeting)
```

But `get_default_greeting_greetings_greeting` still references `config` which is not in scope, causing `NameError`.

### Required Fix

The bundler needs to:

1. Check if the imported name corresponds to a file/module
2. If yes, treat it as a module import and ensure proper module registration
3. If no, treat it as a value import and inline appropriately

### Implementation Guidance Using Existing Semantic Analysis

**IMPORTANT**: The codebase already has robust semantic analysis powered by `ruff_python_semantic`. The fix should leverage this existing infrastructure rather than adding ad-hoc patches.

#### Current Semantic Analysis Infrastructure

1. **SemanticBundler** (`semantic_bundler.rs`):
   - Already creates semantic models for each module
   - Tracks bindings with `BindingKind::Import` and `BindingKind::FromImport`
   - Has access to full semantic information about what each import references

2. **Semantic Model** provides:
   - `semantic.resolve_qualified_name()` - Can resolve what a name refers to
   - Binding information that knows the difference between module and value imports
   - Scope information for understanding import contexts

3. **ImportItem** in the codebase already has:
   - Information about whether it's a module or specific symbol
   - The resolved path information

#### Recommended Implementation Approach

1. **Enhance ImportItem classification**:
   ```rust
   // In the semantic analysis phase, determine import type
   let import_type = if resolver.is_module_path(&resolved_path) {
       ImportType::Module
   } else {
       ImportType::Value
   };
   ```

2. **Use semantic binding information**:
   - When processing `from X import Y`, query the semantic model
   - Check if `Y` resolves to a module path or a symbol within `X`
   - The semantic model already has this information through qualified name resolution

3. **Propagate import type through the pipeline**:
   - Pass the `ImportType` information from semantic analysis to code generation
   - In `code_generator.rs`, use this to decide between module wrapping vs value inlining

4. **Key insight**: The semantic model can tell us:
   - If `from greetings import greeting` has `greeting` as a submodule (file exists at `greetings/greeting.py`)
   - If `from greeting import message` has `message` as a value defined in `greeting.py`

#### Why This Approach Is Better

1. **Reuses existing infrastructure**: No need to reimplement path resolution or import analysis
2. **Consistent with ruff**: Uses the same semantic analysis that ruff uses for linting
3. **Handles edge cases**: The semantic model already handles complex cases like namespace packages, relative imports, etc.
4. **Future-proof**: As ruff's semantic analysis improves, the bundler automatically benefits

#### Code Locations to Modify

1. **`orchestrator.rs`**: Where imports are collected and analyzed
2. **`semantic_bundler.rs`**: Enhance to classify import types using semantic model
3. **`code_generator.rs`**: Use the import type classification to generate correct code
4. **`ImportItem` struct**: Add an `import_type` field to carry this information

The key is to query the semantic model during the analysis phase and propagate that information to the code generation phase, rather than trying to guess at code generation time.

## Potentially Missing Import Pattern Fixtures

Based on the existing fixtures, the following import patterns and scenarios might be under-tested or missing, and could be valuable additions for comprehensive bundling verification:

- **Star Imports (`from module import *`)**:
  - **Description**: Testing how the bundler handles wildcard imports, including how it resolves names, potential name clashes, and its impact on tree-shaking or unused symbol detection.
  - **Nuances**: Behavior with `__all__` in the imported module, re-exporting starred imports, and potential for including a large number of unnecessary symbols.
  - **Verification**: Currently NOT covered by existing fixtures.

- **Conditional Imports (within `if/else` blocks or `try/except`)**:
  - **Description**: Imports that only occur if certain conditions are met (e.g., based on Python version, OS, or feature flags) or as fallbacks in `try/except` blocks.
  - **Nuances**: Ensuring the bundler can trace these conditional paths, or if it conservatively includes all potential conditional imports. How does this interact with dead code elimination if the condition is statically determinable?
  - **Verification**: Currently NOT covered by existing fixtures.

- **Dynamic Imports using `importlib` or `__import__()`**:
  - **Description**: Scenarios where module names are determined at runtime and imported using `importlib.import_module()` or the built-in `__import__()` function.
  - **Nuances**: These are generally very hard for static analysis. Tests could explore what the bundler does: ignores them, attempts to find literal string arguments, or provides warnings. How does it handle cases where the module name is a variable?
  - **Verification**: `importlib.import_module()` is used in `xfail_stickytape_script_with_dynamic_import/main.py`. `__import__()` is NOT covered.

- **Imports within Class Definitions or Methods**:
  - **Description**: Imports that are scoped to a class definition (e.g., for type hints available only during class creation) or within instance/static/class methods.
  - **Nuances**: Similar to function-level imports but with the added context of class structure. This can affect when the import is resolved and its lifecycle.
  - **Verification**: Currently NOT covered by existing fixtures.

- **Relative Imports with Multiple Dots (e.g., `from ..subpkg import mod`, `from ... import mod`)**:
  - **Description**: Testing more complex relative imports that navigate higher up the package hierarchy.
  - **Nuances**: Ensuring correct path resolution for `..`, `...`, etc., especially in deeply nested package structures or when the entry point is itself within a package.

- **Namespace Packages (PEP 420)**:
  - **Description**: Importing modules from namespace packages, which can be split across multiple directories.
  - **Nuances**: How the bundler discovers and combines parts of a namespace package.

- **Imports from `sys.path` Modifications**:
  - **Description**: Scenarios where `sys.path` is manipulated at runtime before an import statement is encountered.
  - **Nuances**: Whether the bundler can detect or account for `sys.path` changes, or if it relies solely on a statically determined set of import paths.

- **Circular Imports Involving Aliases or Complex Paths**:
  - **Description**: While some circular dependencies might be tested, more intricate scenarios involving aliases, function-level imports, or multiple files in the cycle could reveal edge cases.
  - **Nuances**: How robustly the bundler detects and handles (or reports) various forms of circular dependencies, especially when aliasing obscures the direct circular path.

- **Imports of C Extensions or Built-in Modules with Non-Standard Behavior**:
  - **Description**: Testing imports of modules that are not plain Python files, such as compiled C extensions (`.so`, `.pyd`) or built-in modules.
  - **Nuances**: How the bundler identifies these (e.g., does it assume they are always present, or does it try to locate them?). How does it handle their (typically opaque) contents?

- **Re-exporting Imported Names (`__all__` interaction)**:
  - **Description**: A module imports names and then re-exports them, possibly using `__all__` to control the public API. Testing how the bundler traces these re-exports to the original definitions.
  - **Nuances**: Correctly identifying the true source of a re-exported name, especially if it involves multiple levels of re-export or aliasing during re-export.

- **Imports from `zip` archives (via `zipimport`)**:
  - **Description**: Modules imported from a `.zip` file that has been added to `sys.path`.
  - **Nuances**: This is an advanced case, but tests could determine if the bundler has any awareness of `zipimport` or if it fails gracefully.

- **Imports that trigger side effects in the imported module's global scope**:
  - **Description**: An `import` statement that, by merely being executed, causes code in the imported module's global scope to run and potentially alter global state or perform actions.
  - **Nuances**: Ensuring the bundler correctly includes and executes such top-level code in the bundled output if the import is deemed necessary, and how this interacts with tree-shaking if only a part of the module is used but the side-effecting code is at the top level.

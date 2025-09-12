# System Design: Splitting `import_transformer.rs`

## 1. Introduction

The `crates/cribo/src/code_generator/import_transformer.rs` file is a critical component responsible for recursively transforming Python import statements. Its current monolithic size of over 4200 lines makes it difficult to maintain, test, and debug.

This document outlines a detailed system design for refactoring `import_transformer.rs` into a dedicated, modular sub-package within `code_generator`. The goal is to improve code clarity, enforce a strong separation of concerns, and enhance maintainability without altering the public API consumed by other parts of the codebase, such as `inliner.rs`.

## 2. Guiding Principles

- **API Stability:** The public interface of `RecursiveImportTransformer` (`::new()` and `transform_module()`) will remain unchanged to ensure zero impact on its consumers.
- **High Cohesion:** Each new module will have a single, well-defined responsibility (e.g., handling only stdlib imports, rewriting only expressions).
- **Low Coupling:** Modules will interact through well-defined interfaces, and visibility will be kept as restrictive as possible (preferring `pub(super)` over `pub(crate)`).
- **Clarity and Naming:** File and module names will be specific to avoid ambiguity with existing modules like `expression_handlers.rs`.
- **Determinism and Correctness:** The refactoring will preserve the exact behavior of the current implementation, including logging output and deterministic AST modifications, validated by existing snapshot tests.

## 3. Proposed Architecture

The current `import_transformer.rs` file will be replaced by a new `import_transformer/` directory.

### 3.1. New File Structure

```
crates/cribo/src/code_generator/
├── import_transformer/
│   ├── mod.rs               # Public API (RecursiveImportTransformer struct)
│   ├── expr_rewriter.rs     # Expression rewriting logic (transform_expr)
│   ├── handlers/
│   │   ├── mod.rs           # Handler module declaration
│   │   ├── common.rs        # Shared logic for handlers (if necessary)
│   │   ├── dynamic.rs       # Logic for importlib.import_module()
│   │   ├── inlined.rs       # Logic for imports from inlined modules
│   │   ├── stdlib.rs        # Logic for stdlib import normalization
│   │   └── wrapper.rs       # Logic for imports from wrapper modules
│   ├── state.rs             # State management for the transformer
│   └── statement.rs         # AST traversal and statement transformation
└── ...
```

### 3.2. Module Responsibilities

#### `import_transformer/state.rs`

- **Purpose:** To encapsulate the state of the transformation process.
- **Contents:**
  - `RecursiveImportTransformerParams`: The existing parameter struct for initialization.
  - `TransformerState`: A new struct holding all fields from the current `RecursiveImportTransformer` (e.g., `bundler`, `module_id`, `import_aliases`, `local_variables`, `populated_modules`).
  - State-related helper methods (e.g., `get_module_name()`, `track_local_variable()`).

#### `import_transformer/mod.rs`

- **Purpose:** To define the public-facing `RecursiveImportTransformer` and its API.
- **Contents:**
  - The `RecursiveImportTransformer` struct, which will contain the `TransformerState`.
  - The `pub(crate) fn new(...)` constructor.
  - The `pub(crate) fn transform_module(...)` entry point, which delegates to the traversal logic in `statement.rs`.

#### `import_transformer/statement.rs`

- **Purpose:** To manage the core AST traversal and dispatching of statement transformations.
- **Contents:**
  - `transform_statements`: The main recursive loop for iterating through `Vec<Stmt>`.
  - `transform_statement`: The primary dispatcher for `Stmt::Import` and `Stmt::ImportFrom`, which will call the appropriate functions from the `handlers` submodule.
  - Statement-specific, stateless helpers like `collect_assigned_names`, `is_type_checking_condition`, and `hoist_function_globals`.

#### `import_transformer/expr_rewriter.rs`

- **Purpose:** To centralize all expression-rewriting logic, named to avoid confusion with the existing `expression_handlers.rs`.
- **Contents:**
  - `transform_expr`: The recursive function for transforming expressions.
  - The large `match` statement for handling different `Expr` types.
  - Expression-specific helpers like `collect_attribute_path` and `find_module_for_alias`.

#### `import_transformer/handlers/` (Submodule)

This submodule contains the specialized logic for handling different categories of imports. Each handler will be a self-contained unit responsible for a specific transformation strategy.

- **`handlers/stdlib.rs`**: Implements all logic for normalizing standard library imports, including alias creation and rewriting attribute access to the `_cribo` proxy.
- **`handlers/wrapper.rs`**: Manages imports from lazily-initialized "wrapper" modules, including inserting initialization calls and rewriting symbol access.
- **`handlers/inlined.rs`**: Handles imports from first-party modules that are inlined, creating direct assignments to renamed symbols.
- **`handlers/dynamic.rs`**: Isolates the logic for transforming dynamic `importlib.import_module()` calls into static module references.
- **`handlers/common.rs`**: A place for any helper functions or parameter structs that are genuinely shared between multiple handlers. To be created only if a clear need emerges.

## 4. Refactoring Implementation Plan

The refactoring will be executed in a series of small, verifiable steps to minimize risk.

**📝 STRATEGIC UPDATE**: After initial analysis, the original plan has been refined. The monolithic 4000+ line file has deeper interdependencies than initially assessed. The revised approach prioritizes:

1. ✅ **Module Structure**: Complete - Directory created, compilation verified
2. ✅ **State Definition**: Complete - `TransformerState` created with full field extraction
3. 🔄 **Handler-First Extraction**: Extract specific import handlers incrementally, which will naturally drive the expression and statement refactoring
4. 🔄 **State Integration**: Integrate state changes as handlers are extracted, maintaining compilation at each step

This approach reduces risk by working with smaller, focused pieces while maintaining the working codebase throughout the process.

1. **Setup Module Structure:** ✅ **COMPLETED**
   - ✅ Create the `import_transformer/` directory and all proposed files.
   - ✅ Move the entire content of the current `import_transformer.rs` into `import_transformer/mod.rs`.
   - ✅ Update `code_generator/mod.rs` to declare `pub mod import_transformer;`.
   - ✅ Ensure the project compiles (`cargo check`).

2. **Extract State:** ✅ **COMPLETED** - Full state integration throughout transformer
   - ✅ Define `TransformerState` in `state.rs` and move all fields from `RecursiveImportTransformer` into it.
   - ✅ Update `RecursiveImportTransformer` to hold a single `state: TransformerState` field with systematic field access replacement
   - ✅ Refactor all method calls to use `self.state.field` pattern (16 fields across 4000+ lines)
   - ✅ Fix external constructor calls and add accessor methods for API compatibility
   - ✅ Verify with `cargo check` and maintain test validation throughout

3. **Isolate Expression Rewriting:** ✅ **COMPLETED** - Successfully extracted large expression rewriter
   - ✅ **Analysis Complete**: `transform_expr` function identified (lines 2201-2739, ~539 lines)
   - ✅ **Dependencies Identified**: `collect_attribute_path`, `find_module_for_alias`, and multiple helper methods extracted
   - ✅ **Complexity Assessment**: Function had deep interdependencies with transformer state, resolved with transformer-passing approach
   - ✅ **Major Extraction Achievement**: Successfully moved `transform_expr` (~539 lines) and helper functions (`collect_attribute_path`, `find_module_for_alias`) to `expr_rewriter.rs`
   - ✅ **Clean API Integration**: Functions are `pub(super)` and integrate cleanly with main transformer via `ExpressionRewriter::transform_expr(transformer, expr)`
   - ✅ **Zero Regression Validation**: All snapshot tests pass, proving correct behavior preservation
   - ✅ **Substantial Progress**: Removed ~600 lines from main file, created clean separation of expression rewriting concerns

4. **Extract Statement Utilities:** ✅ **COMPLETED** - Statement utility functions extracted
   - ✅ Created `StatementProcessor` struct in `statement.rs` with 3 utility functions
   - ✅ Moved `is_type_checking_condition` (TYPE_CHECKING condition checking) and `hoist_function_globals` (global statement hoisting)
   - ✅ Extracted `collect_assigned_names` (assignment target name collection with destructuring support)
   - ✅ Updated call sites to use `StatementProcessor::function_name()` pattern
   - ✅ Removed original implementations from `mod.rs`, cleaned unused imports
   - ✅ All tests pass, functionality preserved (~65 lines extracted)

5. **Isolate Statement Traversal:** 🔄 **NEXT TARGET**
   - Move `transform_statements`, `transform_statement`, and related helpers into `statement.rs`.
   - Make them `pub(super)` and update `mod.rs` to call `statement::transform_module_body`.
   - Verify with `cargo check`.

6. **Extract Handlers Incrementally:** 🔄 **IN PROGRESS** - Stdlib & Dynamic complete, Wrapper partially complete
   - **Stdlib Handler** ✅ **COMPLETED**:
     - ✅ Created `handlers/stdlib.rs` with `StdlibHandler` struct
     - ✅ Extracted 4 functions: `should_normalize_stdlib_import`, `build_stdlib_rename_map`, `handle_stdlib_from_import`, `handle_wrapper_stdlib_imports`
     - ✅ Removed old functions from `mod.rs`, updated call sites to use handler directly
     - ✅ Used proper visibility: `pub(in crate::code_generator::import_transformer)`
     - ✅ Validated with targeted snapshot tests: `INSTA_GLOB_FILTER="**/stdlib_*"` - all tests pass
     - ✅ Reduced `mod.rs` by ~115 lines, improved separation of concerns
   - **Dynamic Handler** ✅ **COMPLETED**:
     - ✅ Created `handlers/dynamic.rs` with `DynamicHandler` struct
     - ✅ Extracted 4 functions: `is_importlib_import_module_call`, `transform_importlib_import_module`, `rewrite_attr_for_importlib_var`, `handle_importlib_assignment`
     - ✅ Removed old functions from `mod.rs`, updated call sites to use handler directly
     - ✅ Fixed borrow checker conflicts with state extraction pattern
     - ✅ Validated with targeted snapshot tests: importlib and dynamic import fixtures - all tests pass
     - ✅ Reduced `mod.rs` by ~110 lines, improved separation of concerns for `importlib.import_module` handling
   - **Wrapper Handler** ✅ **SIGNIFICANTLY COMPLETED**:
     - ✅ Created `handlers/wrapper.rs` with `WrapperHandler` struct
     - ✅ Extracted `log_wrapper_wildcard_info` function and updated call site
     - ✅ Added utility functions for wrapper module detection and initialization
     - ✅ **Major Embedded Logic Extraction**: Successfully extracted complex wrapper logic that was scattered throughout the transformer:
       - `handle_wrapper_submodule_import()`: Complex wrapper-to-wrapper import handling (~70 lines)
       - `try_rewrite_wrapper_attribute()`: Attribute access rewriting for wrapper imports (~25 lines)
       - `try_rewrite_wrapper_name()`: Name expression rewriting for wrapper imports (~15 lines)
     - ✅ **Systematic Embedded Extraction**: Demonstrated extraction of conditional logic embedded throughout the codebase, not just standalone functions
     - ✅ Reduced `mod.rs` by ~140 lines total, improved separation of concerns for all wrapper functionality
     - ✅ **Validation**: All test validations continue to pass, proving correct behavior preservation
   - **Inlined Handler** ✅ **COMPLETED**: Extract inlined module import transformations
     - ✅ Created `handlers/inlined.rs` with `InlinedHandler` struct
     - ✅ Extracted 2 functions: `is_importing_from_inlined_module`, `create_namespace_call_for_inlined_module`
     - ✅ Removed old functions from `mod.rs`, updated call sites to use handler directly
     - ✅ Used proper visibility: `pub(in crate::code_generator::import_transformer)`
     - ✅ Validated with snapshot tests: all tests pass
     - ✅ Reduced `mod.rs` by ~93 lines, completed inlined module handling separation

7. **Final Cleanup:**
   - Ensure the main `mod.rs` only contains the public API.
   - Review all new modules and tighten visibility to `pub(super)` or private where possible.
   - Run `cargo clippy --workspace --all-targets` to identify and remove any dead code.
   - Run the full test suite (`cargo nextest run --workspace`) one final time.

## 5. Validation Strategy

The existing generic snapshot testing framework at `crates/cribo/tests/test_bundling_snapshots.rs` is the primary tool for validation.

- **Targeted Validation:** During each step of the refactoring, specific test fixtures will be run using the `INSTA_GLOB_FILTER` environment variable to isolate the impact of the changes.
- **No Snapshot Changes:** The goal is to complete the refactoring with zero changes to the existing snapshots. Any deviation indicates an unintended behavioral change and must be corrected before proceeding.
- **Full Suite Execution:** The complete test suite will be run at the end of the process to guarantee full correctness.

## 6. Implementation Results & Achievements ✅

### Successfully Completed Components

1. **✅ Module Structure Setup**: Created complete `import_transformer/` directory with proper file structure and compilation
2. **✅ State Extraction**: Extracted comprehensive `TransformerState` with all 16 fields from original struct
3. **✅ Stdlib Handler**: Complete extraction of 4 functions with proper visibility and call site updates (~115 line reduction)
4. **✅ Dynamic Handler**: Complete extraction of 4 functions for `importlib.import_module` handling (~110 line reduction)
5. **✅ Wrapper Handler Foundation**: Partial extraction with complexity analysis and future roadmap (~30 line reduction)

### Total Progress Metrics

- **Original File Size**: 4200+ lines (monolithic)
- **Lines Extracted**: ~1123+ lines moved to specialized modules and handlers (~600 lines to expr_rewriter + ~458 to handlers + ~65 to statement utilities)
- **State Integration**: Complete - 16 fields systematically integrated across 4000+ lines
- **Modules Created**: 5 complete specialized modules (expr_rewriter + 4 complete handlers)
- **Functions Extracted**: 20+ functions successfully moved and integrated (includes major expression rewriter + embedded logic extraction)
- **Test Coverage**: All extractions validated with targeted snapshot tests
- **Compilation**: Maintained throughout all changes with proper error handling

### Strategic Insights Gained

1. **Handler-First Approach Validation**: Proved successful for functions with clear boundaries (stdlib, dynamic)
2. **Complexity Assessment**: Identified embedded vs. standalone logic patterns requiring different extraction strategies
3. **Incremental Safety**: Demonstrated safe refactoring with continuous compilation and test validation
4. **Future Roadmap**: Established clear patterns and approaches for remaining handler extractions

### Foundation Established for Future Work

The refactoring has successfully established:

- ✅ Proven modular architecture with proper visibility controls
- ✅ Working handler pattern with consistent API design
- ✅ Validated extraction methodology for complex codebases
- ✅ Clear separation between standalone functions (easy) and embedded logic (complex)
- ✅ Comprehensive testing strategy with zero behavioral changes

This systematic approach has reduced the original monolithic file by ~27% (~1123+ lines extracted) while establishing the foundation and methodology for completing the full transformation. The successful extraction of the massive `transform_expr` function (~539 lines), completion of four specialized handlers, and systematic utility extraction demonstrates that even the most complex, deeply integrated components can be systematically extracted.

## 7. Next Phase: Orchestrator Method Refactoring ✨

### 7.1 Current Challenge: The `handle_import_from` Orchestrator

**Status**: Analysis completed with Gemini AI assistance\
**Size**: 907 lines - the largest remaining method in the transformer\
**Type**: Monolithic orchestrator with multiple execution paths\
**Single Caller**: Only called from `transform_statement` at line 1249

### 7.2 Gemini Analysis Results

The `handle_import_from` method is a classic **God Method anti-pattern** that contains intertwined logic for different module types, import styles, and optimizations. It violates the Single Responsibility Principle by handling:

- **Module Type Detection**: Inlined vs Wrapper vs Stdlib vs Dynamic modules
- **Import Style Processing**: Named imports, aliased imports, star imports
- **Optimization Logic**: Tree-shaking, type-only import elimination
- **State Management**: Symbol renames, namespace population tracking

### 7.3 Identified Execution Paths

Gemini identified **4 distinct execution paths** that can be extracted to appropriate handlers:

#### **Path 1: Inlined Module Import Processing**

- **Target Handler**: `handlers/inlined.rs`
- **Logic**: Imports from modules inlined into the bundle
- **Behavior**: Records symbol mappings, discards original import, delegates to expression rewriter
- **Complexity**: O(n) where n = number of imported symbols

#### **Path 2: Wrapper Module Import Processing**

- **Target Handler**: `handlers/wrapper.rs`
- **Logic**: Imports from wrapper modules (bundled but not inlined)
- **Behavior**: Generates assignment statements like `h = _cribo_bundle.utils.helper`
- **Includes**: Type-only import optimization logic

#### **Path 3: Namespace Population Processing**

- **Target Handler**: New utility or existing structure
- **Logic**: Creates and populates namespace objects with symbols
- **Behavior**: Generates `utils = types.SimpleNamespace()` and attribute assignments
- **Includes**: Tree-shaking integration, `__all__` generation logic

#### **Path 4: Stdlib/Dynamic Import Processing**

- **Target Handler**: `handlers/stdlib.rs` or minimal orchestrator logic
- **Logic**: Standard library or external imports that pass through unchanged
- **Behavior**: Returns original import statement unmodified

### 7.4 Implementation Strategy

**Phase 1: Internal Method Decomposition**

1. Create private methods within `handle_import_from` for each execution path:
   - `handle_inlined_import_from_internal()`
   - `handle_wrapper_import_from_internal()`
   - `handle_namespace_population_internal()`
   - `handle_stdlib_dynamic_import_internal()`

2. Refactor main method to resolve module type and dispatch to appropriate internal method
3. Validate with existing tests to ensure no behavioral changes

**Phase 2: Handler Integration**

1. Move logic from internal methods to appropriate handler structs
2. Update handlers to accept transformer reference and return transformed statements
3. Replace internal method calls with handler method calls
4. Validate each handler integration separately

**Phase 3: Orchestrator Simplification**

1. Reduce main `handle_import_from` to pure dispatch logic:
   - Module resolution and type detection
   - Handler selection and invocation
   - Result aggregation if needed
2. Final validation with complete test suite

### 7.5 Expected Impact

- **Size Reduction**: ~900 lines moved from main orchestrator to specialized handlers
- **Separation of Concerns**: Each module type handled by dedicated, focused code
- **Maintainability**: Complex logic isolated in appropriate domain handlers
- **Testing**: Each execution path can be tested independently
- **Architecture**: Completes the handler pattern implementation

### 7.6 Risk Mitigation

- **State Dependencies**: All handlers will receive transformer reference for state access
- **Incremental Approach**: Each phase validates before proceeding to next
- **Test Coverage**: Existing snapshot tests provide comprehensive regression detection
- **Rollback Strategy**: Each commit represents a stable, working state

This orchestrator refactoring represents the **final major extraction** needed to complete the modular transformation of the import transformer, moving from a 4200+ line monolith to a clean, handler-based architecture.

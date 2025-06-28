# Code Generator Refactoring TODO List

This document tracks the remaining work to complete the refactoring of `code_generator.rs` into smaller, focused modules.

## High Priority - Core Functionality Migration

### 1. AST Node Indexing (`bundler.rs`)

- [x] **Line 367**: Implement `NodeIndexAssigner` visitor to assign indices to AST nodes
  - **Original location**: `code_generator_old.rs:2560-2655`
  - **Components to migrate**:
    - Struct definition: lines 2560-2562
    - `SourceOrderVisitor` implementation: lines 2564-2650
    - Usage in `assign_node_indices_to_ast`: lines 2652-2654
  - This is critical for AST transformation tracking
  - **Completed**: Migrated in commit b4ba5a6

### 2. Expression Equality Checking (`bundler.rs`)

- [x] **Line 1035**: Implement proper expression equality checking
  - **Original location**: `code_generator_old.rs:2658-2667`
  - **Function signature**: `fn expr_equals(expr1: &Expr, expr2: &Expr) -> bool`
  - **Implementation**: Recursive comparison for Name and Attribute expressions
  - Required for duplicate import detection
  - **Completed**: Migrated in commit 4849215

### 3. Circular Dependencies (`circular_deps.rs`)

- [x] **Line 140**: Implement `ClassDependencyCollector` to analyze class dependencies
  - **Original location**: `code_generator_old.rs:3094-3126`
  - **Function**: `analyze_class_dependencies`
  - **Note**: The TODO mentions a visitor pattern, but the original uses a direct analysis function
  - **Key logic**: Analyzes base classes and method dependencies for circular module detection
  - Required for proper symbol dependency graph construction
  - **Completed**: Implemented analyze_class_dependencies, analyze_function_dependencies, and analyze_assignment_dependencies in commit 13331b7

### 4. Import Transformation (`import_transformer.rs`)

- [x] **Line 1137**: Complete the import transformation implementation
  - **Original location**: `code_generator_old.rs:1607-1616` (end of `handle_import_from`)
  - **Missing call**: `rewrite_import_in_stmt_multiple_with_full_context`
  - **Implementation**: Falls back to standard transformation for non-inlined imports
  - **Completed**: Added temporary fallback in commit 8c1277f (proper implementation pending bundler method availability)

## Medium Priority - Module Processing Functions

### 5. Module Transformation Pipeline (`bundler.rs`)

- [x] **Line 399**: `transform_module_to_init_function` - Convert module to init function
  - **Original location**: `code_generator_old.rs:6076-6612`
  - **Large function**: Transforms module body into an init function
  - **Status**: Stub added in commit a9b309c, implementation pending
- [ ] **Line 410**: `transform_module_to_cache_init_function` - Convert module for cache initialization
  - **Original location**: `code_generator_old.rs:14177-14210`
  - **Purpose**: Similar to above but for module cache system
- [ ] **Line 426**: `inline_module` - Inline module implementation
  - **Original location**: `code_generator_old.rs:10868-11072`
  - **Core function**: Handles inlining of module bodies
- [ ] **Line 436**: `inline_class` - Inline class implementation
  - **Original location**: `code_generator_old.rs:12944-13094`
  - **Purpose**: Inlines class definitions with renaming
- [ ] **Line 468**: `inline_assignment` - Inline assignment implementation
  - **Original location**: `code_generator_old.rs:13097-13218`
  - **Purpose**: Handles assignment statement inlining
- [ ] **Line 486**: `inline_ann_assignment` - Inline annotated assignment implementation
  - **Original location**: `code_generator_old.rs:13221-13298`
  - **Purpose**: Handles annotated assignment inlining

### 6. Import Processing (`bundler.rs`)

- [x] **Line 569**: Implement proper stdlib module checking
  - **Original location**: `code_generator_old.rs:10024-10026`
  - **Implementation**: Calls `crate::side_effects::is_safe_stdlib_module(module_name)`
  - **Completed**: In commit 5275ee7
- [ ] **Line 788**: Implement import classification based on semantic analysis
  - **Context**: Function `collect_module_renames` needs semantic-based classification
  - **Note**: May need to integrate with `ImportType` from resolver/semantic_analysis modules
- [x] **Line 976**: Collect imports from entry module
  - **Original location**: `code_generator_old.rs:7811-7841`
  - **Function**: `get_entry_module_imports`
  - **Completed**: In commit 6592e3c
- [ ] **Line 2282**: Implement relative import collection
  - **Original location**: `code_generator_old.rs:10325-10394`
  - **Function**: `collect_direct_relative_imports`

### 7. Statement Processing (`bundler.rs`)

- [ ] **Line 1047**: Implement entry module statement processing
  - **Original location**: `code_generator_old.rs:5780-5844`
  - **Function**: `process_entry_module_statement`
- [ ] **Line 2085**: Complete statement transformation implementation
  - **Context**: Tree-shaking logic for checking if imports are used by surviving code
  - **Purpose**: Determine if assignments using imports should be kept

## Low Priority - Helper Functions

### 8. Dependency Analysis (`bundler.rs`)

- [ ] **Line 835**: Implement proper dependency sorting algorithm
  - **Original location**: `code_generator_old.rs:3243-3383`
  - **Function**: `sort_wrapper_modules_by_dependencies`
- [ ] **Line 847**: Build symbol dependency graph
  - **Original location**: `code_generator_old.rs:2990-3044`
  - **Function**: `build_symbol_dependency_graph`
- [ ] **Line 858**: Detect hard dependencies between modules
  - **Original location**: `code_generator_old.rs:13851-13987`
  - **Function**: `detect_hard_dependencies`
- [ ] **Line 966**: Sort wrapper modules by dependencies
  - **Original location**: `code_generator_old.rs:3386-3479`
  - **Function**: `sort_wrapped_modules_by_dependencies`

### 9. Module Cache Infrastructure (`bundler.rs`)

- [ ] **Line 873**: Generate module cache initialization
  - **Original location**: `code_generator_old.rs:14039-14059`
  - **Function**: `generate_module_cache_init`
- [ ] **Line 885**: Populate module cache
  - **Original location**: `code_generator_old.rs:14061-14115`
  - **Function**: `generate_module_cache_population`
- [ ] **Line 891**: Implement sys.modules synchronization
  - **Original location**: `code_generator_old.rs:14117-14175`
  - **Function**: `generate_sys_modules_sync`

### 10. Code Generation (`bundler.rs`)

- [ ] **Line 864**: Generate module namespace class
  - **Original location**: `code_generator_old.rs:14019-14037`
  - **Function**: `generate_module_namespace_class`
- [ ] **Line 902**: Process globals
  - **Original location**: `code_generator_old.rs:9310-9340`
  - **Function**: `process_wrapper_module_globals`
- [ ] **Line 912**: Transform module
  - **Note**: Refers to various transformation functions already listed
- [ ] **Line 925**: Generate module init calls
  - **Original location**: `code_generator_old.rs:7670-7809`
  - **Function**: `generate_module_init_call`
- [ ] **Line 937**: Inline modules
  - **Note**: Duplicate of item 5.3 (`inline_module`)
- [ ] **Line 947**: Create namespace for inlined modules
  - **Original location**: `code_generator_old.rs:6621-6769`
  - **Function**: `create_namespace_for_inlined_submodule`
- [ ] **Line 956**: Generate registries and hooks
  - **Original location**: `code_generator_old.rs:6615-6619`
  - **Function**: `generate_registries_and_hook`
- [ ] **Line 987**: Generate submodule attributes
  - **Original location**: `code_generator_old.rs:7843-8153`
  - **Function**: `generate_submodule_attributes_with_exclusions`

### 11. Deduplication (`bundler.rs`)

- [ ] **Line 996**: Implement deduplication logic
  - **Original location**: `code_generator_old.rs:7362-7606`
  - **Function**: `deduplicate_deferred_imports_with_existing`
- [ ] **Line 1002**: Check for duplicate imports
  - **Original location**: `code_generator_old.rs:2694-2718`
  - **Function**: `is_duplicate_import`
- [ ] **Line 1008**: Check for duplicate statements
  - **Original location**: `code_generator_old.rs:2669-2692`
  - **Function**: `is_duplicate_import_from`

## Detailed Code Mapping Summary

All TODO items have been mapped to their original implementations:

- **High Priority (4 items)**: AST indexing, expression equality, circular dependencies, import transformation
- **Medium Priority (16 items)**: Module transformation pipeline, import processing, statement processing
- **Low Priority (16 items)**: Dependency analysis, module cache, code generation, deduplication

**Total: 36 specific functions/features to migrate**

## Migration Strategy

### Phase 1: Core Infrastructure (Highest Priority)

1. Migrate `NodeIndexAssigner` - Critical for AST transformation
2. Migrate `expr_equals` - Required for duplicate detection
3. Complete `ClassDependencyCollector` - Needed for dependency analysis

### Phase 2: Module Processing Pipeline

1. Migrate all `inline_*` functions as a group
2. Migrate `transform_module_to_*` functions
3. Ensure proper integration with existing context structures

### Phase 3: Import Resolution

1. Complete stdlib module checking
2. Implement semantic-based import classification
3. Add relative import collection

### Phase 4: Final Integration

1. Implement all dependency sorting algorithms
2. Add module cache infrastructure
3. Complete code generation functions
4. Add deduplication logic

## Testing Requirements

Each migrated function should:

1. Have unit tests covering edge cases
2. Pass all existing integration tests
3. Maintain the same behavior as the original implementation
4. Work with the refactored module structure

## Notes

- Many TODOs reference "Implementation from original file" - these require careful migration from `code_generator_old.rs`
- The refactoring should maintain the same public API to avoid breaking changes
- Use the existing context structures in `context.rs` for parameter passing
- Leverage the `GlobalsLifter` in `globals.rs` for global variable handling
- Utilize `RecursiveImportTransformer` in `import_transformer.rs` for import rewriting

## Progress Tracking

- [x] Phase 1: Core Infrastructure (3/3 tasks) ✅
- [ ] Phase 2: Module Processing Pipeline (1/6 tasks) - transform_module_to_init_function stub added
- [ ] Phase 3: Import Resolution (2/3 tasks) - stdlib checking, entry module imports
- [ ] Phase 4: Final Integration (0/4 tasks)

Total: 6/16 major tasks completed (with some partial implementations)

## Migration Helpers

Use the following tools for efficient migration:

- `ast-grep` for finding and extracting specific functions
- `mcp__Gemini__*` tools for analyzing large code sections
- Git worktree for side-by-side comparison with main branch

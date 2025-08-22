# Circular Dependency Challenge: PyYAML Bundle Failure

## Executive Summary

This document details a critical issue discovered when bundling PyYAML with cribo, where circular dependencies involving the entry module cause incorrect code ordering in the bundled output. The issue manifests as a `NameError: name 'Loader' is not defined` when the bundled code attempts to use symbols before they are defined.

## Problem Description

### Test Failure

When running `./ecosystem/run_tests.py pyyaml`, the test fails with:

```python
NameError: name 'Loader' is not defined
```

The error occurs in the bundled output at line 1331:

```python
class YAMLObject_1(metaclass=YAMLObjectMetaclass_1):
    yaml_loader = [Loader, FullLoader, UnsafeLoader]  # <- Error here
```

### Bundle Structure Analysis

The bundled output has the following problematic structure:

- Line 1315: `YAMLObjectMetaclass_1` class definition (from `yaml/__init__.py`)
- Line 1328: `YAMLObject_1` class definition (from `yaml/__init__.py`)
- Line 1331: References to `Loader`, `FullLoader`, `UnsafeLoader` (undefined at this point)
- Line 3763: `BaseLoader` class definition (from `yaml.loader`)
- Line 3009: `Loader` class definition (from `yaml.loader`)
- Line 2989: `FullLoader` class definition (from `yaml.loader`)
- Line 3019: `UnsafeLoader` class definition (from `yaml.loader`)

The `YAMLObject` class from the entry module (`yaml/__init__.py`) is being placed at line 1328, but it depends on `Loader` classes that aren't defined until line 2989+.

## Investigation Process

### Theory 1: Module Dependency Ordering Issue

**Hypothesis**: The modules are not being sorted in dependency order before bundling.

**Investigation**:

1. Checked dependency edges in the graph:
   ```
   yaml.loader -> __init__ (meaning __init__ depends on yaml.loader)
   ```
2. Verified that the dependency graph correctly identifies that `__init__.py` depends on `yaml.loader`
3. Examined the topological sort implementation in `cribo_graph.rs`

**Finding**: The dependency graph is correct - it properly identifies that `__init__` depends on `yaml.loader`. The topological sort should place `yaml.loader` before `__init__`.

**Outcome**: This wasn't the root cause. The dependency tracking is working correctly.

### Theory 2: Inlinable Modules Not Sorted by Dependencies

**Hypothesis**: When modules are classified as "inlinable" vs "wrapper", the inlinable modules are processed in an arbitrary order rather than dependency order.

**Investigation**:

1. Found that modules are classified into two categories:
   - Inlinable modules: No side effects, can be directly inlined
   - Wrapper modules: Have side effects, need to be wrapped in init functions
2. Discovered that `yaml.loader` is inlinable while `yaml` uses wrapper approach
3. Found that inlinable modules were being processed in the order they appear in the classification result, not in dependency order

**Fix Attempted**:

```rust
// Sort inlinable modules according to dependency order from sorted_modules
let mut sorted_inlinable_modules = Vec::new();
for (sorted_name, _, _) in params.sorted_modules {
    if let Some(module_data) = inlinable_modules
        .iter()
        .find(|(name, _, _, _)| name == sorted_name)
    {
        sorted_inlinable_modules.push(module_data.clone());
    }
}
```

**Outcome**: The fix was applied but didn't resolve the issue. The problem was deeper than just the ordering of inlinable modules.

### Theory 3: Entry Module Content Being Split

**Hypothesis**: The entry module (`yaml/__init__.py`) content is being split, with some parts placed early and others placed late in the bundle.

**Investigation**:

1. Found that `YAMLObjectMetaclass_1` appears at line 1315 (too early)
2. Found that `__version__` from the same module appears at line 4806 (much later)
3. Discovered that content from `yaml/__init__.py` is being placed in multiple locations

**Finding**: The entry module content is indeed being split. The classes that have circular dependencies are being extracted and placed early, while the rest of the module content is placed at the end.

**Outcome**: This confirmed a major issue but wasn't the complete picture.

### Theory 4: Circular Dependency Resolution Extracting Content

**Hypothesis**: The circular dependency resolution mechanism is extracting content from modules and placing it incorrectly.

**Investigation**:

1. Discovered that PyYAML has a massive circular dependency involving all modules:
   ```
   CircularDependencyGroup { 
     modules: ["yaml.resolver", "yaml.representer", "yaml.serializer", 
               "yaml.constructor", "yaml.dumper", "yaml.emitter", 
               "yaml.composer", "yaml.parser", "yaml.scanner", 
               "yaml.loader", "yaml.reader", "yaml", "yaml.cyaml"], 
     cycle_type: FunctionLevel
   }
   ```
2. Found that when the entry module (`yaml`) is part of circular dependencies, its statements are reordered
3. The reordering function `reorder_statements_for_circular_module` rearranges statements based on symbol dependencies

**Finding**: This is the root cause. The circular dependency resolution is extracting `YAMLObjectMetaclass` and `YAMLObject` from `yaml/__init__.py` and placing them early in the bundle, before their dependencies are defined.

**Outcome**: Root cause identified.

### Theory 5: Content Appearing Between Wrapper Functions

**Hypothesis**: The extracted content is being incorrectly inserted between wrapper module definitions.

**Investigation**:

1. Found that the problematic content appears right after `yaml.reader` wrapper function ends (line 1312)
2. Lines 1313-1314 are imports from `yaml._yaml`
3. Lines 1315+ are the YAMLObjectMetaclass and YAMLObject classes
4. This content should be with the entry module at the end of the file

**Finding**: The circular dependency resolution is inserting content at the wrong location in the bundle structure.

**Outcome**: Confirmed the specific mechanism of the bug.

## Root Cause Analysis

The root cause is a fundamental issue in how cribo handles circular dependencies when the entry module is involved:

1. **Circular Dependency Detection**: PyYAML has circular dependencies involving all its modules, including the entry module (`yaml/__init__.py`)

2. **Statement Reordering**: When circular dependencies are detected, cribo attempts to reorder statements within modules to break the cycles

3. **Entry Module Special Case**: The entry module is treated specially - its content should go at the end of the bundle. However, when it's part of circular dependencies, the reordering logic extracts some of its content

4. **Incorrect Placement**: The extracted content (YAMLObjectMetaclass and YAMLObject classes) is placed early in the bundle, between wrapper module definitions, before the symbols they depend on (Loader, FullLoader, etc.) are defined

5. **Symbol Dependencies Not Honored**: The reordering doesn't properly account for the fact that YAMLObject depends on Loader classes, so it places YAMLObject before Loader is defined

## Attempted Fixes

### Fix 1: Sort Inlinable Modules by Dependencies

**Status**: Implemented but insufficient

Ensured that inlinable modules are processed in dependency order. This helps with general module ordering but doesn't address the core issue of content being extracted from the entry module.

### Fix 2: Entry Module Dependency Check (Considered)

**Status**: Not implemented

Considered adding logic to ensure entry module dependencies are satisfied before processing its content. However, this wouldn't address the fundamental issue of content being extracted and misplaced.

## Proposed Solutions

### Solution 1: Prevent Entry Module Content Extraction

**Priority**: High
**Complexity**: Medium

Modify the circular dependency resolution to never extract content from the entry module. The entry module should always be processed as a whole at the end of the bundle.

Implementation approach:

1. In `reorder_statements_for_circular_module`, check if the module is the entry module
2. If yes, skip reordering and return statements as-is
3. Ensure entry module content is always placed at the end

### Solution 2: Defer Symbol-Dependent Classes

**Priority**: High
**Complexity**: High

Implement a mechanism to defer classes that depend on not-yet-defined symbols until after their dependencies are available.

Implementation approach:

1. Analyze class bodies for symbol dependencies
2. Build a dependency graph at the statement level
3. Defer statements that have unresolved dependencies
4. Place deferred statements after all dependencies are satisfied

### Solution 3: Two-Phase Bundle Generation

**Priority**: Medium
**Complexity**: High

Separate the bundling into two phases:

1. Phase 1: Bundle all non-entry modules
2. Phase 2: Process and append entry module content

This ensures that all dependencies are available before the entry module is processed.

### Solution 4: Improve Circular Dependency Resolution

**Priority**: High
**Complexity**: Very High

Redesign the circular dependency resolution to better handle cases where the entry module is involved:

1. Identify hard dependencies (like class inheritance and class attributes)
2. Ensure hard dependencies are always satisfied before dependent code
3. Use function-scoped imports more aggressively for circular cases
4. Never break up class definitions - keep them atomic

### Solution 5: Special Case for PyYAML Pattern

**Priority**: Low
**Complexity**: Low

Add a specific workaround for the PyYAML pattern where:

1. Detect when YAMLObject-like classes depend on Loader-like classes
2. Ensure Loader classes are defined before YAMLObject classes
3. This is a band-aid fix but could unblock PyYAML immediately

## Recommendations

### Immediate Action

Implement Solution 1 (Prevent Entry Module Content Extraction) as it's the most straightforward fix that addresses the root cause without major architectural changes.

### Long-term Fix

Implement Solution 4 (Improve Circular Dependency Resolution) to properly handle all edge cases involving circular dependencies and entry modules.

### Testing Strategy

1. Add a specific test case for PyYAML bundling
2. Create minimal reproducible test cases for circular dependencies involving entry modules
3. Test with other packages that have similar circular dependency patterns

## Code Locations

Key files involved in this issue:

- `crates/cribo/src/code_generator/bundler.rs`: Main bundling logic, module classification
- `crates/cribo/src/code_generator/circular_deps.rs`: Circular dependency handling
- `crates/cribo/src/code_generator/inliner.rs`: Module inlining logic
- `crates/cribo/src/orchestrator.rs`: High-level bundling orchestration
- `crates/cribo/src/cribo_graph.rs`: Dependency graph and topological sorting

Key functions:

- `bundle_modules()`: Main bundling entry point
- `reorder_statements_for_circular_module()`: Reorders statements for circular modules
- `classify_modules()`: Classifies modules as inlinable vs wrapper
- `inline_all_modules()`: Inlines all inlinable modules
- `process_entry_module_statement()`: Processes entry module statements

## Lessons Learned

1. **Entry Module Special Cases**: Entry modules require special handling throughout the bundling process, especially when they're part of circular dependencies

2. **Circular Dependencies Complexity**: Circular dependency resolution is one of the most complex aspects of bundling, requiring careful consideration of symbol dependencies, statement ordering, and module boundaries

3. **Testing Real-World Packages**: Testing with real packages like PyYAML reveals edge cases that might not be apparent in synthetic test cases

4. **Symbol Dependency Tracking**: Class attributes that reference other symbols create hard dependencies that must be honored in the bundled output

5. **Module Content Atomicity**: Breaking up module content (especially classes) can lead to subtle bugs where symbols are used before they're defined

## Conclusion

The PyYAML bundling failure reveals a fundamental issue in how cribo handles circular dependencies when the entry module is involved. The current approach of extracting and reordering content from the entry module breaks the assumption that the entry module's content should appear at the end of the bundle. The fix requires either preventing content extraction from the entry module or implementing more sophisticated dependency tracking that ensures symbols are defined before they're used, even in the presence of circular dependencies.

## Postmortem (2024-08-22)

### Was the Original Theory Correct?

**Yes, the original theory was correct.** The investigation confirmed that:

1. The entry module (`yaml/__init__.py`) was indeed part of circular dependencies
2. The `reorder_statements_for_circular_module` function was being called on the entry module
3. The circular dependency resolution was extracting and reordering content from the entry module
4. This extraction was placing `YAMLObject` and `YAMLObjectMetaclass` classes in the wrong location

### Additional Complexity Discovered

During implementation, we discovered an additional layer of complexity:

- The entry module was being registered twice: once as `__init__` (the actual entry) and once as `yaml` (the package name)
- Both registrations pointed to the same file (`yaml/__init__.py`)
- This duplication meant the module was being processed multiple times in different contexts

### Implemented Fixes

Three complementary fixes were implemented to address the issue:

1. **Prevent Entry Module Reordering** (`reorder_statements_for_circular_module`):
   - Added logic to detect when a module is the entry module
   - Skip statement reordering entirely for the entry module
   - Handle the case where entry is `__init__.py` but module is identified by package name

2. **Skip Duplicate Package in Classification** (`classify_modules`):
   - When entry is `__init__.py`, skip processing the package-named module
   - Prevents the same file from being classified twice

3. **Remove Package from Circular Modules** (`bundle_modules`):
   - After identifying circular modules, remove the package name if entry is `__init__.py`
   - Ensures the package isn't treated as a separate circular module

### Current Status

**Partially Fixed**: The implemented fixes successfully:

- ✅ Prevent the entry module from being reordered
- ✅ Eliminate duplicate processing of the same file
- ✅ Reduce bundle size (186.4 KB vs 207.3 KB, indicating duplicate elimination)
- ✅ Pass all existing tests

**Still Failing**: However, the PyYAML test still fails with the same fundamental issue:

- ❌ `YAMLObject_1` still appears before `YAMLObjectMetaclass_1`
- ❌ Both classes still appear at line ~1303 instead of at the end with the entry module
- ❌ The classes appear immediately after imports from `yaml._yaml`

### Root Cause Still Present

The fixes addressed the symptoms but not the complete root cause. The classes are still being extracted and placed incorrectly, likely through a different mechanism:

- Possibly through hard dependency handling
- Possibly through symbol-level circular dependency resolution
- Possibly through a separate extraction mechanism for circular module symbols

### Next Steps Required

To fully fix the issue, investigation is needed into:

1. Why the classes are appearing after `yaml._yaml` imports
2. Whether hard dependency hoisting is extracting these classes
3. Whether the symbol dependency graph is causing extraction despite the entry module checks
4. Whether there's another code path that extracts circular module content

The core insight from Theory 4 remains valid: content should never be extracted from the entry module, regardless of circular dependencies. However, there appears to be another extraction mechanism that hasn't been identified and fixed yet.

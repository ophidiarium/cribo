# System Design: Dependency-Ordered Module Processing

## Current Implementation Issues (December 2024)

### Multiple Conflicting Sorting Mechanisms

The bundler currently has **THREE separate sorting mechanisms** that conflict with each other:

1. **Main dependency graph sorting** - The correct topological order of all modules
2. **Wrapper module sorting** - A separate sort just for wrapper modules (`sort_wrapper_modules_by_dependencies`)
3. **Symbol-level sorting** - Ordering of symbols within circular modules (`topological_sort_symbols`)

### The Root Problem

The bundler processes modules in **two separate phases based on classification**:

1. **Phase 1**: Process all inlinable modules (modules without side effects)
2. **Phase 2**: Process all wrapper modules (modules with side effects or in circular dependencies)

This **violates the fundamental principle** that "dependency graph dictates inlining order". The classification (inlinable vs wrapper) is determining WHEN modules are processed, not just HOW they're processed.

### Why This Fails

When an inlinable module `pkg` imports from a wrapper module `pkg.pretty`:

- `pkg` is processed in Phase 1 (it's inlinable)
- `pkg.pretty` is processed in Phase 2 (it's a wrapper due to circular deps)
- Result: `pkg` tries to call `pkg.pretty`'s init function before it's defined → `NameError`

### Example Failure Scenario

```python
# Module structure:
# - foo_wrapper: has side effects, needs init function
# - bar_inlined: no side effects, imports from foo_wrapper
# - main: imports both

# Current incorrect output order:
bar_var = foo_wrapper.some_function()  # From bar_inlined
# ... many lines later ...
def _cribo_init_foo_wrapper():         # Definition of foo_wrapper's init
    # ...
foo_wrapper = _cribo_init_foo_wrapper()  # Call to init foo_wrapper
```

The call to `foo_wrapper.some_function()` fails because `foo_wrapper` hasn't been initialized yet.

## Root Cause Analysis

The fundamental issue is that we're categorizing modules by their processing type (inlined vs wrapped) and then processing each category as a batch, rather than respecting the dependency order that ensures modules are processed only after their dependencies are available.

### Architectural Dissonance (Gemini Analysis - 95% Confidence)

The bundler exhibits **two conflicting strategies** for code ordering:

1. **Coarse-grained two-phase model**: Process all inlined modules first, then all wrapped modules
2. **Fine-grained circular dependency system**: Symbol-level dependency graph with pre-declarations

This creates ambiguity about which mechanism is responsible for solving specific ordering problems. The two-phase model appears to be either a legacy concern or an orthogonal optimization that has outlived its usefulness.

### Current Flow

```
1. Classify modules → [inlinable_modules], [wrapper_modules]
2. Process all inlinable_modules → generate inlined statements
3. Process all wrapper_modules → generate init functions
4. Combine everything
```

### Why This Breaks

When an inlined module imports from a wrapper module, the import transformation generates:

- A call to the wrapper module's init function: `wrapper = _cribo_init_wrapper()`
- An assignment from the wrapper: `var = wrapper.attribute`

But the init function `_cribo_init_wrapper` hasn't been defined yet because wrapper modules are processed later.

## Proposed Solution: Single Source of Truth

### Core Principle

**The dependency graph is the ONLY source of truth for processing order**. All modules must be processed in the order dictated by the dependency graph, regardless of their classification.

### Key Changes

1. **Remove all secondary sorting mechanisms**:
   - Remove `sort_wrapper_modules_by_dependencies()`
   - Remove separate wrapper module processing phase
   - Remove symbol-level sorting for circular modules (handle within modules)

2. **Use classification only for HOW, not WHEN**:
   - Inlinable → inline the module's code directly
   - Wrapper → create an init function
   - But both are processed in dependency order

3. **Fix the dependency graph if needed**:
   - If something is emitted in the wrong place, the dependency graph itself is wrong
   - Don't add post-processing sorts to "fix" ordering issues
   - Improve the graph construction to capture all real dependencies

### New Flow

```
1. Get topologically sorted module list from dependency graph
2. For each module in that exact order:
   - Classify it (inlinable vs wrapper)
   - Process it according to its classification
   - Emit the result immediately in sequence
3. Handle entry module and final cleanup
```

### Why This Works

- Dependencies are always available when needed (that's what topological sort guarantees)
- No forward references to undefined functions
- Circular dependencies are already handled by the wrapper mechanism
- The output order matches the logical dependency structure

## Implementation Details

### 1. Available Dependency Order

The topologically sorted module list is already computed and passed to the bundler:

```rust
pub struct BundleParams<'a> {
    pub modules: &'a [(String, ModModule, PathBuf, String)],
    pub sorted_modules: &'a [(String, PathBuf, Vec<String>)], // ← This is the correct order!
                                                              // ...
}
```

`sorted_modules` contains ALL modules in dependency order from the graph's topological sort.

### 2. Implementation Approach

#### Step 1: Remove Secondary Sorts

```rust
// REMOVE THIS:
let sorted_wrapper_modules = module_transformer::sort_wrapper_modules_by_dependencies(
    &wrapper_modules_saved,
    params.graph,
);

// REMOVE THIS:
self.symbol_dep_graph.topological_sort_symbols(&self.circular_modules)
```

#### Step 2: Process in Dependency Order

Instead of:

```rust
// OLD: Process by classification  
for module in inlinable_modules {
    inline_module(module);
}
for module in wrapper_modules {
    create_init_function(module);
}
```

Do:

```rust
// NEW: Process by dependency order
for module in sorted_modules {
    if is_inlinable(module) {
        inline_module(module);
    } else {
        create_init_function(module);
    }
}
```

#### b. Module Classification Usage

The classification step should:

- Still run to identify which modules need wrapping
- Store the classification results for lookup
- NOT determine processing order

```rust
// Classification determines HOW to process
let classification = classify_modules(modules);
let inlinable_set = HashSet::from(classification.inlinable_modules);
let wrapper_set = HashSet::from(classification.wrapper_modules);

// Dependency graph determines WHEN to process
for module in sorted_modules {
    if inlinable_set.contains(module) {
        // Process as inlinable
    } else if wrapper_set.contains(module) {
        // Process as wrapper
    }
}
```

### 3. Benefits

1. **Correctness**: Modules are always processed after their dependencies
2. **Simplicity**: Single pass through modules in correct order
3. **No Reordering Needed**: Eliminates the need for complex statement reordering
4. **Predictable Output**: Bundle order matches dependency order

### 4. Investigation Results: Current Mechanisms

#### Lifted Global Declarations

**Purpose**: Handles Python's `global` statement in wrapped modules.

**How it works**:

- When a wrapped module contains a `global` statement, the bundler creates a true global variable at the bundle's top level
- Example: `global x` in module `foo` becomes `_cribo_foo_x = None` at bundle top level
- The module's init function then references this lifted global

**Assessment**: This mechanism is **orthogonal** to module ordering and should be retained. It solves a different problem (global variable scoping) and doesn't affect module processing order.

#### Hard Dependencies

**Purpose**: Handles cross-module class inheritance where the base class comes from another bundled module.

**How it works**:

- Detects when a class inherits from `module.attr` where `module` is another bundled module
- Example: `class CookieJar(requests.compat.MutableMapping)`
- Currently hoists these imports after the dependency module is initialized
- Rewrites base class references to use the initialized module

**Problem with current approach**:

- Hard dependencies are processed in batches AFTER module processing
- This creates complex "hoisting" logic that tries to patch up ordering issues
- The hoisting happens at different points depending on module types

**Solution with dependency ordering**:

- With proper dependency ordering, modules defining base classes are processed first
- Hard dependency rewriting can happen inline during module processing
- No need for separate hoisting phase - just rewrite references as modules are processed

**Assessment**: The hard dependency **detection** should be retained, but the **hoisting mechanism** should be simplified to inline rewriting once we have dependency-ordered processing.

### 5. Edge Cases to Consider

#### Circular Dependencies - The Real Challenge

**This is likely why the current architecture exists!** The statement reordering was probably introduced to handle circular dependencies between modules. Let's analyze this carefully:

##### Circular Dependencies Are Already Solved!

The current wrapper mechanism already handles circular dependencies correctly:

1. **All modules in a circular group become wrapper modules**
2. **Wrapper modules use init functions with self-initialization flags**
3. **The init functions handle partial initialization**

Example:

```python
def _cribo_init_A(self):
    if self.__initialized__:
        return self
    if self.__initializing__:
        return self  # Break recursion
    self.__initializing__ = True

    # Module A's code here
    from B import foo  # This will call B's init if needed

    def bar():
        return foo() + 1

    self.bar = bar

    self.__initialized__ = True
    return self
```

**Key Insight**: The wrapper mechanism already solves circular dependencies! We just need to ensure these wrapper init functions are defined in dependency order so that when A's init calls B's init, B's init function exists.

##### The Real Issue

The problem isn't circular dependencies themselves - it's that the **two-phase processing** breaks the dependency order. With proper dependency ordering:

1. All init functions in a circular group are defined first
2. Then they can call each other as needed during initialization
3. The self-initialization flags prevent infinite recursion

## Summary

### Current State

- Three conflicting sorting mechanisms fight for control
- Module classification overrides dependency order
- Complex workarounds try to patch the resulting issues

### Proposed State

- Single source of truth: the dependency graph
- Process all modules in dependency order
- Use classification only to determine HOW to process each module

### Implementation Priority

1. **Immediate**: Remove secondary sorting mechanisms
2. **Next**: Process modules in strict dependency order
3. **Cleanup**: Simplify hard dependency handling (no more hoisting)
4. **Future**: Improve dependency graph if any ordering issues remain

### Expected Benefits

- Eliminates forward reference errors
- Simplifies the codebase significantly
- Makes the bundling process predictable and debuggable
- Aligns implementation with the stated design principle: "dependency graph dictates inlining order"

This *sometimes* works but is fragile and creates new problems (like init functions being called before they're defined).

##### Our Proposed Solution: Leverage Existing Sophisticated Mechanisms

**Gemini's Deep Analysis Confirms**: The codebase already has sophisticated circular dependency detection that is **independent of and superior to** global statement reordering:

1. **Module-level cycle detection**: `circular_modules: FxIndexSet<String>` identifies SCCs
2. **Symbol-level dependency graph**: `symbol_dep_graph: SymbolDependencyGraph` for fine-grained analysis
3. **Pre-declaration mechanism**: `circular_predeclarations: FxIndexMap<String, FxIndexMap<String, String>>` stores placeholder declarations
4. **Lazy evaluation**: `@functools.cache` ensures single initialization

For circular modules, the enhanced approach:

- When topological sort encounters an SCC (strongly connected component):
  1. First emit all pre-declarations for symbols in the cycle (e.g., `MyClass = None`)
  2. Then process full bodies of all modules in the cycle
  3. Placeholders are populated at runtime with actual objects
- This breaks the cycle at definition time, not runtime

Example transformation:

```python
@functools.cache
def _cribo_init_A():
    module = types.SimpleNamespace()
    from B import foo  # This becomes: foo = B.foo (after B is initialized)

    def bar():
        return foo() + 1

    module.bar = bar
    return module


@functools.cache
def _cribo_init_B():
    module = types.SimpleNamespace()
    from A import bar  # This becomes: bar = A.bar (after A is initialized)

    def foo():
        return bar() * 2

    module.foo = foo
    return module


# Initialize both (order doesn't matter due to lazy evaluation)
A = _cribo_init_A()
B = _cribo_init_B()
```

##### Why Dependency Order Still Works

1. **Non-circular modules**: Process in strict dependency order
2. **Circular module groups**:
   - All modules in the cycle are wrapped (become init functions)
   - Init functions are defined together as a group
   - Initialization calls happen after all definitions
   - Lazy evaluation via `@functools.cache` handles mutual dependencies

##### The Real Problem with Current Approach

The current statement reordering is too aggressive - it reorders EVERYTHING, even when not needed:

- Breaks module boundaries unnecessarily
- Separates related statements (like init + assignment)
- Applies circular dependency solution to non-circular code

Our proposal: **Use targeted solutions for circular dependencies, not global reordering**

#### Cross-Module Symbol Dependencies (Class Inheritance)

Another reason for statement reordering might be cross-module class inheritance:

```python
# Module A
class Base:
    pass


# Module B
from A import Base


class Derived(Base):
    pass
```

**Solution**: This is already handled correctly by dependency ordering!

- Module A is processed before Module B (due to import dependency)
- So `Base` is defined before `Derived` tries to inherit from it
- No reordering needed if we respect the dependency graph

The only edge case is when there's a circular dependency with inheritance, but that's a Python anti-pattern that would fail in normal Python too.

#### Entry Module

The entry module is always last in topological order (it depends on everything else), so it naturally gets processed last, which is correct.

#### Namespaces

Namespace requirements should still be collected during module processing and generated at the appropriate points.

## Migration Path (Updated After Investigation)

### Phase 1: ✅ Investigation Complete

**Findings**:

- `lifted_global_declarations`: Handles Python's `global` statement - **KEEP** (orthogonal to ordering)
- `hard_dependencies`: Cross-module inheritance - **SIMPLIFY** (inline rewriting instead of hoisting)
- Circular dependency handling: Already independent via init functions - **CONFIRMED**

### Phase 2: Core Implementation

1. **Implement new processing loop** based on topological sort
2. **Handle SCCs specially**:
   - Emit pre-declarations first
   - Then process module bodies
3. **Simplify hard dependency handling**:
   - Keep detection logic
   - Replace hoisting with inline rewriting during module processing
4. **Remove two-phase processing** (inlined/wrapped separation)

### Phase 3: Cleanup

1. **Remove `reorder_cross_module_statements`** function entirely
2. **Keep `lifted_global_declarations`** (serves different purpose)
3. **Simplify hard dependency hoisting** to inline rewriting
4. **Remove statement batching and categorization** logic

### Gemini's Specific Implementation Steps:

```rust
// When processing modules in dependency order:
for component in topological_sort_with_sccs(module_graph) {
    match component {
        Single(module) => {
            // Process single module based on its type
            if is_inlinable(module) {
                inline_module(module);
            } else {
                create_init_function(module);
            }
        }
        Cycle(modules) => {
            // First: Emit pre-declarations for all symbols in cycle
            for module in &modules {
                emit_pre_declarations(module);  // e.g., MyClass = None
            }
            
            // Second: Process all module bodies
            for module in &modules {
                if is_inlinable(module) {
                    inline_module(module);
                } else {
                    create_init_function(module);
                }
            }
        }
    }
}
```

## Testing Strategy

1. **Unit Tests**: Verify module processing order matches dependency order
2. **Integration Tests**:
   - Test cases with inlined modules depending on wrapper modules
   - Test cases with wrapper modules depending on inlined modules
   - Test circular dependency scenarios
3. **Regression Tests**: Ensure all existing tests pass

## Success Criteria

1. No `NameError` exceptions for init functions
2. All modules can access their dependencies
3. Circular dependency handling still works
4. Bundle output is deterministic and follows dependency order
5. Removal of complex reordering logic

## Risks and Mitigations

### Risk 1: Performance Impact

- **Mitigation**: Single pass is actually more efficient than current multi-phase approach

### Risk 2: Unexpected Dependencies

- **Mitigation**: The dependency graph is already thoroughly tested and used for cycle detection

### Risk 3: Breaking Existing Tests

- **Mitigation**: Most tests should pass as-is; snapshot tests may need updating for different statement order

## Why The Current Architecture Exists (And Why We Can Do Better)

The current statement reordering architecture likely evolved to handle:

1. **Circular dependencies between modules** - The "big hammer" solution of reordering ALL statements
2. **Cross-module symbol dependencies** - Classes inheriting from other modules
3. **Forward references** - Functions calling other functions defined later

However, this approach is:

- **Over-engineered**: Applies a global solution to local problems
- **Fragile**: Creates new ordering issues while solving others
- **Complex**: Hard to maintain and reason about

## Our Better Approach

1. **For non-circular modules** (90% of cases): Simple dependency ordering works perfectly
2. **For circular modules** (10% of cases): Already handled by wrapping in init functions with lazy evaluation
3. **For symbol dependencies**: The import graph already captures these dependencies

The key insight: **We don't need global statement reordering. We need:**

- **Correct module ordering** (from dependency graph)
- **Targeted circular dependency handling** (init functions with `@functools.cache`)
- **Respect for module boundaries** (don't mix statements from different modules)

## External Validation by Gemini AI

This design was reviewed by Gemini's deep code analysis system with the following results:

### Key Validations (95% Confidence):

1. **Architectural Dissonance Confirmed**: The two-phase processing conflicts with existing sophisticated mechanisms
2. **Circular Dependencies Already Handled**: Pre-declaration mechanism is independent and superior to statement reordering
3. **Hypothesis Strongly Supported**: Dependency-ordered processing won't break circular handling

### Additional Discoveries:

- `lifted_global_declarations` likely solves forward references, not circular dependencies
- This mechanism becomes unnecessary with proper dependency ordering
- The existing circular dependency system is more robust than initially understood

### Gemini's Conclusion:

> "The hypothesis that removing global statement reordering and using dependency-ordered processing won't break circular dependency handling is strongly supported. The codebase contains a dedicated, advanced mechanism that is superior to and largely independent of simple statement reordering."

## Conclusion

This change fixes a fundamental architectural issue where we were ignoring the carefully computed dependency graph in favor of a type-based batching approach. By respecting the dependency order throughout the bundling process, we ensure correctness, simplify the code, and eliminate the need for complex post-processing reordering.

The statement reordering was likely introduced to solve circular dependencies, but it's a sledgehammer solution. We already have a scalpel - the circular dependency detection and init function wrapping. Let's use the right tool for the job.

The key insight is: **Module classification determines HOW to process, dependency order determines WHEN to process.**

## Implementation Notes

### Understanding "Deferred Imports" (Actually Deferred Assignments)

Even with dependency-ordered processing, "deferred imports" are still necessary. The name is misleading - they're actually **deferred assignments from import transformations**.

**What really happens**:

1. Import statements get transformed into assignments:
   - `from .messages import message` → `message = greetings_messages.message`
   - `import utils` → `utils = types.SimpleNamespace()`
2. These assignments can't execute immediately if they reference namespace objects that don't exist yet
3. So the assignments are "deferred" - collected and placed after namespace creation

**Why still needed with dependency ordering**:

- Module code is inlined as soon as the module is processed
- But namespace objects for modules are created later in a separate phase
- The transformed import assignments must wait for their target namespaces to exist

**Concrete example**:

```python
# Original in greetings/greeting.py:
from .messages import message

# Gets transformed to:
message = greetings_messages.message  # Can't run yet - greetings_messages doesn't exist!

# Final bundle structure:
# 1. Module code inlined (without the import)
# 2. ... more processing ...
# 3. greetings_messages = types.SimpleNamespace()  # Namespace created
# 4. message = greetings_messages.message  # Deferred assignment placed here
```

**The misnomer**: They're called "deferred imports" but the import transformation already happened. What's deferred is the ASSIGNMENT that resulted from transforming the import. A better name would be "deferred import assignments" or "namespace-dependent assignments".

### Alternative: Eager Namespace Creation

The current need for deferral exists because namespaces are created "on demand" based on import patterns. But there's a simpler alternative:

**Create ALL namespace objects eagerly when processing each module:**

- When processing module `foo.bar`, immediately create `foo_bar = types.SimpleNamespace()`
- No need to track which modules "need" namespaces
- No need to defer assignments - namespaces always exist when referenced
- Trade-off: A few extra `SimpleNamespace()` objects vs. significant complexity reduction

This would eliminate:

- The entire deferred imports mechanism
- Complex namespace requirement tracking
- Multiple phases of namespace creation
- Timing issues between module inlining and namespace creation

The current architecture optimizes for minimal namespace creation, but `types.SimpleNamespace()` objects are cheap. The complexity cost seems higher than the benefit of avoiding a few namespace objects.

## Partial Module Initialization for Circular Dependencies

### The Problem

When modules have circular dependencies (e.g., `foo/__init__.py` imports from `foo.boo`, and `foo.boo` needs `foo` to be initialized), we need to support partial initialization. Python handles this by allowing modules to be partially initialized - when a circular import is detected, the importing module gets access to whatever has been initialized so far.

Our initial approach had modules setting `__initializing__` flags on OTHER modules, which is wrong. Python modules control their own initialization state internally.

### The Solution: Self-Controlled Partial Initialization (REVISED)

Each module controls its OWN `__initializing__` flag, but the strategy has been refined based on implementation:

1. **Set `__initializing__ = True`** right BEFORE statements that were transformed from imports (specifically init calls)
2. **When detecting recursion** (i.e., `__initializing__` is already True), set it to `False` to allow the next call to proceed further, then return the partial module
3. **Continue initialization** after import calls complete

**IMPORTANT LESSON LEARNED**: Parent modules should NOT call child module init functions. In Python, the import machinery ensures parent initialization happens before child modules, but this happens OUTSIDE the module's own code. Child modules don't explicitly initialize their parents - that would create artificial circular dependencies.

#### Example Flow (CORRECTED)

```python
def _cribo_init_foo():
    if foo.__initialized__:
        return foo
    if foo.__initializing__:
        foo.__initializing__ = False  # Allow next call to proceed further
        return foo  # Return partial module

    # Some initialization code...
    print("Initializing foo package")

    # NOTE: We do NOT call child module init functions here!
    # In Python, parent modules don't initialize their children.
    # The import machinery handles parent initialization when a child is imported.

    # If we have "from .boo import helper_function" in the original code:
    # This gets transformed to just accessing the already-initialized boo module
    helper_function = foo_boo.helper_function  # boo was initialized by import machinery

    # Set our attributes
    foo.helper_function = helper_function

    # Finish initialization
    foo.__initialized__ = True
    foo.__initializing__ = False
    return foo


def _cribo_init_foo_boo():
    if foo_boo.__initialized__:
        return foo_boo
    if foo_boo.__initializing__:
        foo_boo.__initializing__ = False
        return foo_boo

    # The import machinery would have initialized parent first
    # We don't explicitly call parent init from child modules

    print("Initializing foo.boo module")

    # Module code here...

    foo_boo.__initialized__ = True
    foo_boo.__initializing__ = False
    return foo_boo
```

### Why This Works

1. **First call**: Module starts initializing, sets `__initializing__ = True` before imports
2. **Circular call**: If called again while initializing, it sees `__initializing__ = True`, sets it to `False`, and returns partial module
3. **Continuation**: After the circular call returns, the original initialization continues
4. **Next circular call**: Since `__initializing__` was set to `False`, it can proceed a bit further

This allows incremental partial initialization, matching Python's behavior where modules are populated with attributes as initialization progresses.

### Implementation Requirements

1. **No external flag setting**: Modules only set their OWN `__initializing__` flag
2. **Strategic flag placement**: Set `__initializing__ = True` only before statements that were transformed from imports
3. **Reset on recursion**: When detecting recursion, set `__initializing__ = False` before returning
4. **Immediate attribute setting**: Set module attributes immediately after definitions (already implemented)

### Code Changes Needed

In `module_transformer.rs`:

- Set `__initializing__ = True` only before transformed import statements (init calls)
- When checking `__initializing__`, set it to `False` before returning
- Remove any code that sets flags on other modules

## Module Variable Naming and Transformation (CRITICAL LESSON)

### The Problem: Hardcoded MODULE_VAR

During implementation, we discovered a critical issue: the bundler was using a hardcoded constant `MODULE_VAR = "_cribo_module"` throughout the codebase for module variable references. This caused failures when:

1. **`locals()` transformation**: Was being transformed to `vars(_cribo_module)` but `_cribo_module` didn't exist
2. **`globals()` transformation**: Was being transformed to `_cribo_module.__dict__` with same issue
3. **Module attribute assignments**: Generated code like `_cribo_module.MyClass = MyClass` for wrapper modules

### The Solution: Module-Specific Variable Names

Each module needs its own sanitized variable name based on its actual module name:

- `foo` → `foo`
- `foo.bar` → `foo_bar`
- `my.package` → `my_package`

### Implementation Changes Required

1. **Pass module variable name through transformation chain**: Every transformation function that deals with module-level variables must receive the actual module variable name as a parameter

2. **Remove hardcoded constants**: Eliminated `MODULE_VAR` constants from:
   - `module_transformer.rs`
   - `import_transformer.rs`
   - `globals.rs`

3. **Use `sanitize_module_name_for_identifier`**: Consistently use this function to get the correct module variable name

### Key Code Patterns

```rust
// WRONG - using hardcoded constant
const MODULE_VAR: &str = "_cribo_module";
transform_locals_in_stmt(stmt, "locals", false, None);

// CORRECT - passing actual module variable
let module_var_name = sanitize_module_name_for_identifier(ctx.module_name);
transform_locals_in_stmt(stmt, "locals", false, Some(&module_var_name));
```

### Affected Transformations

1. **`locals()` calls**: Transform to `vars(actual_module_var)`
2. **`globals()` calls**: Transform to `actual_module_var.__dict__`
3. **Module attribute assignments**: Use `actual_module_var.attr = value`
4. **Import transformations in wrapper modules**: Set attributes on correct module variable

## Implementation Results

### What We Actually Did

After deep investigation and implementation, we successfully refactored the bundler to use dependency-ordered processing with eager namespace creation:

1. **Eager Namespace Creation**: Namespaces are now created immediately when processing each module, before the module's code
2. **Dependency-Ordered Processing**: Modules are processed in topological order (with reversal for bundling context)
3. **Smart Import Assignment Placement**: Import assignments are placed immediately when possible, deferred only when necessary for circular dependencies
4. **Removed Unnecessary Complexity**: Eliminated `reorder_cross_module_statements` and related categorization code

### Current Status

Most tests are passing with the new implementation. The remaining issue is with wrapper modules (modules with side effects) that generate init functions. These need special handling because:

- Init functions are generated after all inlinable modules are processed
- References to these functions (like `pkg.module = _cribo_init_...()`) happen earlier
- This creates a forward reference issue

## The Wrapper Module Forward Reference Problem

### Problem Description

Wrapper modules (modules with side effects) are transformed into init functions to preserve Python's module initialization semantics. However, in our dependency-ordered processing, we encounter a forward reference issue.

#### Python's Parent-Child Module Semantics

In Python, there's an implicit dependency relationship:

- When importing `foo.bar`, Python first imports `foo` (runs `foo/__init__.py`)
- Therefore, `foo.bar` implicitly depends on `foo`
- Parents are initialized before children

However, this creates circular dependencies when:

- Parent `__init__.py` imports FROM its submodules (e.g., `from .bar import something`)
- Now `foo` explicitly depends on `foo.bar`
- Combined with the implicit dependency, we have: `foo` ↔ `foo.bar`

#### The Current Problem

The bundler currently handles this incorrectly by:

1. Processing all inlinable modules first (including parent `pkg/__init__.py`)
2. Deferring ALL wrapper modules to a separate phase
3. When `pkg/__init__.py` imports from `.pretty`, it generates: `pkg.pretty = _cribo_init_pkg_pretty()`
4. But the init function doesn't exist yet - it's deferred!

```python
# Current problematic output:
pkg.pretty = _cribo_init_pkg_pretty()  # NameError! Function not defined yet

# ... many lines later ...


@functools.cache
def _cribo_init_pkg_pretty():
    # Module initialization code
    pass
```

The root cause is **treating wrapper modules as a special category** that gets processed in a separate phase, rather than processing ALL modules (wrapper or not) in dependency order.

### Solution Analysis

#### Option 1: Pre-Declaration with None

```python
# At the top of the bundle
_cribo_init_pkg_pretty = None

# Later, when needed
pkg.pretty = _cribo_init_pkg_pretty()  # Still fails! Can't call None


# Much later
@functools.cache
def _cribo_init_pkg_pretty():
    pass
```

**Issues**:

- Doesn't actually solve the problem - can't call `None`
- Would need complex deferred assignment mechanism
- Adds complexity without real benefit

#### Option 2: Pre-Declaration with Lambda

```python
# Forward declaration
_cribo_init_pkg_pretty = lambda: _cribo_init_pkg_pretty_impl()

# Usage works immediately
pkg.pretty = _cribo_init_pkg_pretty()


# Later, define the real implementation
@functools.cache
def _cribo_init_pkg_pretty_impl():
    pass
```

**Issues**:

- Adds unnecessary indirection and runtime overhead
- Breaks `@functools.cache` optimization (caches the lambda, not the impl)
- Complicates debugging and stack traces

#### Option 3: Generate Init Functions During Processing

Instead of deferring init function generation, generate them immediately when processing each wrapper module in dependency order:

```python
# When processing pkg.pretty (a wrapper module):
@functools.cache
def _cribo_init_pkg_pretty():
    # Module code here
    pass


# Later, when processing pkg:
pkg.pretty = _cribo_init_pkg_pretty()  # Function already exists!
```

**Benefits**:

- No forward references
- Maintains single-pass processing
- Aligns with dependency-ordered architecture
- No runtime overhead or indirection

#### Option 4: Two-Phase Approach

1. First pass: Generate all init function definitions
2. Second pass: Generate all init function calls

**Issues**:

- Violates single-pass processing principle
- Adds complexity to track what needs to be called where
- Harder to maintain and debug

### Recommended Solution: Option 3 - Generate During Processing

**This is the most technically correct solution** because:

1. **Maintains Invariants**: Respects the fundamental principle that definitions must precede usage
2. **Aligns with Architecture**: Fits naturally with dependency-ordered processing
3. **Zero Runtime Overhead**: No lambdas, no indirection, no extra function calls
4. **Simplicity**: Single-pass processing, no complex state tracking
5. **Debuggability**: Clean stack traces, straightforward execution flow
6. **Performance**: Preserves `@functools.cache` optimization

### Implementation Plan

1. Modify the processing loop to handle wrapper modules immediately:
   ```python
   for module in dependency_order:
       if module.is_wrapper:
           generate_init_function(module)  # Generate NOW, not later
           if parent_needs_reference:
               add_assignment(f"{parent}.{child} = {init_func}()")
       else:
           inline_module(module)
   ```

2. Remove the separate "wrapper module processing" phase that happens after inlining

3. Ensure init functions are generated in dependency order (leaves before parents)

This solution eliminates forward references while maintaining the architectural improvements we've already made.

## Parent-Child Module Initialization: Partial Initialization

### The Problem

When a package `__init__.py` imports from its submodules, we get circular dependencies:

- `foo/__init__.py` imports from `foo.boo`
- `foo.boo` needs `foo` to exist as its parent package
- This creates a circular dependency

### Python's Actual Behavior

Python handles this through **partial initialization**:

1. When importing `foo`, Python creates a module object and immediately adds it to `sys.modules`
2. The module exists but is empty (partially initialized)
3. As `foo/__init__.py` executes, it populates the module object
4. If during execution it imports `foo.boo`, and `foo.boo` needs `foo`, it gets the partial module
5. This allows circular dependencies to work

**Key insight**: Modules exist immediately but get populated gradually. There's no "all or nothing" initialization.

### The Correct Solution: Three-State Partial Initialization with Self Parameter

#### The Critical Forward Reference Problem

The fundamental challenge in bundling Python modules with circular dependencies is **forward references**:

- Parent packages often import from their children (e.g., `foo/__init__.py` imports from `foo.boo`)
- Children always need their parent initialized first (Python semantics)
- This creates an impossible ordering: parent needs child's init function, but child is defined after parent

#### How Self Parameter Improves the Design

By passing the module as a `self` parameter to its init function, we make the functions more generic and maintainable:

```python
# Parent module defined first (natural dependency order)
foo = SimpleNamespace(__name__='foo', ...)
def _cribo_init_foo(self):  # Takes 'self' parameter
    if self.__initialized__:
        return self
    if self.__initializing__:
        return self  # Return partial module
    
    self.__initializing__ = True
    
    # Can call foo.boo.__init__(foo.boo) even though foo.boo doesn't exist yet!
    # This works because attribute lookup happens at CALL time, not definition time
    foo.boo.__init__(foo.boo)  # No forward reference - lazy attribute lookup
    helper = foo.boo.helper_function
    self.helper = helper  # Use 'self' for our own attributes
    
    self.__initialized__ = True
    self.__initializing__ = False
    return self
    
foo.__init__ = _cribo_init_foo  # Attach to module

# Child module defined later (when needed)
foo.boo = SimpleNamespace(__name__='foo.boo', ...)  # Attached to existing parent
def _cribo_init_foo_boo(self):  # Takes 'self' parameter
    if self.__initialized__:
        return self
    if self.__initializing__:
        return self
    
    self.__initializing__ = True
    
    # Parent initialization (if needed)
    foo.__init__(foo)  # Parent already exists and has __init__ attached
    
    # Initialize our own module using 'self'
    self.helper_function = lambda x: f"Helper: {x}"
    
    self.__initialized__ = True
    self.__initializing__ = False
    return self
    
foo.boo.__init__ = _cribo_init_foo_boo  # Attach to module

# Grandchild even later
foo.boo.zoo = SimpleNamespace(__name__='foo.boo.zoo', ...)
def _cribo_init_foo_boo_zoo(self):  # Takes 'self' parameter
    if self.__initialized__:
        return self
    if self.__initializing__:
        return self
    
    self.__initializing__ = True
    
    # Parent initialization
    foo.boo.__init__(foo.boo)  # Parent exists
    
    # Own initialization using 'self'
    class Zoo:
        def format(self, value):
            return f"[{value}]"
    self.Zoo = Zoo  # Use 'self' for our attributes
    
    self.__initialized__ = True
    self.__initializing__ = False
    return self
    
foo.boo.zoo.__init__ = _cribo_init_foo_boo_zoo
```

**Key Benefits of Self Parameter**:

1. **Generic Functions**: Init functions don't need to hardcode the module name for their own attributes
2. **Clear Separation**: Use `self` for own module, global names for cross-module references
3. **Maintainable**: Easier to refactor and rename modules
4. **Explicit Contract**: Clear that we're passing the module to be initialized
5. **No forward references**: Attribute lookup still happens at call time

Instead of using `@functools.cache` (which makes initialization all-or-nothing), we:

1. Track three states: not started, initializing, initialized
2. Store init functions as module attributes for unified access
3. Allow partial module access during circular imports
4. **Eliminate ALL forward references through attribute-based access**

```python
# Parent module - created just before its init function
foo = types.SimpleNamespace(__name__="foo", __path__=[], __initializing__=False, __initialized__=False)


def _cribo_init_foo(self):  # Takes 'self' parameter
    if self.__initialized__:
        return self

    if self.__initializing__:
        return self  # Return partial module

    self.__initializing__ = True

    # Execute __init__.py code
    print("Initializing foo package")

    # Import from submodule - foo.boo will exist by the time this is called
    foo.boo.__init__(foo.boo)  # Pass the module explicitly
    helper_function = foo.boo.helper_function
    self.helper_function = helper_function  # Use 'self' for our attributes

    def package_level_function(x):
        return helper_function(x) + " (from package)"

    self.package_level_function = package_level_function  # Use 'self'

    self.__initialized__ = True
    self.__initializing__ = False

    return self


foo.__init__ = _cribo_init_foo  # Attach immediately after definition

# Child module - created when needed, attached to parent
foo.boo = types.SimpleNamespace(__name__="foo.boo", __initializing__=False, __initialized__=False)


def _cribo_init_foo_boo(self):  # Takes 'self' parameter
    if self.__initialized__:
        return self

    if self.__initializing__:
        return self

    self.__initializing__ = True

    # Initialize parent first (if needed)
    foo.__init__(foo)  # Pass parent module explicitly

    print("Initializing foo.boo module")

    # Import from sibling
    foo.zoo.__init__(foo.zoo)  # Pass sibling module explicitly
    Zoo = foo.zoo.Zoo
    self.Zoo = Zoo  # Use 'self' for our attributes

    def helper_function(x):
        zoo = Zoo()
        return zoo.format(x)

    self.helper_function = helper_function  # Use 'self'

    self.__initialized__ = True
    self.__initializing__ = False

    return self


foo.boo.__init__ = _cribo_init_foo_boo  # Attach immediately

# Grandchild module - created even later
foo.zoo = types.SimpleNamespace(__name__="foo.zoo", __initializing__=False, __initialized__=False)


def _cribo_init_foo_zoo(self):  # Takes 'self' parameter
    if self.__initialized__:
        return self

    if self.__initializing__:
        return self

    self.__initializing__ = True

    # Initialize parent (if needed)
    foo.__init__(foo)  # Pass parent module explicitly

    print("Initializing foo.zoo module")

    class Zoo:
        def format(self, value):
            return f"[{value}]"

    self.Zoo = Zoo  # Use 'self' for our attributes

    self.__initialized__ = True
    self.__initializing__ = False

    return self


foo.zoo.__init__ = _cribo_init_foo_zoo  # Attach immediately
```

### Key Benefits of Unified Access

1. **Eliminates forward references**: Parent can call child init functions defined later
2. **Consistent pattern**: All init functions accessed through `module.__init__()`
3. **Natural hierarchy**: `foo.__init__()`, `foo.boo.__init__()`, `foo.zoo.__init__()`
4. **No separate naming scheme**: No need to track `_cribo_init_foo` separately
5. **Module and init together**: Everything about a module is on the module object
6. **Dependency order preserved**: Children defined after parents, but callable from parents

### Why Three States?

1. **`__initialized__`**: Distinguishes between "never run" and "completed"
2. **`__initializing__`**: Prevents infinite recursion while allowing partial access

During circular imports:

- If A imports B and B imports A
- A starts initializing, sets `__initializing__ = True`
- A imports B, which starts B's initialization
- B imports A, calls `_cribo_init_A()`
- `_cribo_init_A()` sees `__initializing__ = True`, returns partial A
- B continues with partial A (can access any attributes A has set so far)
- B completes, returns to A
- A continues and completes

This exactly matches Python's behavior with `sys.modules`.

### Key Implementation Points (REVISED)

1. **Module creation pattern** (CRITICAL):
   - Each module object created RIGHT BEFORE its init function
   - Init function attached to module IMMEDIATELY after definition
   - Child modules attached to parent objects when created
   - Natural dependency order: parents before children
2. **Lazy attribute lookup**: `foo.boo.__init__()` doesn't validate `foo.boo` exists at definition time
3. **No forward references**: Access init functions through `module.__init__()`, not by name
4. **Global module objects**: All wrapper modules at global scope
5. **No @functools.cache**: We manually track initialization state
6. **Three-state tracking**:
   - `__initializing__`: Currently executing module code
   - `__initialized__`: Module fully populated
7. **Parent initialization in import machinery** (CRITICAL CHANGE):
   - Parent initialization happens in the IMPORT TRANSFORMATION, not in child init functions
   - When transforming `import foo.bar`, generate:
     ```python
     foo.__init__()  # Initialize parent
     foo.bar.__init__()  # Then child
     ```
   - Child modules DO NOT call parent init internally - this would create artificial circular dependencies
8. **Partial access allowed**: Circular imports see partially-populated modules
9. **Flags set at strategic times**:
   - `__initializing__ = True` ONLY before statements that were transformed from imports
   - `__initialized__ = True` at end (after all code)
   - `__initializing__ = False` when returning partial module
10. **Idempotent init functions**: Safe to call multiple times, only initialize once

### Benefits Over Previous Approaches

1. **Matches Python exactly**: Modules can be partially initialized
2. **No artificial constructs**: No fake "namespace" wrappers
3. **Handles all circular patterns**: Works for any dependency cycle
4. **Simple and clear**: Just a flag check, no complex caching

### Import Transformation Rules

1. **Import from submodule** (`from foo.boo import X`):
   ```python
   foo.__init__(foo)  # Initialize parent first with self
   foo.boo.__init__(foo.boo)  # Then child with self
   X = foo.boo.X
   ```

2. **Import from package** (`from foo import X`):
   ```python
   foo.__init__(foo)  # Pass module as self
   X = foo.X
   ```

3. **Import submodule as namespace** (`import foo.boo`):
   ```python
   foo.__init__(foo)  # Parent first with self
   foo.boo.__init__(foo.boo)  # Child with self
   ```

### Example Execution Flow

For the test case where:

- `main.py`: `from foo.boo import process_data`
- `foo/__init__.py`: `from .boo import helper_function`
- `foo/boo.py`: `from .zoo import Zoo`

Execution order:

1. Module objects `foo`, `foo.boo`, `foo.zoo` created with `__init__` attributes
2. `main` calls `foo.__init__(foo)` # Pass foo as self
3. → `foo.__initializing__ = True` (via self)
4. → `foo` calls `foo.boo.__init__(foo.boo)` # Pass foo.boo as self
5. → → `foo.boo.__initializing__ = True` (via self)
6. → → `foo.boo` calls `foo.__init__(foo)` # Pass foo as self
7. → → → Returns partial `foo` immediately (`__initializing__` is `True`)
8. → → `foo.boo` calls `foo.zoo.__init__(foo.zoo)` # Pass foo.zoo as self
9. → → → `foo.zoo` fully initializes (using self for attributes)
10. → → `foo.boo` populates attributes (using self)
11. → → `foo.boo.__initialized__ = True`, `__initializing__ = False` (via self)
12. → `foo` continues, gets `helper_function` from now-complete `foo.boo`
13. → `foo` populates remaining attributes (using self)
14. → `foo.__initialized__ = True`, `__initializing__ = False` (via self)
15. `main` calls `foo.boo.__init__(foo.boo)` # Pass foo.boo as self
16. → Returns immediately (`__initialized__` is `True`)
17. `main` gets `process_data` from `foo.boo`

### Critical Insight: Partial Module Access

In step 7, when `foo.boo` calls `_cribo_init_foo()` during circular import:

- `foo` is **partially populated** (some attributes may exist)
- `foo.boo` can access whatever attributes `foo` has set before importing `.boo`
- This is exactly how Python handles circular imports

The three-state system ensures:

- No infinite recursion (`__initializing__` check)
- Partial modules are accessible (return immediately with current state)
- Full initialization happens exactly once (`__initialized__` check)

## Bundled Output Structure

The final bundled Python file outputs ALL modules in strict dependency graph order, mixing inlined and wrapper modules as needed.

### Module Output Order (CRITICAL)

**ALL modules are output in dependency graph order**, regardless of whether they're wrapper or inlined. This ensures:

- Any module can reference modules that come before it
- Inlined modules can call wrapper module init functions
- Wrapper module init functions exist before any code tries to call them

### Example Mixed Output

```python
# 1. Pure utility module (inlined)
def pure_utility():
    return 42


# 2. Wrapper module that uses the utility
foo = types.SimpleNamespace(__name__="foo", __path__=[], __initializing__=False, __initialized__=False)


def _cribo_init_foo():
    if foo.__initialized__:
        return foo
    if foo.__initializing__:
        return foo
    foo.__initializing__ = True

    # Can use pure_utility - it's already defined
    result = pure_utility()
    foo.result = result

    foo.__initialized__ = True
    foo.__initializing__ = False
    return foo


foo.__init__ = _cribo_init_foo

# 3. Inlined module that imports from wrapper
# Original: from foo import result
foo.__init__()  # Call wrapper init HERE, where import was
result = foo.result


def process_with_result():
    return result * 2


# 4. Child wrapper module
foo.boo = types.SimpleNamespace(__name__="foo.boo", __initializing__=False, __initialized__=False)


def _cribo_init_foo_boo():
    if foo.boo.__initialized__:
        return foo.boo
    if foo.boo.__initializing__:
        return foo.boo
    foo.boo.__initializing__ = True

    foo.__init__()  # Parent init

    # Can use process_with_result - it's already defined
    foo.boo.processed = process_with_result()

    foo.boo.__initialized__ = True
    foo.boo.__initializing__ = False
    return foo.boo


foo.boo.__init__ = _cribo_init_foo_boo

# 5. Another inlined module
# Original: from foo.boo import processed
foo.boo.__init__()  # Call wrapper init HERE
processed = foo.boo.processed
```

### Entry Point with Import Replacements

The entry module's code with imports replaced by init calls AT THE EXACT SCOPE where imports were:

```python
# main.py
print("Starting main")

# Where "from foo.boo import process_data" was:
foo.__init__(foo)  # Initialize parent with self
foo.boo.__init__(foo.boo)  # Initialize child with self
process_data = foo.boo.process_data

# Where "from foo import boo" was:
foo.__init__(foo)  # Initialize parent with self
boo = foo.boo  # Just reference the module (already initialized as child of foo)

# Where "from foo import helper" was:
foo.__init__(foo)  # Initialize with self
helper = foo.helper

# Rest of main.py code
result = process_data("test")
```

### Key Principles

1. **Strict dependency order**: ALL modules (wrapper and inlined) output in dependency graph order
2. **Mixed module types**: Inlined and wrapper modules interleaved based on dependencies
3. **Inlined modules call wrapper inits**: Where original imports were, preserving exact behavior
4. **Parent init inside child**: Each child's init calls parent's init internally - NOT at import site
5. **Simple import replacement**: Import sites just call the target module's init
6. **Exact scope preservation**: Init calls happen exactly where original imports were
7. **Init functions handle dependencies**: No need to explicitly initialize parents at import sites

## Lessons Learned and Key Insights

### 1. Python Import Semantics Are Subtle

**Key Insight**: Python's import machinery handles parent initialization EXTERNALLY to the module's code. Child modules don't explicitly initialize their parents - the import system does this before the child module's code runs.

**Initial Wrong Approach**: We had child init functions calling parent init functions:

```python
def _cribo_init_foo_boo():
    # WRONG: Child calling parent creates artificial circular dependency
    foo = _cribo_init_foo()  # If foo imports from foo.boo, infinite recursion!
    # ... rest of foo.boo initialization
```

**Why This Creates Problems**: Consider this common pattern:

```python
# foo/__init__.py
from .boo import helper_function  # Parent imports from child

# foo/boo.py
# Just defines helper_function, doesn't import from parent
```

With child-calls-parent approach:

1. Main code calls `_cribo_init_foo_boo()`
2. `foo.boo` calls `_cribo_init_foo()` (child → parent)
3. `foo` tries to import from `foo.boo`, calls `_cribo_init_foo_boo()` (parent → child)
4. **CIRCULAR!** We're back at step 2, infinite recursion

**Correct Approach**: Parent initialization happens at the IMPORT SITE, not inside child:

```python
def _cribo_init_foo_boo():
    # NO parent call here - child is self-contained
    # ... foo.boo initialization only
    
# At the import site (e.g., in main):
# When we see "import foo.boo", we generate:
foo = _cribo_init_foo()      # Initialize parent FIRST
foo_boo = _cribo_init_foo_boo()  # Then child
```

**Why This Works**: The initialization order is controlled by the CALLER (import site), not by the child module:

1. Import site calls `_cribo_init_foo()` first
2. `foo` tries to import from `foo.boo` - but this just accesses the not-yet-initialized `foo_boo` variable
3. `foo` completes with partial data (or defers the access)
4. Import site then calls `_cribo_init_foo_boo()`
5. No circular calls because child doesn't call parent

**The Critical Difference**:

- **Wrong**: Child modules have hardcoded knowledge that they must initialize their parent (creates bidirectional dependency)
- **Right**: Import machinery (external to both modules) handles the parent-child initialization order (unidirectional flow)

This matches Python's actual behavior where the import system, not the module code itself, ensures parents are initialized before children.

**Real-World Example - The `parent_child_circular` Test Case**:

Original Python code:

```python
# foo/__init__.py
from .boo import helper_function  # Parent needs child

# foo/boo.py
from .zoo import Zoo  # Child needs sibling, NOT parent

# main.py
from foo.boo import process_data  # Import child directly
```

With WRONG approach (child calls parent):

```python
# main.py calls:
_cribo_init_foo_boo()
  → _cribo_init_foo()  # Child calls parent
    → _cribo_init_foo_boo()  # Parent needs child's helper_function
      → _cribo_init_foo()  # Child calls parent again
        → INFINITE RECURSION!
```

With CORRECT approach (import site handles order):

```python
# main.py transformation for "from foo.boo import process_data":
_cribo_init_foo()  # Import machinery calls parent first
  → foo needs boo.helper_function but boo not initialized yet
  → foo.__initializing__ = True (before the import statement)
  → Returns partial foo (with __initializing__ flag set)
  
_cribo_init_foo_boo()  # Import machinery then calls child
  → foo.boo initializes successfully
  → Returns complete foo.boo

# If foo is called again later, it can complete initialization
# because foo.boo now exists
```

The key is that `foo.boo` NEVER calls `_cribo_init_foo()` itself - that would create an artificial dependency that doesn't exist in the original Python code.

### 2. Module Variable Names Must Be Dynamic

**Key Insight**: Using a hardcoded `_cribo_module` variable for all modules breaks when multiple modules need different variable names.

**Implementation Impact**: All transformation functions must receive and use the actual module variable name, computed via `sanitize_module_name_for_identifier()`.

### 3. Partial Initialization Still Required for True Circular Dependencies

**Key Insight**: The `__initializing__` flag is still necessary for GENUINE circular dependencies in the original Python code, not the artificial ones we were creating with parent-child initialization.

**When `__initializing__` is needed**:

```python
# True circular dependency between siblings or unrelated modules:
# module_a.py
from module_b import foo  # A needs B

# module_b.py
from module_a import bar  # B needs A - genuine circle!
```

**When `__initializing__` is NOT needed** (but we were incorrectly creating the need):

```python
# Simple parent-child where only parent imports from child:
# foo/__init__.py
from .boo import helper  # Parent needs child

# foo/boo.py
# No imports from parent - no real circular dependency!
```

**Implementation Impact**:

- Set `__initializing__ = True` only before statements that could cause genuine circular calls
- Most parent-child relationships don't need this protection
- But Python does allow true circular imports, so we must support them
- This flag enables partial module access during genuine circular imports, matching Python's behavior

**Optimization Opportunity**: We could optimize by only adding the `__initializing__` logic to modules that are part of detected circular dependency SCCs (Strongly Connected Components). Modules not in any cycle don't need this protection. However, the current approach of always having the flag is simpler and has negligible performance impact.

### 4. Import Transformation Location Matters

**Key Insight**: The location where import transformations generate init calls is critical. Parent initialization must happen in the import machinery (at the import site), not inside child modules.

**Implementation Impact**:

- Import sites generate parent init calls followed by child init calls
- Child init functions are self-contained and don't call parent inits
- This prevents artificial circular dependencies

### 5. Testing Complex Scenarios Is Essential

**Key Insight**: Edge cases like `locals()` transformation, parent-child circular dependencies, and module attribute access patterns revealed fundamental issues with the original design.

**Implementation Impact**: The test fixtures that seemed like edge cases actually exposed core architectural problems that needed fixing.

## Original Implementation Summary

After investigation, the path forward was clear:

### What to Keep

- **Circular dependency detection** and init function wrapping (already works well)
- **Lifted global declarations** for Python's `global` statement (orthogonal concern)
- **Hard dependency detection** for cross-module inheritance (useful metadata)

### What to Change

- **Module processing order**: Use dependency graph's topological sort, not type-based batches
- **Hard dependency handling**: Inline rewriting during processing, not post-hoc hoisting
- **Statement organization**: Remove global reordering, maintain module boundaries

### What to Remove

- **`reorder_cross_module_statements`** function (the root cause of issues)
- **Type-based batching** (process all inlined, then all wrapped)
- **Complex hoisting logic** for hard dependencies

### Expected Benefits

1. **Correct initialization order** - No more `NameError` for init functions
2. **Simpler code** - Remove hundreds of lines of reordering logic
3. **Better maintainability** - Clear, single-pass processing
4. **Deterministic output** - Follows dependency order consistently

The refactoring is not just a fix - it's a simplification that makes the bundler more correct and easier to understand.

## Future Optimization Opportunities

### 1. Selective `__initializing__` Flag Generation

**Current State**: All wrapper modules get the full three-state initialization logic:

```python
def _cribo_init_foo():
    if foo.__initialized__:
        return foo
    if foo.__initializing__:  # May not be needed if foo isn't in a cycle!
        foo.__initializing__ = False
        return foo
    # ... rest of init
```

**Optimization**: Only emit `__initializing__` logic for modules in circular dependency SCCs:

```python
# For non-circular module:
def _cribo_init_simple():
    if simple.__initialized__:
        return simple
    # No __initializing__ check needed!
    # ... rest of init
    simple.__initialized__ = True
    return simple


# For circular module (in SCC):
def _cribo_init_circular():
    if circular.__initialized__:
        return circular
    if circular.__initializing__:  # Only for modules in cycles
        circular.__initializing__ = False
        return circular
    circular.__initializing__ = True  # Only before imports that could cycle
    # ... rest of init
```

**Implementation**:

- The bundler already detects SCCs via `circular_modules: FxIndexSet<String>`
- During init function generation, check if module is in `circular_modules`
- Only emit `__initializing__` logic for those modules
- Could reduce generated code size by ~10-20% for typical projects

### 2. Smarter `__initializing__` Flag Placement

**Current State**: Set `__initializing__ = True` before any transformed import statement

**Optimization**: Only set the flag before imports that could actually lead to cycles:

- Analyze the dependency graph to identify which specific imports are part of cycles
- Only set the flag before those specific imports
- Other imports don't need the protection

### 3. Eliminate Redundant Parent Initialization Calls

**Current State**: Every import site that needs a child module calls both parent and child init:

```python
# For "from foo.bar.baz import something":
foo = _cribo_init_foo()
foo_bar = _cribo_init_foo_bar()
foo_bar_baz = _cribo_init_foo_bar_baz()
something = foo_bar_baz.something
```

**Optimization**: Track initialization state globally and skip redundant calls:

```python
# If we know foo is already initialized from earlier:
foo_bar_baz = _cribo_init_foo_bar_baz()  # Parent inits handled internally if needed
something = foo_bar_baz.something
```

### 4. Inline Simple Init Functions

**Current State**: Every wrapper module gets an init function, even trivial ones

**Optimization**: For simple wrapper modules with no circular dependencies and minimal code, inline the initialization directly:

```python
# Instead of:
def _cribo_init_constants():
    if constants.__initialized__:
        return constants
    constants.PI = 3.14159
    constants.E = 2.71828
    constants.__initialized__ = True
    return constants


constants = _cribo_init_constants()

# Could be:
constants.PI = 3.14159
constants.E = 2.71828
constants.__initialized__ = True
```

### Why These Optimizations Aren't Critical

1. **Runtime overhead is minimal**: The flag checks are just attribute lookups, very fast
2. **Code size impact is small**: Init functions are typically a small fraction of bundle size
3. **Complexity cost**: These optimizations add complexity to the bundler
4. **Correctness first**: The current approach is simple and correct

However, for large projects with hundreds of modules, these optimizations could provide meaningful benefits in both bundle size and initialization performance.

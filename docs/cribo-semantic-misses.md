# Cribo Semantic Analysis Issues

This document identifies areas where Cribo either reimplements functionality already provided by `ruff_python_semantic` or uses the API incorrectly.

## 1. Duplicate ExecutionContext Implementation

### Issue

**File**: `crates/cribo/src/semantic_analysis.rs:8-35`

Cribo defines its own `ExecutionContext` enum that duplicates functionality already provided by `ruff_python_semantic::ExecutionContext`.

```rust
// Cribo's implementation
pub enum ExecutionContext {
    ModuleLevel,
    FunctionBody,
    ClassBody,
    TypeAnnotation,
    TypeCheckingBlock,
}
```

### Ruff Already Provides

- `ruff_python_semantic::ExecutionContext` with `Runtime` and `Typing` variants
- `SemanticModel` methods like `in_typing_context()`, `in_runtime_context()`, `in_type_checking_block()`

### Specific Recommendation

**Option 1**: Use ruff's semantic model flags (requires a properly built semantic model):

```rust
// Instead of: if context == ExecutionContext::TypeCheckingBlock
if semantic_model.in_type_checking_block() { ... }

// Instead of: if context == ExecutionContext::TypeAnnotation  
if semantic_model.in_typing_context() { ... }

// Instead of: if context.requires_runtime()
if semantic_model.in_runtime_context() { ... }
```

**Option 2**: Keep your ExecutionContext for now if:

- You only need basic context tracking
- You're not building a full semantic model
- Your use case is simpler than ruff's full analysis

This is reasonable because Cribo's bundling use case might not need the full complexity of ruff's execution context tracking.

## 2. Manual AST Traversal Instead of Using Ruff's Infrastructure

### Issue

**File**: `crates/cribo/src/semantic_bundler.rs:82-186`

The `SemanticModelBuilder::traverse_and_bind()` method manually traverses the AST and creates bindings, but this is exactly what ruff's semantic model builder already does internally.

### Problems

- Incomplete handling of AST nodes (only handles a few statement types)
- Missing proper scope management (doesn't push/pop scopes for functions/classes)
- Doesn't handle nested scopes correctly
- Misses many binding types (comprehensions, exception handlers, etc.)

### Ruff Already Provides

The `SemanticModel` is designed to be built using ruff's internal builders, not manually populated.

### Specific Recommendation

**Option 1**: Follow pymermaider's limited approach if you only need imports:

```rust
// Like pymermaider, only process imports if that's all you need
pub fn see_imports(&mut self, stmts: &'a [ast::Stmt]) {
    for stmt in stmts {
        match stmt {
            ast::Stmt::Import(import) => {
                // Process regular imports
                for alias in &import.names {
                    let qualified_name = QualifiedName::user_defined(&alias.name);
                    let binding_id = self.semantic.push_binding(
                        alias.range(),
                        BindingKind::Import(Import {
                            qualified_name: Box::new(qualified_name),
                        }),
                        BindingFlags::EXTERNAL,
                    );
                    let name = alias.asname.as_ref().unwrap_or(&alias.name);
                    self.semantic.current_scope_mut().add(name, binding_id);
                }
            }
            ast::Stmt::ImportFrom(import_from) => {
                // Process from imports
            }
            _ => {} // Skip non-import statements
        }
    }
}
```

**Option 2**: Build a full semantic model using the proper visitor pattern:

```rust
use ruff_python_semantic::{SemanticModel, BindingKind, BindingFlags};

// Step 1: Create the semantic model
let semantic = SemanticModel::new(&typing_modules, path, module);

// Step 2: Build it using a visitor pattern similar to ruff's Checker
struct SemanticBuilder<'a> {
    semantic: SemanticModel<'a>,
}

impl<'a> SemanticBuilder<'a> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(func) => {
                // Push scope for the function
                self.semantic.push_scope(ScopeKind::Function(func));
                
                // Visit parameters and body
                for stmt in &func.body {
                    self.visit_stmt(stmt);
                }
                
                // Pop scope and create binding
                let scope_id = self.semantic.scope_id;
                self.semantic.pop_scope();
                
                // Add function binding to parent scope
                let binding_id = self.semantic.push_binding(
                    func.name.range(),
                    BindingKind::FunctionDefinition(scope_id),
                    BindingFlags::empty(),
                );
                self.semantic.current_scope_mut().add(&func.name, binding_id);
            }
            // Handle other statements...
        }
    }
}
```

**Note**: Both approaches require access to internal methods like `push_scope()`, `pop_scope()`, and `current_scope_mut()`. Pymermaider shows this is viable - they successfully use these internal APIs. Your options:

1. **Do what pymermaider does**: Accept using internal APIs and implement your own Checker
2. **Use ruff's `Checker`**: If you need full semantic analysis
3. **Fork/patch ruff**: To expose these methods officially
4. **Implement your own**: If you have very specific needs

## 3. Incorrect Scope Access

### Issue

**File**: `crates/cribo/src/semantic_bundler.rs:76-77, 204-205`

```rust
let scope = self.semantic.global_scope_mut();
scope.add(builtin, binding_id);
// ...
let scope = self.semantic.current_scope_mut();
scope.add(name, binding_id);
```

### Problems

- `global_scope_mut()` and `current_scope_mut()` don't exist in the public API
- Trying to mutate scopes directly bypasses ruff's internal consistency checks
- The semantic model should be built using the proper builder pattern

### BUT: Pymermaider Shows These Methods DO Exist Internally

**From pymermaider/src/checker.rs:38-40**:

```rust
let binding_id = self.semantic.push_builtin();
let scope = self.semantic.global_scope_mut();
scope.add(builtin, binding_id);
```

Pymermaider successfully uses these "non-existent" methods! The issue isn't that they don't exist - it's that they're not in the public API.

### Specific Recommendation

**Option 1**: Do exactly what pymermaider does - implement your own Checker:

```rust
// From pymermaider - this WORKS with internal methods
pub struct Checker<'a> {
    semantic: SemanticModel<'a>,
    // ... other fields
}

impl<'a> Checker<'a> {
    fn bind_builtins(&mut self) {
        for builtin in python_builtins(u8::MAX, false).chain(MAGIC_GLOBALS.iter().copied()) {
            let binding_id = self.semantic.push_builtin();
            let scope = self.semantic.global_scope_mut();
            scope.add(builtin, binding_id);
        }
    }

    fn add_binding(
        &mut self,
        name: &'a str,
        range: TextRange,
        kind: BindingKind<'a>,
        flags: BindingFlags,
    ) {
        let binding_id = self.semantic.push_binding(range, kind, flags);

        // Direct scope access - pymermaider does this
        let scope = &mut self.semantic.scopes[scope_id];
        scope.add(name, binding_id);

        // Handle shadowing
        if let Some(shadowed) = self.semantic.current_scope().get(name) {
            self.semantic.shadowed_bindings.insert(binding_id, shadowed);
        }
    }
}
```

**Option 2**: Use the exact ruff version pymermaider uses:

```toml
ruff_python_semantic = { git = "https://github.com/astral-sh/ruff.git", tag = "0.11.6" }
```

The internal methods ARE available - you just need to import from the right place and accept that you're using internal APIs.

## 4. Reimplemented Symbol Resolution

### Issue

**File**: `crates/cribo/src/semantic_analysis.rs:100-303`

The `SemanticImportVisitor` reimplements name resolution and usage tracking that ruff already provides through:

- `SemanticModel::resolve_name()`
- `SemanticModel::resolve_qualified_name()`
- `Binding::references()`

### Problems

- Incomplete implementation (doesn't handle all expression types)
- Doesn't respect Python's scoping rules properly
- Missing handling of shadowing, global/nonlocal declarations

### Consider Pymermaider's Approach

Pymermaider shows that `resolve_qualified_name()` works well for their use case:

```rust
// From pymermaider/src/class_diagram.rs:194-200
let base_name = match checker.semantic().resolve_qualified_name(base) {
    Some(base_name) => base_name,
    None => {
        let name = checker.locator().slice(base);
        QualifiedName::user_defined(name)
    }
};
```

They only use semantic resolution for specific purposes (resolving base classes, decorators) and fall back to raw text when resolution fails.

### Specific Recommendation

If you have a properly built semantic model, use its resolution methods:

```rust
// To track import usage:
fn analyze_import_usage(semantic: &SemanticModel, name_expr: &ast::ExprName) {
    if let Some(binding_id) = semantic.resolve_name(name_expr) {
        let binding = semantic.binding(binding_id);

        // Check if it's an import
        if let Some(import) = binding.as_any_import() {
            // Track this usage
            let module = import.module_name();
            let context = if semantic.in_typing_context() {
                "typing"
            } else {
                "runtime"
            };
            // Record usage...
        }
    }
}
```

However, you need a properly built semantic model to use these methods correctly.

## 5. Manual Import Tracking

### Issue

**File**: `crates/cribo/src/semantic_bundler.rs:128-179`

Manual handling of imports instead of using ruff's import resolution:

```rust
// Manual import handling
Stmt::Import(import) => {
    for alias in &import.names {
        let module = alias.name.as_str().split('.').next()...
        // Manual tracking
    }
}
```

### Ruff Already Provides

- `BindingKind::Import`, `BindingKind::FromImport`, `BindingKind::SubmoduleImport`
- `Imported` trait with methods like `qualified_name()`, `module_name()`, `member_name()`
- Proper handling of aliased imports

### Pymermaider's Successful Pattern

Pymermaider shows how to properly create import bindings:

```rust
// From pymermaider/src/checker.rs:171-179
let qualified_name = QualifiedName::user_defined(&alias.name);
self.add_binding(
    name,
    alias.identifier(),
    BindingKind::Import(Import {
        qualified_name: Box::new(qualified_name),
    }),
    flags,
);
```

Key insights:

- They use `QualifiedName::user_defined()` for creating qualified names
- They properly handle aliased imports with appropriate flags
- They distinguish between Import, FromImport, and SubmoduleImport

### Specific Recommendation

If you have a semantic model, use the `Imported` trait:

```rust
// Given a binding from the semantic model
if let Some(import) = binding.as_any_import() {
    match import {
        AnyImport::Import(imp) => {
            let module_name = imp.qualified_name().to_string();
            // Handle regular import
        }
        AnyImport::FromImport(imp) => {
            let module = imp.module_name(); // ["foo", "bar"] for "from foo.bar import baz"
            let member = imp.member_name(); // "baz"
            // Handle from import
        }
        AnyImport::SubmoduleImport(imp) => {
            // Handle submodule import
        }
    }
}
```

The key is to use ruff's import binding types with proper qualified names.

## 6. Incorrect BindingKind Usage

### Issue

**File**: `crates/cribo/src/semantic_bundler.rs:100-101, 109-110`

```rust
BindingKind::ClassDefinition(self.semantic.scope_id),
BindingKind::FunctionDefinition(self.semantic.scope_id),
```

### Problems

- Should pass the ScopeId of the *new* scope created by the class/function, not the current scope
- Missing scope creation for classes and functions

### Specific Recommendation

This is an internal implementation detail of ruff. You cannot create these bindings manually. Instead:

```rust
// If using ruff's semantic model, it's already populated correctly
if let BindingKind::FunctionDefinition(scope_id) = &binding.kind {
    // scope_id is the function's internal scope
    let function_scope = &semantic_model.scopes[*scope_id];
    // Analyze function's local bindings...
}

// The correct pattern (as ruff does it):
// 1. Push scope for the function/class
self.semantic.push_scope(ScopeKind::Function(func_def));
// 2. Get the scope ID
let scope_id = self.semantic.scope_id;
// 3. Visit body...
// 4. Pop scope
self.semantic.pop_scope();
// 5. Create binding with the function's scope ID
let binding_id = self.semantic.push_binding(
    range,
    BindingKind::FunctionDefinition(scope_id), // scope_id from step 2
    flags
);
```

## 7. Missing Semantic Model Features

### Not Using

1. **Reference tracking**: Ruff automatically tracks all references to bindings
2. **Shadowing detection**: `Binding::redefines()` and `Scope::shadowed_binding()`
3. **Builtin resolution**: `SemanticModel::resolve_builtin_symbol()`
4. **Type checking detection**: `SemanticModel::in_type_checking_block()`
5. **Forward reference handling**: `SemanticModel::in_forward_reference()`

### Specific Recommendation

Use these features if you have a properly built semantic model:

```rust
// Check if a symbol is used
if binding.is_used() {
    // Get all references
    for ref_id in binding.references() {
        let reference = semantic.reference(ref_id);
        // Analyze usage context...
    }
}

// Check for shadowing
if let Some(shadowed_id) = scope.shadowed_binding(binding_id) {
    let shadowed = semantic.binding(shadowed_id);
    // Handle shadowing...
}

// Resolve builtins
if let Some(builtin_name) = semantic.resolve_builtin_symbol(expr) {
    // Handle builtin usage
}
```

## 8. Incomplete Global/Nonlocal Handling

### Issue

**File**: `crates/cribo/src/semantic_bundler.rs:576-664`

The `GlobalUsageVisitor` manually tracks global declarations but ignores:

- `BindingKind::Global` and `BindingKind::Nonlocal`
- Ruff's built-in global/nonlocal tracking
- The `rebinding_scopes` map in SemanticModel

### Specific Recommendation

If you have a semantic model, use its global/nonlocal tracking:

```rust
// Check if a binding is global
if binding.is_global() {
    // This binding was declared with 'global'
}

// Check if it's nonlocal
if binding.is_nonlocal() {
    // This binding was declared with 'nonlocal'
}

// Find global declarations
for (name, binding_id) in scope.bindings() {
    let binding = semantic.binding(binding_id);
    if let BindingKind::Global(Some(global_binding_id)) = binding.kind {
        // This is a global declaration pointing to global_binding_id
    }
}
```

The semantic model tracks this automatically when built correctly through the visitor pattern.

## 9. Private Symbol Detection

### Issue

**File**: `crates/cribo/src/semantic_bundler.rs:198-200`

```rust
if name.starts_with('_') && !name.starts_with("__") {
    binding_flags |= BindingFlags::PRIVATE_DECLARATION;
}
```

### Problems

- This logic is already implemented in ruff
- Should use `Binding::is_private_declaration()` instead

### Specific Recommendation

Don't reimplement this logic. If you have a semantic model:

```rust
// Use the binding's built-in check
if binding.is_private_declaration() {
    // This is a private symbol
}
```

The semantic model sets this flag automatically when creating bindings if you build it properly through the visitor pattern.

## 10. TYPE_CHECKING Detection

### Issue

**File**: `crates/cribo/src/semantic_analysis.rs:251-259`

```rust
fn is_type_checking_block(&self, expr: &Expr) -> bool {
    if let Expr::Name(name) = expr {
        name.id == "TYPE_CHECKING"
    } else {
        false
    }
}
```

### Problems

- Doesn't check if TYPE_CHECKING is imported from typing
- Doesn't handle `typing.TYPE_CHECKING` attribute access
- Should use `SemanticModel::match_typing_expr(expr, "TYPE_CHECKING")`

### Specific Recommendation

With a semantic model:

```rust
// Proper TYPE_CHECKING detection
if semantic.match_typing_expr(expr, "TYPE_CHECKING") {
    // This is typing.TYPE_CHECKING
}

// Check if currently in TYPE_CHECKING block
if semantic.in_type_checking_block() {
    // Code is inside if TYPE_CHECKING
}
```

The semantic model handles this automatically when you:

1. Register typing modules with `add_module("typing")`
2. Create proper import bindings
3. Use `match_typing_expr()` which resolves through the binding system

## General Recommendations

### 1. How to Properly Use Ruff's Semantic Model

Based on pymermaider's successful implementation, there are two valid approaches:

**Approach A: Limited Scope (Like Pymermaider)**
If you only need import resolution and basic name lookup:

- Create a minimal semantic model
- Implement your own Checker with just the methods you need
- Only process import statements
- Use internal APIs directly (they work!)
- Accept that you're using internal APIs

**Approach B: Full Semantic Analysis**
If you need complete Python semantic understanding:

```rust
use ruff_python_ast::visitor::{Visitor, walk_stmt};
use ruff_python_semantic::{SemanticModel, SemanticModelBuilder};

struct SemanticAnalyzer<'a> {
    semantic: SemanticModel<'a>,
    // Additional state like current_node_id, flags, etc.
}

impl<'a> Visitor<'a> for SemanticAnalyzer<'a> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        // Pre-visit: handle binding creation
        match stmt {
            Stmt::FunctionDef(func_def) => {
                // Create type parameter scope
                self.semantic.push_scope(ScopeKind::Type);

                // Visit type parameters
                if let Some(type_params) = &func_def.type_params {
                    self.visit_type_params(type_params);
                }

                // Create function scope
                self.semantic.push_scope(ScopeKind::Function(func_def));

                // Store current scope for the binding
                let scope_id = self.semantic.scope_id;

                // Visit function body
                walk_stmt(self, stmt);

                // Pop scopes
                self.semantic.pop_scope(); // function scope
                self.semantic.pop_scope(); // type param scope

                // Add binding in parent scope
                let binding_id = self.semantic.push_binding(
                    func_def.name.range(),
                    BindingKind::FunctionDefinition(scope_id),
                    BindingFlags::empty(),
                );

                // Add to parent scope
                let current_scope = self.semantic.current_scope_mut();
                current_scope.add(&func_def.name, binding_id);
            }
            Stmt::Import(import) => {
                // Handle imports with proper qualified names
                for alias in &import.names {
                    let qualified_name = QualifiedName::user_defined(&alias.name);
                    let binding_id = self.semantic.push_binding(
                        alias.range(),
                        BindingKind::Import(Import {
                            qualified_name: Box::new(qualified_name),
                        }),
                        BindingFlags::EXTERNAL,
                    );

                    let name = alias.asname.as_ref().unwrap_or(&alias.name);
                    self.semantic.current_scope_mut().add(name, binding_id);
                }
            }
            _ => walk_stmt(self, stmt),
        }
    }
}
```

### 2. Key Patterns from Ruff's Implementation

**Scope Management**:

- Push scopes when entering function/class/comprehension
- Pop scopes when exiting
- Track scope IDs for bindings

**Binding Creation**:

- Create binding with `push_binding()`
- Add to current scope
- Handle shadowing and rebinding

**Reference Resolution**:

- Use `resolve_load()` when encountering name expressions
- Track resolved and unresolved references
- Handle forward references and typing contexts

### 3. Dealing with Internal APIs

**The Pymermaider Proof**: Internal APIs ARE accessible and work!

Pymermaider successfully uses:

- `semantic.global_scope_mut()`
- `semantic.current_scope_mut()`
- `semantic.push_builtin()`
- Direct access to `semantic.bindings`
- Direct access to `semantic.scopes`

**Option 1: Do What Pymermaider Does (RECOMMENDED for limited use)**

```rust
// Use the exact ruff version
ruff_python_semantic = { git = "https://github.com/astral-sh/ruff.git", tag="0.11.6" }

// Create your own Checker
pub struct Checker<'a> {
    semantic: SemanticModel<'a>,
}

// Use internal methods directly
let scope = self.semantic.global_scope_mut();
scope.add(name, binding_id);
```

**Option 2: Use ruff's Full Infrastructure**
If you need complete semantic analysis:

```rust
use ruff_linter::checker::Checker;
let mut checker = Checker::new(...);
checker.visit_module(ast);
```

**Option 3: Fork/Patch Ruff**
Only if you want official support for these methods.

### 4. Key Lessons from Pymermaider

1. **Internal APIs Work**: Don't be afraid to use them if they solve your problem
2. **Limited Scope is Valid**: You don't need full semantic analysis for many use cases
3. **Import-Only Processing**: If you just need name resolution, only process imports
4. **Pragmatic Approach**: Pymermaider reimplements some ruff logic but reuses what works
5. **Version Pinning**: Use a specific ruff version to ensure stability

### 5. Specific Recommendations for Cribo

Given that Cribo is doing Python bundling (similar scope to pymermaider's class diagram generation):

1. **Consider pymermaider's approach**: It's proven to work for a similar use case
2. **Accept internal API usage**: It's better than reimplementing incorrectly
3. **Pin your ruff version**: Like pymermaider does with `tag="0.11.6"`
4. **Focus on what you need**: Don't build full semantic analysis if you only need imports
5. **Study pymermaider's Checker**: It's a good template for your needs

## Improvements Implementation Plan

This section provides a step-by-step plan for implementing improvements, organized from smallest to largest impact on the codebase.

### Phase 1: Quick Wins (Direct Function/Struct Reuse)

These changes can be implemented independently without breaking existing functionality:

#### Step 1: Use Ruff's Private Symbol Detection

**Impact**: Minimal - Replace 3 lines with 1 function call
**Location**: `semantic_bundler.rs:198-200`

```rust
// Remove manual check:
// if name.starts_with('_') && !name.starts_with("__") {
//     binding_flags |= BindingFlags::PRIVATE_DECLARATION;
// }

// Use ruff's built-in (after creating binding):
if binding.is_private_declaration() {
    // Handle private declaration
}
```

#### Step 2: Use Ruff's TYPE_CHECKING Detection

**Impact**: Minimal - Replace custom function with ruff's method
**Location**: `semantic_analysis.rs:251-259`

```rust
// Remove is_type_checking_block() method
// Use: semantic.match_typing_expr(expr, "TYPE_CHECKING")
```

#### Step 3: Use Ruff's Decorator Analysis Functions

**Impact**: Minimal - Direct function reuse
**Add these checks where needed**:

- `is_enumeration()` for enum detection
- `is_final()` for final classes/methods
- `is_staticmethod()`, `is_classmethod()` for method types
- `is_abstract()`, `is_overload()`, `is_override()` for method decorators

### Phase 2: Small Refactoring (Reduce Code Ownership)

#### Step 4: Replace ExecutionContext with Ruff's Methods (Optional)

**Impact**: Small - Can keep your simpler version if it works
**Location**: `semantic_analysis.rs:8-35`

If you want to use ruff's execution context:

```rust
// Replace: if context == ExecutionContext::TypeCheckingBlock
// With: if semantic_model.in_type_checking_block()

// Replace: if context == ExecutionContext::TypeAnnotation
// With: if semantic_model.in_typing_context()
```

**Note**: This requires a properly built semantic model. Skip if your ExecutionContext is sufficient.

#### Step 5: Use QualifiedName::user_defined() Consistently

**Impact**: Small - Standardize qualified name creation
**Location**: Multiple places in `semantic_bundler.rs`

```rust
// Replace manual string manipulation with:
let qualified_name = QualifiedName::user_defined(&name);
```

### Phase 3: Import Processing Refactoring

#### Step 6: Create a Minimal Checker (Pymermaider Style)

**Impact**: Medium - New struct, but isolated from existing code
**Create new file**: `crates/cribo/src/minimal_checker.rs`

```rust
pub struct MinimalChecker<'a> {
    semantic: SemanticModel<'a>,
    locator: &'a Locator<'a>,
}

impl<'a> MinimalChecker<'a> {
    pub fn new(semantic: SemanticModel<'a>, locator: &'a Locator<'a>) -> Self {
        let mut checker = Self { semantic, locator };
        checker.bind_builtins();
        checker
    }

    fn bind_builtins(&mut self) {
        // Copy from pymermaider/src/checker.rs:35-42
    }

    pub fn see_imports(&mut self, stmts: &'a [ast::Stmt]) {
        // Copy from pymermaider/src/checker.rs:138-249
    }
}
```

#### Step 7: Switch to MinimalChecker for Import Processing

**Impact**: Medium - Replace manual import handling
**Location**: `semantic_bundler.rs:128-179`

Replace manual import processing with:

```rust
let mut checker = MinimalChecker::new(semantic, locator);
checker.see_imports(&ast);
```

### Phase 4: Semantic Model Building (Larger Refactoring)

#### Step 8: Pin Ruff Version

**Impact**: Low risk, high stability benefit
**Location**: `Cargo.toml`

```toml
# Change from:
ruff_python_semantic = "0.x.x"
# To:
ruff_python_semantic = { git = "https://github.com/astral-sh/ruff.git", tag = "0.11.6" }
```

#### Step 9: Replace SemanticModelBuilder with Proper Pattern

**Impact**: Large - Core refactoring
**Location**: `semantic_bundler.rs:82-186`

Two options:

**Option A: Minimal Import-Only Builder**

- Remove `traverse_and_bind()`
- Use MinimalChecker from Step 6
- Only process imports like pymermaider

**Option B: Full Semantic Builder**

- Implement proper visitor pattern
- Handle all statement types
- Push/pop scopes correctly
- Create proper bindings

### Phase 5: Symbol Resolution Improvements

#### Step 10: Use Ruff's resolve_qualified_name()

**Impact**: Large - Replace custom resolution
**Location**: `semantic_analysis.rs:100-303`

After implementing proper semantic model:

```rust
// Use ruff's resolution
if let Some(qualified_name) = semantic.resolve_qualified_name(expr) {
    // Use the resolved name
}
```

### Implementation Order Recommendation

1. **Week 1**: Phase 1 (Steps 1-3) - Quick wins, no risk
2. **Week 2**: Phase 2 (Steps 4-5) - Small refactoring
3. **Week 3**: Phase 3 (Steps 6-7) - Import processing
4. **Week 4**: Evaluate progress, decide on Phase 4 approach
5. **Week 5+**: Phase 4-5 based on needs

### Testing Strategy

For each step:

1. Add tests for the new functionality
2. Ensure existing tests pass
3. Compare bundling output before/after
4. Check for performance impact

### Rollback Plan

Each phase can be rolled back independently:

- Phase 1: Revert individual function changes
- Phase 2: Restore ExecutionContext if needed
- Phase 3: Keep manual import processing as fallback
- Phase 4: Maintain existing SemanticModelBuilder until new one is proven
- Phase 5: Keep custom resolution as backup

### Success Metrics

- **Code reduction**: Aim for 30-50% less code in semantic analysis
- **Correctness**: Better handling of edge cases (measured by tests)
- **Performance**: Similar or better bundling speed
- **Maintainability**: Fewer custom implementations to maintain

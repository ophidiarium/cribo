# Proper Usage of Ruff's Semantic Model

This document explains the correct way to build and use `ruff_python_semantic::SemanticModel` based on how ruff internally uses it.

## The Core Issue

Cribo attempts to manually build a `SemanticModel` by creating bindings and manipulating scopes directly. However, ruff's `SemanticModel` is designed to be built during AST traversal by a visitor that maintains complex invariants about scopes, bindings, and execution contexts.

## How Ruff Actually Builds Semantic Models

### 1. The Visitor Pattern

Ruff uses a `Checker` struct that implements the visitor pattern. The `Checker`:

- Contains a `SemanticModel` instance
- Traverses the AST in a specific order
- Maintains state about the current scope, execution context, etc.
- Creates bindings and references as it encounters them

```rust
// Simplified version of ruff's approach
struct Checker<'a> {
    semantic: SemanticModel<'a>,
    // ... other state
}

impl<'a> Visitor<'a> for Checker<'a> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        // Process statement and update semantic model
    }
}
```

### 2. Scope Management

Ruff maintains a scope stack internally:

```rust
// When entering a function
self.semantic.push_scope(ScopeKind::Function(function_def));

// Process function body...

// When exiting
self.semantic.pop_scope();
```

**Key point**: The `push_scope` and `pop_scope` methods are not part of the public API. They're internal to ruff's checker.

### 3. Binding Creation

Bindings are created through the checker, not directly:

```rust
// In ruff's checker
self.add_binding(
    "variable_name",
    range,
    BindingKind::Assignment,
    BindingFlags::empty(),
);
```

This method:

1. Creates the binding with `semantic.push_binding()`
2. Adds it to the current scope
3. Handles shadowing and other bookkeeping

### 4. The Deferred Visitation Pattern

Python's execution model requires careful handling of when code is analyzed:

```rust
// Function definitions are processed in two phases:
// 1. Create the function binding immediately
self.add_binding(name, range, BindingKind::FunctionDefinition(scope_id), flags);

// 2. Function body is visited later (deferred)
self.visit.functions.push((function_def, self.semantic.snapshot()));
```

## Why Cribo's Approach Doesn't Work

### 1. Missing Internal Methods

Cribo tries to use methods that don't exist in the public API:

- `semantic.global_scope_mut()` - doesn't exist
- `semantic.current_scope_mut()` - doesn't exist
- Direct scope manipulation - not supported

### 2. Incomplete State Management

The semantic model maintains complex invariants:

- Current scope ID
- Execution context flags
- Node IDs for source mapping
- Exception handling context

Manually building bindings doesn't maintain these invariants.

### 3. Missing Traversal Logic

Ruff's visitor handles many edge cases:

- Deferred function/class body visitation
- Type parameter scopes (PEP 695)
- Comprehension scopes
- Exception handler scopes
- Context manager scopes

## The Correct Approach for Cribo

### Option 1: Use Ruff's Linter Infrastructure (Recommended)

Instead of building a semantic model manually, use ruff's existing infrastructure:

```rust
use ruff_linter::checker::Checker;
use ruff_linter::settings::LinterSettings;

// Create a checker which will build the semantic model
let mut checker = Checker::new(
    &settings,
    &locator,
    &stylist,
    &indexer,
    flags,
    &source_kind,
);

// Run the checker (this builds the semantic model)
checker.visit_module(python_ast);

// Now you can access the fully-built semantic model
let semantic = checker.semantic();
```

### Option 2: Build a Custom Visitor

If you need custom analysis, build a visitor that works with an existing semantic model:

```rust
struct SymbolExtractor<'a> {
    semantic: &'a SemanticModel<'a>,
    symbols: Vec<String>,
}

impl<'a> SymbolExtractor<'a> {
    fn extract_symbols(&mut self) {
        // Use the semantic model's public API
        let global_scope = self.semantic.global_scope();

        for (name, binding_id) in global_scope.bindings() {
            let binding = self.semantic.binding(binding_id);

            match &binding.kind {
                BindingKind::ClassDefinition(_) | BindingKind::FunctionDefinition(_) => {
                    if !binding.is_private_declaration() {
                        self.symbols.push(name.to_string());
                    }
                }
                _ => {}
            }
        }
    }
}
```

### Option 3: Simplified Analysis Without Full Semantic Model

For basic symbol extraction, you might not need the full semantic model:

```rust
// Simple visitor that just collects top-level definitions
struct SimpleSymbolCollector {
    symbols: Vec<String>,
    in_function: bool,
}

impl SimpleSymbolCollector {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        if !self.in_function {
            match stmt {
                Stmt::FunctionDef(f) => {
                    self.symbols.push(f.name.to_string());
                }
                Stmt::ClassDef(c) => {
                    self.symbols.push(c.name.to_string());
                }
                _ => {}
            }
        }
    }
}
```

## Key Principles

1. **Don't manually build SemanticModel** - It's designed to be built by ruff's checker during AST traversal
2. **Use the public API** - Only use methods documented in the public interface
3. **Respect the visitor pattern** - The semantic model is populated during traversal, not after
4. **Consider alternatives** - You might not need a full semantic model for your use case

## Recommended Reading

1. Look at ruff's checker implementation: `crates/ruff_linter/src/checker/mod.rs`
2. Study how rules use the semantic model: `crates/ruff_linter/src/rules/`
3. Understand the visitor pattern: `crates/ruff_python_ast/src/visitor/`

The key insight is that `SemanticModel` is not a general-purpose API for semantic analysis - it's specifically designed to support ruff's linting infrastructure and assumes it's being built by ruff's checker during a specific traversal pattern.

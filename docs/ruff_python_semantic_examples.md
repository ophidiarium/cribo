# Using ruff_python_semantic and SemanticModel

This document provides examples of how to use `ruff_python_semantic`, particularly the `SemanticModel` and scope analysis features.

## Overview

The `ruff_python_semantic` crate provides semantic analysis capabilities for Python code. The main entry point is the `SemanticModel` struct, which tracks:

- **Scopes**: Module, class, function, and other scopes
- **Bindings**: Variable definitions and their properties
- **References**: Uses of variables
- **Import tracking**: Module imports and their usage

## Basic Setup

To create a `SemanticModel`, you need:

```rust
use ruff_python_semantic::{SemanticModel, Module, ModuleKind};
use ruff_python_ast::ModModule;
use std::path::Path;

// Create the module info
let module = Module::new(
    "my_module",
    ModuleKind::Module,
    Path::new("my_module.py"),
    /* source */ None,
);

// Create the semantic model
let typing_modules = vec![]; // Additional typing modules beyond typing/typing_extensions
let semantic = SemanticModel::new(&typing_modules, Path::new("my_module.py"), module);
```

## Key Components

### 1. Scopes

Scopes represent different contexts in Python code:

```rust
use ruff_python_semantic::{ScopeKind, ScopeId};

// The semantic model tracks a stack of scopes
// Access current scope:
let current_scope_id = semantic.scope_id;
let current_scope = &semantic.scopes[current_scope_id];

// Check scope type:
if current_scope.kind.is_function() {
    // In a function scope
}
if current_scope.kind.is_class() {
    // In a class scope  
}

// Iterate through parent scopes:
for scope_id in semantic.scopes.ancestor_ids(current_scope_id) {
    let scope = &semantic.scopes[scope_id];
    // Process ancestor scope
}
```

### 2. Bindings

Bindings represent variable definitions:

```rust
use ruff_python_semantic::{BindingKind, BindingFlags};

// Create a binding
let binding_id = semantic.push_binding(
    range,                                    // TextRange of the binding
    BindingKind::Assignment,                  // Type of binding
    BindingFlags::empty(),                    // Optional flags
);

// Common binding kinds:
// - BindingKind::Import(Import) - for imports
// - BindingKind::FromImport(FromImport) - for from imports
// - BindingKind::Assignment - for variable assignments
// - BindingKind::Annotation - for type annotations
// - BindingKind::ClassDefinition(ScopeId) - for class definitions
// - BindingKind::FunctionDefinition(ScopeId) - for function definitions

// Access binding information:
let binding = &semantic.bindings[binding_id];
if binding.is_unused() {
    // Binding has no references
}
if binding.is_external() {
    // Binding is from an external module
}
```

### 3. Symbol Lookup

Looking up symbols in the current scope:

```rust
// Lookup a symbol in the current scope
if let Some(binding_id) = semantic.lookup_symbol("my_var") {
    let binding = &semantic.bindings[binding_id];
    // Process the binding
}

// Lookup in a specific scope
if let Some(binding_id) = semantic.lookup_symbol_in_scope("my_var", scope_id, false) {
    // Found the symbol
}

// Check for builtin bindings
if semantic.has_builtin_binding("print") {
    // 'print' is available as a builtin
}
```

### 4. Import Analysis

Analyzing imports in the semantic model:

```rust
use ruff_python_semantic::{Imported, Import, FromImport};
use ruff_python_ast::name::QualifiedName;

// Check if a module has been seen
if semantic.seen_module(QualifiedName::builtin("os")) {
    // The 'os' module has been imported
}

// When processing an import statement:
match &binding.kind {
    BindingKind::Import(Import { qualified_name }) => {
        // Handle regular import
        println!("Import: {}", qualified_name);
    }
    BindingKind::FromImport(FromImport { module, member }) => {
        // Handle from import
        println!("From {} import {}", 
            module.as_ref().map_or("", |m| m.as_str()), 
            member);
    }
    _ => {}
}
```

## Complete Example: Finding Unused Imports

Here's a complete example that demonstrates finding unused imports:

```rust
use ruff_python_ast::visitor::{Visitor, walk_module};
use ruff_python_ast::{ModModule, Stmt};
use ruff_python_semantic::{BindingKind, SemanticModel};

struct UnusedImportChecker<'a> {
    semantic: &'a SemanticModel<'a>,
    unused_imports: Vec<String>,
}

impl<'a> UnusedImportChecker<'a> {
    fn check_unused_imports(&mut self) {
        // Iterate through all bindings in the module scope
        let module_scope = self.semantic.scopes.global();

        for (name, binding_id) in module_scope.bindings() {
            let binding = &self.semantic.bindings[*binding_id];

            // Check if it's an import binding
            match &binding.kind {
                BindingKind::Import(import) => {
                    if binding.is_unused() {
                        self.unused_imports
                            .push(format!("Unused import: {}", import.qualified_name));
                    }
                }
                BindingKind::FromImport(from_import) => {
                    if binding.is_unused() {
                        self.unused_imports.push(format!(
                            "Unused from import: {} from {}",
                            from_import.member,
                            from_import
                                .module
                                .as_ref()
                                .map_or("<module>", |m| m.as_str())
                        ));
                    }
                }
                _ => {}
            }
        }
    }
}
```

## Integration with AST Traversal

The `SemanticModel` is typically built up during AST traversal:

```rust
use ruff_python_ast::visitor::Visitor;
use ruff_python_ast::{Expr, Stmt};

struct SemanticAnalyzer<'a> {
    semantic: SemanticModel<'a>,
}

impl<'a> Visitor<'a> for SemanticAnalyzer<'a> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::Import(import_stmt) => {
                // Process import statement
                for alias in &import_stmt.names {
                    let binding_id = self.semantic.push_binding(
                        alias.range,
                        BindingKind::Import(Import {
                            qualified_name: QualifiedName::from_dotted_name(&alias.name),
                        }),
                        BindingFlags::empty(),
                    );

                    // Add binding to current scope
                    let name = alias
                        .asname
                        .as_ref()
                        .map_or(&alias.name, |asname| &asname.id);
                    self.semantic.add_binding(name, binding_id);
                }
            }
            Stmt::FunctionDef(func) => {
                // Create new function scope
                self.semantic.push_scope(ScopeKind::Function);

                // Visit function body
                self.visit_body(&func.body);

                // Pop scope
                self.semantic.pop_scope();
            }
            _ => {
                // Continue traversal
                ruff_python_ast::visitor::walk_stmt(self, stmt);
            }
        }
    }
}
```

## Advanced Features

### 1. Forward References

Handle forward references in type annotations:

```rust
if semantic.in_forward_reference() {
    // Special handling for forward references
    // Only global bindings are visible
}
```

### 2. Execution Context

Track runtime vs typing context:

```rust
use ruff_python_semantic::ExecutionContext;

match semantic.execution_context() {
    ExecutionContext::Runtime => {
        // Code executed at runtime
    }
    ExecutionContext::Typing => {
        // Code in TYPE_CHECKING blocks
    }
}
```

### 3. Exception Handling

Track which exceptions are being handled:

```rust
let current_exceptions = semantic.exceptions();
if current_exceptions.contains(&QualifiedName::builtin("ValueError")) {
    // Currently handling ValueError
}
```

## Best Practices

1. **Always maintain scope consistency**: Push and pop scopes in matching pairs
2. **Track all bindings**: Create bindings for all definitions (variables, functions, classes, imports)
3. **Record references**: Link all symbol uses to their bindings
4. **Handle special contexts**: TYPE_CHECKING blocks, forward references, etc.
5. **Consider binding flags**: Mark external, aliased, explicit exports appropriately

## Common Patterns

### Checking if a Symbol is Defined

```rust
fn is_symbol_defined(semantic: &SemanticModel, name: &str) -> bool {
    semantic.lookup_symbol(name).is_some()
}
```

### Finding All References to a Binding

```rust
fn get_all_references(semantic: &SemanticModel, binding_id: BindingId) -> Vec<TextRange> {
    let binding = &semantic.bindings[binding_id];
    binding
        .references()
        .map(|ref_id| semantic.resolved_references[ref_id].range())
        .collect()
}
```

### Checking Import Usage

```rust
fn is_import_used(semantic: &SemanticModel, import_name: &str) -> bool {
    semantic
        .lookup_symbol(import_name)
        .map(|binding_id| !semantic.bindings[binding_id].is_unused())
        .unwrap_or(false)
}
```

This provides a comprehensive guide to using `ruff_python_semantic` for semantic analysis of Python code.

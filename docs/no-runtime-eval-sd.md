# System Design: Static Python Bundle Generation Without Runtime Code Evaluation

## Executive Summary

This document outlines a comprehensive refactoring strategy to eliminate runtime code evaluation (`exec()`) from cribo's bundled output. Instead, we'll generate a single, statically analyzable Python file where all modules are inlined with proper namespace isolation through AST transformations.

## Current State Analysis

### Problems with Current Approach

1. **Runtime `exec()` usage**: Modules are wrapped in `exec()` calls, making the code:
   - Harder for AI models to analyze
   - Slower at runtime due to dynamic compilation
   - Impossible to generate accurate source maps
   - Difficult to debug with standard Python tools

2. **Namespace complexity**: The current approach creates module objects dynamically and executes code into their namespaces, requiring complex globals/locals manipulation.

3. **Lost optimization opportunities**: Dead code elimination and other optimizations are limited by the dynamic nature.

## Proposed Solution: Static Module Inlining

### Core Concept

Transform the bundled output from:

```python
# Current approach
import types
models = types.ModuleType('models')
models.user = types.ModuleType('models.user')
exec('class User: ...', globals(), models.user.__dict__)
```

To:

```python
# Proposed approach
class __cribo_module_models_user:
    class User:
        ...
    
    # Module-level variables
    __cribo_vars = {
        'DEFAULT_CONFIG': {'debug': True}
    }

# Create module facade
import types
models = types.ModuleType('models')
models.user = types.ModuleType('models.user')
# Copy all public attributes
for __name in dir(__cribo_module_models_user):
    if not __name.startswith('_'):
        setattr(models.user, __name, getattr(__cribo_module_models_user, __name))
# Copy module-level variables
for __k, __v in __cribo_module_models_user.__cribo_vars.items():
    setattr(models.user, __k, __v)
```

## Detailed Implementation Strategy

### Phase 1: AST Transformation Framework

#### 1.1 Module Wrapper Generation

Create a new AST transformer that wraps each module's content in a class:

```rust
// src/static_bundler.rs
pub struct StaticBundler {
    module_registry: IndexMap<String, WrappedModule>,
}

struct WrappedModule {
    wrapper_class_name: String, // e.g., "__cribo_module_models_user"
    original_name: String,      // e.g., "models.user"
    transformed_ast: ModModule,
}

impl StaticBundler {
    fn wrap_module(&mut self, module_name: &str, ast: ModModule) -> Result<WrappedModule> {
        let wrapper_class_name = self.generate_wrapper_name(module_name);
        let wrapped_ast = self.transform_module_to_class(module_name, ast)?;

        Ok(WrappedModule {
            wrapper_class_name,
            original_name: module_name.to_string(),
            transformed_ast: wrapped_ast,
        })
    }

    fn generate_wrapper_name(&self, module_name: &str) -> String {
        format!("__cribo_module_{}", module_name.replace('.', "_"))
    }
}
```

#### 1.2 AST Transformation Rules

Transform module-level code into class methods and attributes:

```rust
fn transform_module_to_class(&self, module_name: &str, mut ast: ModModule) -> Result<ModModule> {
    let mut class_body = Vec::new();
    let mut module_vars = IndexMap::new();

    for stmt in ast.body {
        match stmt {
            // Functions become class methods
            Stmt::FunctionDef(mut func) => {
                // Add @staticmethod decorator
                func.decorator_list.push(create_staticmethod_decorator());
                class_body.push(Stmt::FunctionDef(func));
            }

            // Classes remain as nested classes
            Stmt::ClassDef(class_def) => {
                class_body.push(Stmt::ClassDef(class_def));
            }

            // Module-level assignments go to __cribo_vars
            Stmt::Assign(assign) => {
                if let Some(name) = extract_simple_target(&assign) {
                    module_vars.insert(name, assign.value);
                } else {
                    // Complex assignments need special handling
                    class_body.push(create_init_method_statement(assign));
                }
            }

            // Import statements need special handling
            Stmt::Import(_) | Stmt::ImportFrom(_) => {
                // These are already hoisted, skip
            }

            // Other statements (if, for, etc.) go into __cribo_init
            _ => {
                class_body.push(create_init_method_statement(stmt));
            }
        }
    }

    // Add __cribo_vars as class attribute
    if !module_vars.is_empty() {
        class_body.push(create_module_vars_assignment(module_vars));
    }

    // Create the wrapper class
    let wrapper_class = create_class_def(&self.generate_wrapper_name(module_name), class_body);

    Ok(ModModule {
        body: vec![wrapper_class],
        range: TextRange::default(),
    })
}
```

### Phase 2: Import Resolution and Variable Access

#### 2.1 Import Rewriting

Transform imports to reference the wrapper classes:

```rust
fn rewrite_imports(&mut self, stmt: Stmt) -> Result<Stmt> {
    match stmt {
        Stmt::ImportFrom(import_from) => {
            if let Some(module) = &import_from.module {
                if self.is_bundled_module(&module.id) {
                    // Transform: from models.user import User
                    // To: User = __cribo_module_models_user.User
                    return self.create_wrapper_import(&import_from);
                }
            }
        }
        _ => {}
    }
    Ok(stmt)
}

fn create_wrapper_import(&self, import_from: &StmtImportFrom) -> Result<Stmt> {
    let wrapper_name = self.generate_wrapper_name(&import_from.module.as_ref().unwrap().id);
    let mut assignments = Vec::new();

    for alias in &import_from.names {
        let target = create_name_expr(&alias.name.id, ExprContext::Store);
        let value = create_attribute_expr(
            create_name_expr(&wrapper_name, ExprContext::Load),
            &alias.name.id,
            ExprContext::Load,
        );

        assignments.push(create_assignment(target, value));
    }

    // Return multiple assignments as a single statement block
    Ok(create_statement_block(assignments))
}
```

#### 2.2 Module-Level Variable Access

Handle access to module-level variables:

```python
# Generated code pattern
class __cribo_module_config:
    @staticmethod
    def get_config():
        # Access module variable through the class
        return __cribo_module_config.__cribo_vars['DEFAULT_CONFIG'].copy()
    
    __cribo_vars = {
        'DEFAULT_CONFIG': {'debug': True, 'timeout': 30}
    }
```

### Phase 3: Module Initialization

#### 3.1 Initialization Order

Maintain proper initialization order for modules with side effects:

```rust
fn generate_module_initializers(&self, sorted_modules: &[&ModuleNode]) -> Vec<Stmt> {
    let mut initializers = Vec::new();

    for module in sorted_modules {
        let wrapper_name = self.generate_wrapper_name(&module.name);

        // Check if module has __cribo_init method
        if self.module_has_init_code(&module.name) {
            // Call the initialization method
            let init_call = create_method_call(&wrapper_name, "__cribo_init");
            initializers.push(create_expr_stmt(init_call));
        }
    }

    initializers
}
```

#### 3.2 Module Facade Creation

Generate code to create the module facades that maintain compatibility:

```rust
fn generate_module_facades(&self) -> Vec<Stmt> {
    let mut facades = Vec::new();

    // Import types module
    facades.push(create_import("types"));

    for (module_name, wrapped) in &self.module_registry {
        // Create module hierarchy
        let parts: Vec<&str> = module_name.split('.').collect();

        // Create parent modules first
        for i in 1..=parts.len() {
            let partial_name = parts[..i].join(".");
            facades.extend(self.create_module_object(&partial_name, i == 1));
        }

        // Copy attributes from wrapper to module
        facades.extend(self.create_attribute_copying(&wrapped));
    }

    facades
}

fn create_attribute_copying(&self, wrapped: &WrappedModule) -> Vec<Stmt> {
    // Generate:
    // for __attr in dir(__cribo_module_X):
    //     if not __attr.startswith('_') or __attr == '__all__':
    //         setattr(module.x, __attr, getattr(__cribo_module_X, __attr))

    let wrapper_name = &wrapped.wrapper_class_name;
    let module_expr = self.create_module_reference(&wrapped.original_name);

    vec![
        create_attribute_copy_loop(wrapper_name, module_expr),
        create_vars_copy_statement(wrapper_name, module_expr),
    ]
}
```

### Phase 4: Special Cases Handling

#### 4.1 Circular Dependencies

For circular dependencies, we need lazy initialization:

```python
# Generated code for circular deps
class __cribo_module_a:
    @staticmethod
    def func_that_imports_b():
        # Lazy import at function level
        from __cribo_module_b import something
        return something()

class __cribo_module_b:
    @staticmethod  
    def func_that_imports_a():
        # Lazy import at function level
        from __cribo_module_a import something_else
        return something_else()
```

#### 4.2 `__all__` Exports

Handle `__all__` declarations specially:

```rust
fn handle_all_exports(&mut self, module_name: &str, all_value: &Expr) -> Result<()> {
    // Extract the list of exported names
    let exported_names = extract_string_list(all_value)?;

    // Store for later use when creating module facade
    self.module_exports
        .insert(module_name.to_string(), exported_names);

    // Also add __all__ to the wrapper class
    Ok(())
}
```

### Phase 5: Output Generation

#### 5.1 Final Bundle Structure

```python
#!/usr/bin/env python3
# Generated by Cribo - Python Source Bundler

# Standard library imports (hoisted)
import os
import sys
from typing import Optional

# Module wrapper classes
class __cribo_module_models_user:
    class User:
        def __init__(self, name: str):
            self.name = name
    
    __cribo_vars = {}

class __cribo_module_utils_helpers:
    @staticmethod
    def helper_func():
        return "helper"
    
    __cribo_vars = {}

# Module initialization
# (any module-level code that needs to run)

# Create module facades
import types
models = types.ModuleType('models')
models.user = types.ModuleType('models.user')

# Copy attributes
for __attr in dir(__cribo_module_models_user):
    if not __attr.startswith('_'):
        setattr(models.user, __attr, getattr(__cribo_module_models_user, __attr))

# ... more module setup ...

# Entry point code
def main():
    from models.user import User  # This now references __cribo_module_models_user.User
    user = User("Alice")
    print(user.name)

if __name__ == "__main__":
    main()
```

### Phase 6: Source Map Generation

With static code generation, we can now generate accurate source maps:

```rust
pub struct SourceMap {
    version: u32,
    sources: Vec<String>,
    mappings: String, // VLQ encoded mappings
}

impl StaticBundler {
    fn generate_source_map(&self, bundled_ast: &ModModule) -> Result<SourceMap> {
        let mut mapper = SourceMapper::new();

        for stmt in &bundled_ast.body {
            if let Some(original_location) = self.get_original_location(stmt) {
                let bundled_location = self.get_bundled_location(stmt);
                mapper.add_mapping(original_location, bundled_location);
            }
        }

        Ok(mapper.build())
    }
}
```

## Migration Strategy

### Step 1: Implement Core Transformation

1. Create `static_bundler.rs` with the basic AST transformation logic
2. Add tests for simple module wrapping
3. Implement attribute copying mechanism

### Step 2: Handle Import Resolution

1. Implement import rewriting for bundled modules
2. Add support for relative imports
3. Handle circular dependencies with lazy imports

### Step 3: Module Variables

1. Implement `__cribo_vars` dictionary approach
2. Transform variable access in functions
3. Handle complex assignment patterns

### Step 4: Integration

1. Add configuration option to use static bundling
2. Migrate existing tests to verify compatibility
3. Implement source map generation

### Step 5: Optimization

1. Add dead code elimination pass
2. Implement constant folding where possible
3. Minimize generated wrapper code

## Benefits

1. **AI Model Friendly**: Clean, static Python code without `exec()`
2. **Performance**: No runtime code compilation overhead
3. **Debuggability**: Standard Python debugging tools work
4. **Source Maps**: Accurate mapping back to original files
5. **Static Analysis**: Tools like mypy, pylint work on output
6. **Security**: No dynamic code evaluation risks

## Challenges and Solutions

### Challenge 1: Module-Level Code Execution Order

**Solution**: Use `__cribo_init()` methods called in dependency order

### Challenge 2: Dynamic Imports

**Solution**: Convert to static imports where possible, warn for truly dynamic cases

### Challenge 3: Global State

**Solution**: Isolate in `__cribo_vars` dictionaries per module

## Example Transformation

### Input (multiple files)

```python
# models/user.py
from dataclasses import dataclass

DEFAULT_USER_TYPE = "standard"

@dataclass
class User:
    name: str
    user_type: str = DEFAULT_USER_TYPE
    
    def greet(self):
        return f"Hello, {self.name}"

# main.py
from models.user import User

user = User("Alice")
print(user.greet())
```

### Output (single file)

```python
#!/usr/bin/env python3
# Generated by Cribo

from dataclasses import dataclass

# Module: models.user
class __cribo_module_models_user:
    @dataclass
    class User:
        name: str
        user_type: str = None
        
        def __post_init__(self):
            if self.user_type is None:
                self.user_type = __cribo_module_models_user.__cribo_vars['DEFAULT_USER_TYPE']
        
        def greet(self):
            return f"Hello, {self.name}"
    
    __cribo_vars = {
        'DEFAULT_USER_TYPE': "standard"
    }

# Create module facades
import types
models = types.ModuleType('models')
models.user = types.ModuleType('models.user')

for __attr in dir(__cribo_module_models_user):
    if not __attr.startswith('_'):
        setattr(models.user, __attr, getattr(__cribo_module_models_user, __attr))

for __k, __v in __cribo_module_models_user.__cribo_vars.items():
    setattr(models.user, __k, __v)

# Entry point
User = __cribo_module_models_user.User

user = User("Alice")
print(user.greet())
```

This approach eliminates all runtime code evaluation while maintaining full compatibility with Python's module system, enabling better performance, debuggability, and tool integration.

# Ruff AST Migration: Code Patterns Analysis

This document analyzes specific code patterns in Serpen that would need rewriting for a ruff AST migration, focusing on commonly used patterns and challenging transformations.

## Executive Summary

Serpen currently uses `ruff_python_ast` extensively throughout the codebase. A migration would require updating:

1. **AST Traversal Patterns** - Custom visitor implementations
2. **Node Creation Patterns** - Direct AST node instantiation
3. **Expression Handling** - Pattern matching and field access
4. **Import Statement Processing** - Specific import node handling
5. **Code Generation** - AST-to-source transformations

## File-by-File Analysis

### 1. ast_rewriter.rs - AST Transformation Core

#### Current Patterns

**Transformer Pattern (Currently Commented Out)**

```rust
// TODO: Replace Transformer with ruff visitor pattern
// impl Transformer for AstRewriter {
//     fn visit_expr_name(&mut self, mut expr: ast::ExprName) -> Option<ast::ExprName> {
//         let name = expr.id.as_str();
//         // Transform logic here
//         Some(expr)
//     }
// }
```

**Direct Node Creation**

```rust
let assignment = ast::StmtAssign {
    targets: vec![Expr::Name(ast::ExprName {
        id: alias_name.clone().into(),
        ctx: ExprContext::Store,
        range: Default::default(),
    })],
    value: Box::new(Expr::Name(ast::ExprName {
        id: actual_name.into(),
        ctx: ExprContext::Load,
        range: Default::default(),
    })),
    type_comment: None,
    range: Default::default(),
};
```

**AST Traversal for Renaming**

```rust
fn apply_renames_to_expr(&self, expr: &mut Expr, renames: &HashMap<String, String>) -> Result<()> {
    match expr {
        Expr::Name(name) => {
            if let Some(new_name) = renames.get(name.id.as_str()) {
                name.id = new_name.clone().into();
            }
        }
        Expr::Call(call) => {
            self.apply_renames_to_expr(&mut call.func, renames)?;
            for arg in &mut call.arguments.args {
                self.apply_renames_to_expr(arg, renames)?;
            }
        } // ... extensive pattern matching
    }
    Ok(())
}
```

#### Migration Requirements

**Before (Current ruff AST):**

- Direct field access: `expr.id.as_str()`
- Manual node construction with all fields
- Custom recursive traversal logic
- Pattern matching on specific node types

**After (New ruff AST):**

- Visitor pattern implementation
- Builder patterns for node creation
- Built-in traversal mechanisms
- Updated field names and structures

#### Challenging Transformations

1. **Complex Recursive Traversal**: The `apply_renames_to_expr` function handles 15+ expression types with deep recursion
2. **Symbol Table Management**: Cross-references between modules require maintaining state across traversals
3. **Import Alias Resolution**: Complex logic for resolving conflicting names across modules

### 2. emit.rs - Code Generation and Bundle Assembly

#### Current Patterns

**AST to String Conversion**

```rust
let mut generator = Generator::new();
for stmt in &module_ast.body {
    generator.unparse_stmt(stmt);
}
let module_code = generator.source;
```

**Dynamic Statement Creation**

```rust
fn create_import_statement(&self, module_name: &str) -> Stmt {
    if self.is_valid_module_name(module_name) {
        let import_code = format!("import {}", module_name);
        self.parse_statement(&import_code).unwrap_or_else(|_| {
            self.create_comment_stmt(&format!("# Error importing: {}", module_name))
        })
    } else {
        self.create_comment_stmt(&format!(
            "# import {}  # Warning: unusual module name format",
            module_name
        ))
    }
}
```

**Module Namespace Creation**

```rust
fn create_module_exec_statement(&self, module_name: &str, module_code: &str) -> Result<Stmt> {
    let escaped_code = module_code
        .replace("\\", "\\\\")
        .replace("\"\"\"\"\"\""[..3].as_ref(), "\\\"\\\"\\\"");

    let exec_code = format!(
        "exec(\"\"\"{}\"\"\", {}.__dict__)",
        escaped_code, module_name
    );
    self.parse_statement(&exec_code)
}
```

**Import Filtering Logic**

```rust
fn filter_import_statements<F>(&self, statements: &[Stmt], keep_predicate: F) -> Result<Vec<Stmt>>
where
    F: Fn(&str) -> bool,
{
    let mut filtered_statements = Vec::new();
    for stmt in statements {
        match stmt {
            Stmt::Import(import_stmt) => {
                self.process_import_statement(
                    import_stmt,
                    &keep_predicate,
                    &mut filtered_statements,
                );
            }
            Stmt::ImportFrom(import_from_stmt) => {
                self.process_import_from_statement(
                    import_from_stmt,
                    &keep_predicate,
                    &mut filtered_statements,
                );
            }
            _ => {
                filtered_statements.push(stmt.clone());
            }
        }
    }
    Ok(filtered_statements)
}
```

#### Migration Requirements

**Code Generation Changes:**

- `Generator::new()` → New code generation API
- `unparse_stmt()` → Updated method signatures
- String manipulation for code creation → AST builder patterns

**Import Processing Changes:**

- Pattern matching on `Stmt::Import` → Updated enum variants
- Direct field access on import nodes → New field structures
- Clone operations → Reference handling changes

#### Challenging Transformations

1. **Dynamic Code Generation**: Heavy reliance on parsing generated strings back to AST
2. **Complex Import Filtering**: Nested logic for preserving/removing specific imports
3. **Module Namespace Simulation**: Creating Python `types.ModuleType` calls via AST manipulation

### 3. unused_imports_simple.rs - Import Analysis Engine

#### Current Patterns

**Import Collection from AST**

```rust
fn collect_imports(&mut self, stmt: &Stmt) {
    match stmt {
        Stmt::Import(import_stmt) => {
            for alias in &import_stmt.names {
                let module_name = alias.name.as_str();
                let local_name = alias
                    .asname
                    .as_ref()
                    .map(|n| n.as_str())
                    .unwrap_or(module_name);
                // Process import info
            }
        }
        Stmt::ImportFrom(import_from_stmt) => {
            let module_name = import_from_stmt
                .module
                .as_ref()
                .map(|m| m.as_str())
                .unwrap_or("");
            for alias in &import_from_stmt.names {
                self.process_import_from_alias(alias, module_name);
            }
        }
        _ => {}
    }
}
```

**Usage Tracking Traversal**

```rust
fn track_usage_in_expression(&mut self, expr: &ast::Expr) {
    match expr {
        Expr::Name(name_expr) => {
            let name = name_expr.id.as_str();
            self.used_names.insert(name.to_string());
        }
        Expr::Attribute(attr_expr) => {
            self.process_attribute_usage(expr);
            self.track_usage_in_expression(&attr_expr.value);
        }
        Expr::Call(call_expr) => {
            self.track_usage_in_expression(&call_expr.func);
            for arg in &call_expr.arguments.args {
                self.track_usage_in_expression(arg);
            }
            for keyword in &call_expr.arguments.keywords {
                self.track_usage_in_expression(&keyword.value);
            }
        } // ... 15+ more expression types
    }
}
```

**String Literal Processing**

```rust
fn process_all_list_element(&mut self, element: &ast::Expr) {
    if let Expr::StringLiteral(const_expr) = element {
        let s = &const_expr.value;
        self.exported_names.insert(s.to_string());
    }
}
```

#### Migration Requirements

**Import Node Changes:**

- `alias.name.as_str()` → New field access patterns
- `import_from_stmt.module` → Potential structure changes
- `alias.asname` → Field name/type updates

**Expression Traversal Changes:**

- Pattern matching on 15+ expression types → Updated enum variants
- Field access patterns → New field structures
- Arguments handling → `call_expr.arguments.args` structure changes

#### Challenging Transformations

1. **Comprehensive Expression Coverage**: Handles nearly all Python expression types
2. **Recursive State Management**: Maintains import/usage state across complex traversals
3. **String Literal Extraction**: Specific handling for `__all__` declarations

### 4. bundler.rs - Module Discovery and Import Extraction

#### Current Patterns

**Import Extraction from Parsed AST**

```rust
fn extract_imports_from_statement(&self, stmt: &Stmt, imports: &mut Vec<String>, file_path: &Path) {
    if let Stmt::Import(import_stmt) = stmt {
        for alias in &import_stmt.names {
            let module_name = alias.name.id.to_string();
            imports.push(module_name);
        }
    } else if let Stmt::ImportFrom(import_from_stmt) = stmt {
        self.process_import_from_statement(import_from_stmt, imports, file_path);
    }
}
```

**Relative Import Processing**

```rust
fn process_import_from_statement(
    &self,
    import_from_stmt: &StmtImportFrom,
    imports: &mut Vec<String>,
    file_path: &Path,
) {
    let level = import_from_stmt.level.unwrap_or(0) as u32;

    if level == 0 {
        self.process_absolute_import(import_from_stmt, imports);
        return;
    }

    if let Some(base_module) = self.resolve_relative_import(file_path, level) {
        self.process_resolved_relative_import(import_from_stmt, imports, &base_module);
    } else {
        self.process_fallback_relative_import(import_from_stmt, imports, level);
    }
}
```

**Module Name Resolution**

```rust
fn process_absolute_import(&self, import_from_stmt: &StmtImportFrom, imports: &mut Vec<String>) {
    if let Some(ref module) = import_from_stmt.module {
        let m = module.id.to_string();
        if !imports.contains(&m) {
            imports.push(m);
        }
    }
}
```

#### Migration Requirements

**Import Node Field Changes:**

- `alias.name.id` → Potential field restructuring
- `import_from_stmt.level` → Level handling changes
- `import_from_stmt.module.id` → Module reference updates

**Relative Import Changes:**

- Level calculation logic → Updated relative import handling
- Module resolution → New resolution mechanisms

#### Challenging Transformations

1. **Relative Import Resolution**: Complex file system path manipulation combined with AST analysis
2. **Module Discovery**: Two-phase process requiring coordination between AST parsing and dependency tracking
3. **Import Classification**: Integration with resolver for first-party vs third-party determination

## Most Critical Migration Challenges

### 1. Field Access Pattern Changes

**Current Pattern:**

```rust
let name = expr.id.as_str();
let module_name = alias.name.id.to_string();
```

**Expected Changes:**

- Field names may change
- Access patterns may require different methods
- String conversion may need different approaches

### 2. Node Construction Patterns

**Current Pattern:**

```rust
Expr::Name(ast::ExprName {
    id: name.clone().into(),
    ctx: ExprContext::Store,
    range: Default::default(),
})
```

**Expected Changes:**

- Constructor patterns may change
- Required fields may be different
- Builder patterns may be preferred

### 3. Visitor Pattern Implementation

**Current Challenge:**
The codebase has commented-out Transformer implementations that need to be replaced with ruff's visitor pattern.

**Migration Strategy:**

- Implement new visitor traits
- Update traversal logic
- Maintain state management across visits

### 4. Import Processing Complexity

**Current Challenge:**
Complex import processing logic across multiple files that depends on specific AST structures.

**Migration Strategy:**

- Update import node pattern matching
- Adapt relative import resolution
- Maintain compatibility with existing resolver logic

## Recommended Migration Approach

1. **Start with unused_imports_simple.rs**: Simplest AST usage patterns
2. **Move to bundler.rs**: Module discovery and import extraction
3. **Tackle emit.rs**: Complex code generation patterns
4. **Finish with ast_rewriter.rs**: Most complex transformation logic

Each step should include comprehensive testing to ensure functionality is preserved while adapting to new AST patterns.

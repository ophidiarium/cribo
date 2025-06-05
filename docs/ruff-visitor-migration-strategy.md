# Ruff AST Migration Strategy for Serpen

## Overview

This document outlines a practical migration strategy for adopting ruff's AST transformation patterns in Serpen, based on the analysis of ruff's architecture.

## Current State vs Target State

### Current State (Serpen)

- Manual AST traversal with match statements
- Direct AST mutation without visitor pattern
- Basic import alias tracking
- No systematic location management
- Multiple passes for different operations

### Target State (Ruff-inspired)

- Dual visitor/transformer pattern
- Systematic AST traversal with walk functions
- CST-based import manipulation for formatting preservation
- Comprehensive location management with source mapping
- Single-pass transformations where possible

## Phase 1: Foundation

### 1.1 Create Core Visitor Infrastructure

Create `src/visitor.rs`:

```rust
use ruff_python_ast::{self as ast, Expr, Stmt};

/// Read-only visitor trait for AST analysis
pub trait Visitor<'a> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        walk_expr(self, expr);
    }

    // Add methods for each node type...
}

/// Walk functions for each node type
pub fn walk_stmt<'a, V: Visitor<'a> + ?Sized>(visitor: &mut V, stmt: &'a Stmt) {
    match stmt {
        Stmt::Import(import) => {
            for alias in &import.names {
                visitor.visit_alias(alias);
            }
        }
        Stmt::ImportFrom(import_from) => {
            for alias in &import_from.names {
                visitor.visit_alias(alias);
            }
        } // Handle all statement types...
    }
}
```

### 1.2 Create Transformer Infrastructure

Create `src/transformer.rs`:

```rust
use ruff_python_ast::{self as ast, Expr, Stmt};

/// Mutable transformer trait for AST modifications
pub trait Transformer {
    fn visit_stmt(&self, stmt: &mut Stmt) {
        walk_stmt(self, stmt);
    }

    fn visit_expr(&self, expr: &mut Expr) {
        walk_expr(self, expr);
    }

    // Add methods for each node type...
}

/// Walk functions for mutable traversal
pub fn walk_stmt<V: Transformer + ?Sized>(visitor: &V, stmt: &mut Stmt) {
    match stmt {
        Stmt::Import(import) => {
            for alias in &mut import.names {
                visitor.visit_alias(alias);
            }
        } // Handle all statement types...
    }
}
```

### 1.3 Implement Location Management

Create `src/relocate.rs`:

```rust
use crate::transformer::{Transformer, walk_expr};
use ruff_text_size::TextRange;

/// Relocate expressions to new source positions
pub fn relocate_expr(expr: &mut Expr, range: TextRange) {
    Relocator { range }.visit_expr(expr);
}

struct Relocator {
    range: TextRange,
}

impl Transformer for Relocator {
    fn visit_expr(&self, expr: &mut Expr) {
        // Update range for each expression type
        match expr {
            Expr::Name(name) => name.range = self.range,
            // Handle all expression types...
        }
        walk_expr(self, expr);
    }
}
```

## Phase 2: Migration of Core Components

### 2.1 Migrate Import Alias Collection

Transform current implementation to use visitor pattern:

```rust
use crate::visitor::{Visitor, walk_stmt};

struct ImportAliasCollector {
    import_aliases: HashMap<String, ImportAlias>,
}

impl<'a> Visitor<'a> for ImportAliasCollector {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::ImportFrom(import_from) => {
                self.process_import_from(import_from);
            }
            Stmt::Import(import) => {
                self.process_import(import);
            }
            _ => {}
        }
        walk_stmt(self, stmt);
    }
}
```

### 2.2 Migrate AST Rewriting

Transform manual rewriting to use transformer:

```rust
use crate::transformer::{Transformer, walk_stmt};

struct ImportRewriter {
    renames: HashMap<String, String>,
}

impl Transformer for ImportRewriter {
    fn visit_expr(&self, expr: &mut Expr) {
        match expr {
            Expr::Name(name) => {
                if let Some(new_name) = self.renames.get(&name.id) {
                    name.id = new_name.clone();
                }
            }
            _ => {}
        }
        walk_expr(self, expr);
    }
}
```

## Phase 3: CST-Based Import Manipulation

### 3.1 Add libcst_native Dependency

```toml
[dependencies]
libcst_native = "1.1"
```

### 3.2 Implement Import Manipulation Module

Create `src/import_editor.rs`:

```rust
use crate::cst::matchers::match_statement;
use libcst_native::{ImportAlias, Statement};

pub struct ImportEditor<'a> {
    locator: &'a Locator<'a>,
    stylist: &'a Stylist<'a>,
}

impl<'a> ImportEditor<'a> {
    pub fn remove_imports(
        &self,
        imports_to_remove: &[&str],
        stmt: &Stmt,
    ) -> Result<Option<String>> {
        let module_text = self.locator.slice(stmt);
        let mut tree = match_statement(module_text)?;

        // Manipulate CST while preserving formatting
        // ...

        Ok(Some(tree.codegen_stylist(self.stylist)))
    }
}
```

## Phase 4: Integration and Testing

### 4.1 Update Bundler to Use New Infrastructure

```rust
impl Bundler {
    pub fn process_module(&mut self, module: &mut ast::ModModule) {
        // Phase 1: Collect information using visitors
        let mut collector = ImportAliasCollector::new();
        collector.visit_module(module);

        // Phase 2: Transform AST using transformers
        let mut rewriter = ImportRewriter::new(collector.import_aliases);
        rewriter.visit_module(module);

        // Phase 3: Clean up imports using CST
        let editor = ImportEditor::new(&self.locator, &self.stylist);
        self.cleanup_imports(module, &editor);
    }
}
```

### 4.2 Add Comprehensive Tests

Create test modules for each component:

- `tests/visitor_tests.rs`
- `tests/transformer_tests.rs`
- `tests/import_editing_tests.rs`

## Phase 5: Optimization

### 5.1 Implement Single-Pass Transformations

Combine multiple transformations into single passes:

```rust
struct CombinedTransformer {
    import_rewriter: ImportRewriter,
    unused_trimmer: UnusedImportTrimmer,
    location_updater: LocationUpdater,
}

impl Transformer for CombinedTransformer {
    fn visit_stmt(&self, stmt: &mut Stmt) {
        // Apply all transformations in one pass
        self.import_rewriter.visit_stmt(stmt);
        self.unused_trimmer.visit_stmt(stmt);
        self.location_updater.visit_stmt(stmt);
    }
}
```

### 5.2 Add Caching for Symbol Resolution

```rust
struct CachedSymbolResolver {
    cache: HashMap<String, Symbol>,
    semantic_model: SemanticModel,
}
```

## Migration Checklist

- [ ] **Phase 1: Foundation**
  - [ ] Create visitor trait and walk functions
  - [ ] Create transformer trait and walk functions
  - [ ] Implement location management utilities
  - [ ] Add basic tests for visitor/transformer infrastructure

- [ ] **Phase 2: Core Migration**
  - [ ] Migrate import alias collection to visitor pattern
  - [ ] Migrate AST rewriting to transformer pattern
  - [ ] Update existing tests to use new patterns
  - [ ] Ensure backward compatibility

- [ ] **Phase 3: CST Integration**
  - [ ] Add libcst_native dependency
  - [ ] Implement CST-based import editing
  - [ ] Add formatting preservation tests
  - [ ] Handle edge cases (comments, trailing commas)

- [ ] **Phase 4: Integration**
  - [ ] Update bundler to use new infrastructure
  - [ ] Add integration tests
  - [ ] Performance benchmarking
  - [ ] Documentation updates

- [ ] **Phase 5: Optimization**
  - [ ] Implement single-pass transformations
  - [ ] Add caching mechanisms
  - [ ] Profile and optimize hot paths
  - [ ] Final performance validation

## Risk Mitigation

1. **Gradual Migration**: Keep old implementation alongside new during transition
2. **Feature Flags**: Use feature flags to toggle between old and new implementations
3. **Comprehensive Testing**: Ensure 100% test coverage before switching
4. **Performance Monitoring**: Benchmark each phase against baseline
5. **Rollback Plan**: Maintain ability to revert to previous implementation

## Success Metrics

- **Code Quality**: Reduction in cyclomatic complexity by 40%
- **Performance**: No regression in bundling performance
- **Maintainability**: Clear separation of concerns with visitor pattern
- **Test Coverage**: Maintain or improve current coverage levels
- **Bug Count**: Reduction in import-related bugs by 50%

## Conclusion

This migration strategy provides a structured approach to adopting ruff's AST transformation patterns in Serpen. The phased approach minimizes risk while providing clear checkpoints for validation. The end result will be a more maintainable, performant, and robust AST transformation system.

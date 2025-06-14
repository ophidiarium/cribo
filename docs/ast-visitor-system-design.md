# AST Visitor Pattern System Design for Cribo

## Executive Summary

This document proposes introducing an AST visitor pattern to Cribo to enable comprehensive import discovery and advanced AST transformations. The design prioritizes **reusing existing visitor infrastructure** from established projects to minimize maintenance burden while providing powerful AST traversal capabilities.

**Recommendation**: Adopt **ruff_python_ast's visitor traits** as a git dependency, providing a battle-tested, Python-specific visitor implementation that aligns with our existing AST library choice.

## Problem Statement

### Current Limitations

1. **Incomplete Import Discovery**: Cribo only discovers module-level imports, missing function-scoped imports entirely
2. **Limited AST Transformation**: Current approach uses ad-hoc traversal for import rewriting
3. **Maintenance Burden**: Custom AST traversal code is error-prone and difficult to extend
4. **Missing Features**: Cannot implement advanced features like dead code elimination, import optimization, or comprehensive static analysis

### Requirements

1. **Complete Import Discovery**: Must find imports at any nesting level (functions, classes, conditionals)
2. **Flexible Transformation**: Support various AST modifications (import rewriting, dead code removal)
3. **Minimal Code Ownership**: Reuse existing, well-tested visitor infrastructure
4. **Type Safety**: Leverage Rust's type system for correctness
5. **Performance**: Efficient traversal without unnecessary allocations

## Analysis of Existing Solutions

### 1. Ruff's Visitor Infrastructure

**Pros:**

- Multiple visitor types for different use cases (read-only, mutable, source-order)
- Python-specific, handles all Python AST nodes
- Well-tested in production (Ruff is widely used)
- Clean trait-based design with default implementations
- Already uses `ruff_python_ast` which we depend on

**Cons:**

- Not published as a separate crate
- Would need to depend on specific git revision

**Verdict**: ✅ **Best fit for Cribo**

### 2. Pyrefly's Generic Visitor

**Pros:**

- Highly generic, works with any Rust type
- Sophisticated Uniplate-style design
- Compile-time optimizations
- Derive macro support

**Cons:**

- Overly complex for our needs
- Not Python-specific
- Would require significant adaptation work
- Generic design adds complexity without clear benefits

**Verdict**: ❌ Too complex for our use case

### 3. Rolldown's OXC Visitor

**Pros:**

- Clean visitor pattern implementation
- Good examples of real-world usage
- Well-integrated with AST builder

**Cons:**

- JavaScript/TypeScript specific
- Would require complete reimplementation for Python
- Different AST structure entirely

**Verdict**: ❌ Wrong language target

## Proposed Solution

### Adopt Ruff's Visitor Infrastructure

Add ruff's AST crate as a git dependency and use their visitor traits directly:

```toml
[dependencies]
ruff_python_ast = { git = "https://github.com/astral-sh/ruff", rev = "latest-stable" }
```

### Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                     Cribo Bundle Orchestrator                │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ┌─────────────────────┐      ┌──────────────────────┐    │
│  │   Import Discovery   │      │   AST Transformation  │    │
│  │      Visitor         │      │       Visitor         │    │
│  └──────────┬──────────┘      └──────────┬───────────┘    │
│             │                             │                 │
│             └──────────┬──────────────────┘                │
│                        │                                    │
│                        ▼                                    │
│          ┌──────────────────────────┐                      │
│          │  ruff_python_ast::visit  │                      │
│          │    Visitor Traits        │                      │
│          │  - Visitor<'a>           │                      │
│          │  - Transformer           │                      │
│          │  - walk_* functions      │                      │
│          └──────────────────────────┘                      │
│                        │                                    │
│                        ▼                                    │
│          ┌──────────────────────────┐                      │
│          │   ruff_python_ast AST    │                      │
│          │     (Already used)       │                      │
│          └──────────────────────────┘                      │
└─────────────────────────────────────────────────────────────┘
```

### Implementation Components

#### 1. Import Discovery Visitor

```rust
use ruff_python_ast::visitor::{Visitor, walk_stmt};
use ruff_python_ast::{Stmt, StmtImport, StmtImportFrom};

#[derive(Debug)]
pub struct ImportDiscoveryVisitor {
    imports: Vec<DiscoveredImport>,
    scope_stack: Vec<ScopeContext>,
}

#[derive(Debug, Clone)]
pub struct DiscoveredImport {
    pub module_name: String,
    pub names: Vec<String>,
    pub location: ImportLocation,
    pub line_number: usize,
}

#[derive(Debug, Clone)]
pub enum ImportLocation {
    Module,
    Function(String),
    Class(String),
    Method { class: String, method: String },
    Conditional { kind: ConditionalKind, depth: usize },
}

impl<'a> Visitor<'a> for ImportDiscoveryVisitor {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::Import(StmtImport { names, .. }) => {
                self.record_import(names);
            }
            Stmt::ImportFrom(StmtImportFrom { module, names, .. }) => {
                self.record_import_from(module, names);
            }
            Stmt::FunctionDef(func) => {
                self.scope_stack
                    .push(ScopeContext::Function(func.name.to_string()));
                walk_stmt(self, stmt); // Continue traversal
                self.scope_stack.pop();
                return; // Don't call walk_stmt again
            }
            Stmt::ClassDef(class) => {
                self.scope_stack
                    .push(ScopeContext::Class(class.name.to_string()));
                walk_stmt(self, stmt);
                self.scope_stack.pop();
                return;
            }
            _ => {}
        }

        // Default traversal for other statement types
        walk_stmt(self, stmt);
    }
}
```

#### 2. Import Rewriting Transformer

```rust
use ruff_python_ast::transformer::{Transformer, walk_stmt};

#[derive(Debug)]
pub struct ImportRewritingTransformer {
    movable_imports: Vec<MovableImport>,
    current_function: Option<String>,
    imports_to_remove: Vec<usize>, // Line numbers
}

impl Transformer for ImportRewritingTransformer {
    fn visit_stmt(&mut self, stmt: &mut Stmt) -> Option<Stmt> {
        match stmt {
            Stmt::Import(_) | Stmt::ImportFrom(_) => {
                if self.should_remove_import(stmt) {
                    return None; // Remove the import
                }
            }
            Stmt::FunctionDef(func) => {
                let old_function = self.current_function.clone();
                self.current_function = Some(func.name.to_string());

                // Add imports at the beginning of function body
                if let Some(imports) = self.get_imports_for_function(&func.name) {
                    self.prepend_imports_to_body(&mut func.body, imports);
                }

                // Continue traversal
                walk_stmt(self, stmt);

                self.current_function = old_function;
                return Some(stmt.clone());
            }
            _ => {}
        }

        walk_stmt(self, stmt);
        Some(stmt.clone())
    }
}
```

#### 3. Integration with Orchestrator

```rust
// In orchestrator.rs
impl BundleOrchestrator {
    /// Discover ALL imports in a Python file, including nested ones
    pub fn discover_all_imports(&self, file_path: &Path) -> Result<Vec<DiscoveredImport>> {
        let source = fs::read_to_string(file_path)?;
        let parsed = ruff_python_parser::parse_module(&source)?;

        let mut visitor = ImportDiscoveryVisitor::new();
        visitor.visit_module(&parsed.syntax());

        Ok(visitor.imports)
    }

    /// Transform AST to move imports for circular dependency resolution
    pub fn transform_imports(
        &self,
        ast: &mut ModModule,
        movable_imports: Vec<MovableImport>,
    ) -> Result<()> {
        let mut transformer = ImportRewritingTransformer::new(movable_imports);
        transformer.transform_module(ast);
        Ok(())
    }
}
```

### Migration Plan

#### Phase 1: Add Dependency & Basic Integration (1 day)

1. Add `ruff_python_ast` visitor module as dependency
2. Create `visitors` module in Cribo
3. Implement `ImportDiscoveryVisitor`
4. Replace existing `extract_imports` with visitor-based implementation
5. Verify all existing tests pass

#### Phase 2: Enhanced Import Discovery (1 day)

1. Add scope tracking to discovery visitor
2. Implement detection of function-scoped imports
3. Update graph building to use discovered imports
4. Add tests for nested import discovery

#### Phase 3: Import Rewriting Migration (1 day)

1. Implement `ImportRewritingTransformer` using Transformer trait
2. Replace existing import rewriting logic
3. Ensure all circular dependency tests pass
4. Add tests for edge cases

#### Phase 4: Advanced Features (Future)

1. Dead import detection visitor
2. Import optimization visitor
3. Code coverage visitor
4. Style checking visitors

### Benefits of This Approach

1. **Minimal Code Ownership**: We only maintain our specific visitors, not the traversal infrastructure
2. **Battle-Tested**: Ruff's visitor implementation is used in production by thousands of projects
3. **Type Safety**: Rust's type system ensures we handle all AST node types correctly
4. **Extensibility**: Easy to add new visitors for additional features
5. **Performance**: Ruff's implementation is optimized for performance
6. **Compatibility**: Already using `ruff_python_ast`, so no AST conversion needed

### Example: Complete Import Discovery

With the visitor pattern, discovering all imports becomes trivial:

```rust
// Before (incomplete, only module-level)
for stmt in module.body.iter() {
    if let Stmt::Import(_) = stmt {
        // Process import
    }
}

// After (complete, all scopes)
let mut visitor = ImportDiscoveryVisitor::new();
visitor.visit_module(module);
let all_imports = visitor.get_imports(); // Includes function-scoped!
```

### Risk Mitigation

1. **Dependency Stability**: Pin to specific git revision, update deliberately
2. **API Changes**: Ruff has stable visitor traits, unlikely to change
3. **Performance**: Benchmark before/after to ensure no regression
4. **Correctness**: Comprehensive test suite for all visitor implementations

## Implementation Checklist

- [ ] Add `ruff_python_ast` visitor module dependency
- [ ] Create `src/visitors/mod.rs` module structure
- [ ] Implement `ImportDiscoveryVisitor`
- [ ] Implement `ImportRewritingTransformer`
- [ ] Update `orchestrator.rs` to use visitors
- [ ] Add comprehensive tests for nested imports
- [ ] Update documentation
- [ ] Benchmark performance impact
- [ ] Consider additional visitors for future features

## Conclusion

Adopting Ruff's visitor infrastructure provides Cribo with a robust, well-tested foundation for AST traversal and transformation. This approach minimizes code ownership while enabling powerful features like complete import discovery and sophisticated AST transformations. The visitor pattern will unlock future enhancements and make the codebase more maintainable and extensible.

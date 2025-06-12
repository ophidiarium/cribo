# Ruff Semantic Analysis Implementation Patterns

## Overview

Ruff uses two different approaches for semantic analysis:

1. **Full Visitor Pattern** (`ruff_linter/src/checkers/ast/mod.rs`): Used by the linter for comprehensive analysis
2. **Incremental Builder Pattern** (`ty_python_semantic/src/semantic_index/builder.rs`): Used by the type checker for focused analysis

## Key Components

### 1. SemanticModel Structure

```rust
pub struct SemanticModel<'a> {
    // Core data structures
    nodes: Nodes<'a>,             // Stack of all AST nodes
    scopes: Scopes<'a>,           // Stack of all scopes with current scope ID
    bindings: Bindings<'a>,       // All bindings created in any scope
    definitions: Definitions<'a>, // All definitions created
    resolved_references: ResolvedReferences,
    unresolved_references: UnresolvedReferences,

    // Tracking maps
    shadowed_bindings: FxHashMap<BindingId, BindingId>,
    delayed_annotations: FxHashMap<BindingId, Vec<BindingId>>,
    rebinding_scopes: FxHashMap<BindingId, Vec<ScopeId>>,
    resolved_names: FxHashMap<NameId, BindingId>,

    // Context
    module: Module<'a>,
    flags: SemanticModelFlags,
    handled_exceptions: Vec<Exceptions>,
}
```

### 2. Binding System

```rust
pub struct Binding<'a> {
    pub kind: BindingKind<'a>,
    pub range: TextRange,
    pub scope: ScopeId,
    pub context: ExecutionContext,
    pub source: Option<NodeId>,
    pub references: Vec<ResolvedReferenceId>,
    pub exceptions: Exceptions,
    pub flags: BindingFlags,
}

pub enum BindingKind<'a> {
    Import(Import),
    FromImport(FromImport),
    SubmoduleImport(SubmoduleImport),
    Assignment,
    NamedExprAssignment,
    Annotation,
    FunctionDefinition(ScopeId),
    ClassDefinition(ScopeId),
    // ... more variants
}
```

### 3. Scope Management

```rust
pub struct Scope<'a> {
    pub kind: ScopeKind,
    pub parent: Option<ScopeId>,
    pub bindings: IndexMap<&'a str, BindingId>,
    pub star_imports: Vec<StarImport>,
    // Cached global/nonlocal declarations
}

pub enum ScopeKind {
    Module,
    Class,
    Function(FunctionDef),
    Generator { kind: GeneratorKind, is_async: bool },
    Lambda(Lambda),
}
```

## Implementation Patterns for Cribo

### 1. Minimal Visitor for Import Collection

```rust
struct ImportCollector<'a> {
    imports: Vec<ImportInfo<'a>>,
    current_scope: ScopeId,
    scopes: Vec<Scope>,
}

impl<'a> Visitor<'a> for ImportCollector<'a> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::Import(import) => {
                // Collect import info
                self.imports.push(ImportInfo {
                    names: import.names.clone(),
                    scope: self.current_scope,
                    range: stmt.range(),
                });
            }
            Stmt::ImportFrom(import_from) => {
                // Collect from-import info
            }
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {
                // Push new scope
                let scope_id = self.push_scope(ScopeKind::Function);
                walk_stmt(self, stmt);
                self.pop_scope();
            }
            _ => walk_stmt(self, stmt),
        }
    }
}
```

### 2. Lightweight Binding Tracker

```rust
struct BindingTracker {
    bindings: FxHashMap<String, Vec<BindingLocation>>,
    current_scope: usize,
}

struct BindingLocation {
    scope: usize,
    kind: BindingKind,
    range: TextRange,
}

impl BindingTracker {
    fn add_binding(&mut self, name: &str, kind: BindingKind, range: TextRange) {
        self.bindings
            .entry(name.to_string())
            .or_default()
            .push(BindingLocation {
                scope: self.current_scope,
                kind,
                range,
            });
    }

    fn find_binding(&self, name: &str, scope: usize) -> Option<&BindingLocation> {
        self.bindings
            .get(name)?
            .iter()
            .rfind(|b| b.scope == scope || self.is_ancestor(b.scope, scope))
    }
}
```

### 3. Reference Resolution

```rust
struct ReferenceResolver<'a> {
    bindings: &'a BindingTracker,
    unresolved: Vec<UnresolvedReference>,
}

impl<'a> ReferenceResolver<'a> {
    fn resolve_name(&mut self, name: &str, scope: usize, range: TextRange) {
        match self.bindings.find_binding(name, scope) {
            Some(binding) => {
                // Mark as used
            }
            None => {
                self.unresolved.push(UnresolvedReference {
                    name: name.to_string(),
                    scope,
                    range,
                });
            }
        }
    }
}
```

## Key Insights for Cribo

1. **Separate Passes**: Ruff separates binding collection from reference resolution
2. **Scope Stack**: Maintains a stack of scopes during traversal
3. **Deferred Analysis**: Function bodies are often deferred for later analysis
4. **Flags for Context**: Uses flags to track current context (e.g., in annotation, in type checking block)
5. **Minimal State**: For simple analysis (like import collection), use minimal state tracking

## Recommended Approach for Cribo's Unused Import Detection

1. **First Pass - Import Collection**:
   - Walk AST collecting all imports
   - Track scope information
   - Build simple symbol table

2. **Second Pass - Usage Tracking**:
   - Walk AST looking for name references
   - Mark imports as used when referenced
   - Handle special cases (e.g., `__all__`, re-exports)

3. **Final Analysis**:
   - Identify unused imports
   - Consider scope rules and shadowing
   - Generate fixes

This approach avoids the complexity of full semantic analysis while providing enough information for accurate unused import detection.

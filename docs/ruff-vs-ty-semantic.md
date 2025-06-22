# Ruff Python Semantic vs Ty Python Semantic: API Differences

This document outlines the differences between `ruff_python_semantic` and `ty_python_semantic` crates in the Ruff ecosystem from an API perspective.

## Overview

The Ruff project contains two distinct semantic analysis crates serving different purposes:

- **`ruff_python_semantic`**: Lightweight semantic analysis for linting
- **`ty_python_semantic`**: Full-featured type inference and type checking

## Core Purpose Differences

### `ruff_python_semantic` - Linting-Focused Semantic Analysis

**Primary Goal**: Provide semantic information needed for linting rules

**Key Capabilities**:

- Name binding and resolution
- Scope management
- Import tracking
- Reference tracking
- Basic execution context analysis
- Control flow graph construction

**Typical Use Cases**:

- Detecting undefined names
- Finding unused imports
- Identifying unused variables
- Checking for redefined builtins
- Analyzing import patterns

### `ty_python_semantic` - Type-Focused Semantic Analysis

**Primary Goal**: Provide complete type inference and checking

**Key Capabilities**:

- Full type inference engine
- Type checking with diagnostics
- Support for generics, protocols, type variables
- Module-level type resolution
- Incremental computation via Salsa
- Advanced type system features

**Typical Use Cases**:

- Static type checking (like mypy/pyright)
- IDE type information
- Type-aware code completion
- Type-based refactoring
- Type error diagnostics

## API Comparison

### 1. Semantic Model

#### `ruff_python_semantic::SemanticModel`

```rust
pub struct SemanticModel<'a> {
    // Basic semantic information
    pub bindings: Vec<Binding<'a>>,
    pub scopes: Vec<Scope<'a>>,
    pub references: Vec<Reference>,
    // ... other fields
}

impl<'a> SemanticModel<'a> {
    // Name resolution
    pub fn resolve_name(&self, expr: &ast::ExprName) -> Option<BindingId>;

    pub fn resolve_qualified_name(&self, expr: &Expr) -> Option<QualifiedName>;

    // Context queries
    pub fn in_typing_context(&self) -> bool;

    pub fn in_type_checking_block(&self) -> bool;

    // Import analysis
    pub fn match_typing_expr(&self, expr: &Expr, name: &str) -> bool;
}
```

#### `ty_python_semantic::SemanticModel`

```rust
pub trait SemanticModel {
    // Type-aware operations
    fn resolve_expression_type(&self, expr: &ast::Expr) -> Type;
    fn check_assignability(&self, target: Type, source: Type) -> TypeCheckResult;

    // Module-level type information
    fn module_type_environment(&self) -> &TypeEnvironment;
}

pub trait HasType {
    // Everything has a type in ty semantic model
    fn type_of(&self, db: &dyn Db) -> Type;
}
```

### 2. Type System

#### `ruff_python_semantic` - No Explicit Type System

- Uses string-based qualified names for type hints
- No type inference beyond literal types
- Type annotations are treated as AST nodes

#### `ty_python_semantic::types` - Full Type System

```rust
pub mod types {
    pub enum Type {
        Any,
        Unknown,
        Never,
        Module(ModuleType),
        Class(ClassType),
        Instance(InstanceType),
        Function(FunctionType),
        Union(UnionType),
        Intersection(IntersectionType),
        // ... many more type variants
    }

    impl Type {
        pub fn is_assignable_to(&self, db: &dyn Db, other: &Type) -> bool;

        pub fn member(&self, db: &dyn Db, name: &str) -> Option<Type>;
        // ... type operations
    }
}
```

### 3. Diagnostics

#### `ruff_python_semantic` - Simple References

```rust
// Just tracks whether names are defined/used
pub struct Reference {
    pub range: TextRange,
    pub resolved: Option<BindingId>,
}
```

#### `ty_python_semantic` - Type Diagnostics

```rust
pub struct TypeCheckDiagnostics {
    pub errors: Vec<TypeCheckError>,
}

pub enum TypeCheckError {
    TypeMismatch {
        expected: Type,
        actual: Type,
        range: TextRange,
    },
    UnboundName {
        name: String,
        range: TextRange,
    },
    IncompatibleOverride {
        base_type: Type,
        override_type: Type,
        // ... details
    },
    // ... many more error types
}
```

### 4. Public API Surface

#### `ruff_python_semantic/lib.rs` Exports

```rust
// Basic semantic building blocks
pub use binding::{Binding, BindingFlags, BindingId, BindingKind};
pub use imports::{FromImport, Import, ImportedName};
pub use model::SemanticModel;
pub use reference::{Reference, ResolvedReference};
pub use scope::{Scope, ScopeId, ScopeKind};

// Analysis functions
pub mod analyze {
    pub mod class;
    pub mod visibility; // is_private, is_public // is_enumeration, is_final
}
```

#### `ty_python_semantic/lib.rs` Exports (Conceptual)

```rust
// Type-aware semantic model
pub use semantic_model::{HasType, SemanticModel};

// Full type system
pub mod types;

// Type checking entry point
pub fn check_types(db: &dyn Db, file: File) -> TypeCheckDiagnostics;

// Type inference
pub mod inference {
    pub fn infer_module_types(db: &dyn Db, module: Module) -> TypeEnvironment;
}
```

## Key Architectural Differences

### 1. Incremental Computation

- **`ruff_python_semantic`**: Stateless, rebuilt for each file
- **`ty_python_semantic`**: Uses Salsa for incremental computation with caching

### 2. Cross-File Analysis

- **`ruff_python_semantic`**: Single-file focus
- **`ty_python_semantic`**: Multi-file type resolution and inference

### 3. Memory Model

- **`ruff_python_semantic`**: Lightweight, temporary lifetime
- **`ty_python_semantic`**: Persistent, cached in Salsa database

### 4. Extensibility

- **`ruff_python_semantic`**: Designed for linting rules
- **`ty_python_semantic`**: Designed for type system plugins and extensions

## When to Use Which?

### Use `ruff_python_semantic` when:

1. **Building Linting Rules**
   - Need to detect code patterns
   - Analyze variable usage
   - Check import conventions
   - Enforce coding standards

2. **Lightweight Analysis**
   - Quick AST traversal
   - Simple name resolution
   - Basic scope analysis
   - Import dependency tracking

3. **Performance Critical**
   - Need fast, single-pass analysis
   - Minimal memory footprint
   - No cross-file dependencies

### Use `ty_python_semantic` when:

1. **Type Checking**
   - Need actual type inference
   - Checking type compatibility
   - Validating type annotations
   - Protocol/generic support

2. **IDE Features**
   - Type-aware code completion
   - Type information on hover
   - Type-based refactoring
   - Go-to-type-definition

3. **Advanced Analysis**
   - Cross-file type tracking
   - Module interface analysis
   - Type flow analysis
   - Incremental recompilation

## Migration Considerations

If you're currently using `ruff_python_semantic` and considering `ty_python_semantic`:

### What Changes:

1. **API Surface**: Completely different API focused on types
2. **Performance Model**: Heavier weight, but with caching
3. **Dependencies**: Requires Salsa database setup
4. **Complexity**: Much more complex type system to work with

### What Stays Similar:

1. **AST Integration**: Both work with `ruff_python_ast`
2. **Parser**: Same underlying Python parser
3. **Basic Concepts**: Still have scopes, bindings, etc. but type-aware

## Example: Name Resolution

### `ruff_python_semantic` Approach

```rust
// Simple binding lookup
if let Some(binding_id) = semantic.resolve_name(name_expr) {
    let binding = semantic.binding(binding_id);
    match &binding.kind {
        BindingKind::Import(import) => {
            // Handle import
        }
        _ => {}
    }
}
```

### `ty_python_semantic` Approach

```rust
// Type-aware resolution
let name_type = semantic.resolve_expression_type(name_expr);
match name_type {
    Type::Module(module) => {
        // Full module type information
    }
    Type::Class(class) => {
        // Complete class type with methods, attributes
    }
    _ => {}
}
```

## Summary

- **`ruff_python_semantic`**: Fast, lightweight semantic analysis for linting
- **`ty_python_semantic`**: Complete type inference and checking system

Choose based on whether you need simple semantic information (use `ruff_python_semantic`) or full type analysis (use `ty_python_semantic`). For bundling tools like Cribo that need basic import and name resolution, `ruff_python_semantic` is likely sufficient unless type-aware bundling is required.

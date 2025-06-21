# Pymermaider's Usage of ruff_python_semantic

This document analyzes how the pymermaider project uses `ruff_python_semantic` to generate Mermaid class diagrams from Python code.

## Overview

Pymermaider is a tool that creates Mermaid class diagrams from Python source code. It uses ruff's semantic analysis capabilities to understand Python code structure, resolve names, and identify special patterns like abstract classes, enums, and decorators.

## Usage Patterns

### 1. Creating and Populating SemanticModel

**File**: `src/class_diagram.rs:431-433`

```rust
let semantic = SemanticModel::new(&[], Path::new(file), module);
let mut checker = Checker::new(&stylist, &locator, semantic);
checker.see_imports(&python_ast);
```

**What they do**:

- Create a new `SemanticModel` with empty typing modules list
- Wrap it in a custom `Checker` struct
- Process imports to populate the semantic model

**Why they do it**:

- Need semantic analysis to resolve qualified names of base classes
- Need to understand imports to resolve class inheritance relationships
- Need semantic context for decorator analysis

**How they do it**:

- Create a minimal semantic model (no custom typing modules)
- Use a custom checker that only implements what they need
- Only process import statements, not the full AST

### 2. Custom Checker Implementation

**File**: `src/checker.rs`

Pymermaider implements a "slimmed down" version of ruff's Checker with these key methods:

#### 2.1 Binding Builtins

```rust
fn bind_builtins(&mut self) {
    for builtin in python_builtins(u8::MAX, false).chain(MAGIC_GLOBALS.iter().copied()) {
        let binding_id = self.semantic.push_builtin();
        let scope = self.semantic.global_scope_mut();
        scope.add(builtin, binding_id);
    }
}
```

**What**: Manually adds Python builtins to the global scope
**Why**: Needed for proper name resolution (e.g., to recognize `object` as a builtin)
**How**: Uses internal methods `push_builtin()` and `global_scope_mut()` that are not in the public API

#### 2.2 Adding Bindings

```rust
fn add_binding(
    &mut self,
    name: &'a str,
    range: TextRange,
    kind: BindingKind<'a>,
    flags: BindingFlags,
) -> BindingId {
    // Create binding
    let binding_id = self.semantic.push_binding(range, kind, flags);

    // Handle private declarations
    if name.starts_with('_') {
        self.semantic.bindings[binding_id].flags |= BindingFlags::PRIVATE_DECLARATION;
    }

    // Handle shadowing and scope management
    // ... complex logic for handling existing bindings ...

    // Add to scope
    let scope = &mut self.semantic.scopes[scope_id];
    scope.add(name, binding_id);
}
```

**What**: Creates bindings for imports in the semantic model
**Why**: Enables qualified name resolution for imported symbols
**How**:

- Uses internal binding creation methods
- Directly modifies binding flags
- Manages shadowing manually
- Adds to scope using internal methods

#### 2.3 Processing Imports

```rust
pub fn see_imports(&mut self, stmts: &'a [ast::Stmt]) {
    // Processes import and from-import statements
    // Creates appropriate bindings for each imported name
}
```

**What**: Iterates through statements and processes only imports
**Why**: Only needs import information for name resolution, not full semantic analysis
**How**: Creates different binding kinds (Import, FromImport, SubmoduleImport) based on import type

### 3. Name Resolution

**File**: `src/class_diagram.rs:194-200`

```rust
for base in class.bases() {
    let base_name = match checker.semantic().resolve_qualified_name(base) {
        Some(base_name) => base_name,
        None => {
            let name = checker.locator().slice(base);
            QualifiedName::user_defined(name)
        }
    };
    // ... process base class
}
```

**What**: Resolves base class names to their fully qualified forms
**Why**: Needed to create accurate inheritance relationships in the diagram
**How**:

- Uses `resolve_qualified_name()` to get the full import path
- Falls back to raw text if resolution fails
- Filters out special cases like `object`, `ABC`, `ABCMeta`

### 4. Decorator Analysis

The semantic model is used extensively for analyzing decorators:

#### 4.1 ABC Detection

**File**: `src/utils.rs:5-18`

```rust
pub fn is_abc_class(bases: &[Expr], keywords: &[Keyword], semantic: &SemanticModel) -> bool {
    keywords.iter().any(|keyword| {
        keyword.arg.as_ref().is_some_and(|arg| arg == "metaclass")
            && semantic
                .resolve_qualified_name(&keyword.value)
                .is_some_and(|qualified_name| {
                    matches!(qualified_name.segments(), ["abc", "ABCMeta"])
                })
    }) || bases.iter().any(|base| {
        semantic
            .resolve_qualified_name(base)
            .is_some_and(|qualified_name| matches!(qualified_name.segments(), ["abc", "ABC"]))
    })
}
```

**What**: Checks if a class is abstract (ABC)
**Why**: Abstract classes get special notation in Mermaid diagrams
**How**:

- Resolves metaclass keyword arguments to check for `abc.ABCMeta`
- Resolves base classes to check for `abc.ABC`

#### 4.2 Other Decorator Checks

**File**: `src/class_diagram.rs` (various locations)

Uses these ruff semantic analysis functions:

- `is_enumeration()` - Check if class is an Enum
- `is_final()` - Check for `@final` decorator
- `is_staticmethod()` - Check for `@staticmethod`
- `is_classmethod()` - Check for `@classmethod`
- `is_abstract()` - Check for `@abstractmethod`
- `is_overload()` - Check for `@overload`
- `is_override()` - Check for `@override`

**What**: Identifies special method/class types
**Why**: Different decorators result in different Mermaid notation
**How**: These functions all use the semantic model to resolve decorator names and check if they match known patterns

## Key Observations

### 1. Limited Semantic Model Usage

Pymermaider only uses the semantic model for:

- Import resolution
- Qualified name resolution
- Decorator pattern matching

They don't need:

- Full AST traversal
- Reference tracking
- Scope analysis beyond imports
- Type inference

### 2. Heavy Use of Internal APIs

The custom Checker uses many internal methods:

- `global_scope_mut()`
- `current_scope_mut()`
- `push_builtin()`
- Direct access to `semantic.bindings`
- Direct access to `semantic.scopes`

### 3. Manual Binding Management

Instead of using ruff's visitor pattern, they:

- Manually create bindings
- Manually handle shadowing
- Manually manage scope additions
- Only process import statements

### 4. Decorator Analysis Pattern

For each decorator type, they:

1. Get the decorator list from the AST
2. Pass it to a ruff analysis function along with the semantic model
3. The analysis function resolves decorator names and checks patterns
4. Return boolean indicating if the pattern matches

## Why This Approach?

Pymermaider's approach makes sense because:

1. **Limited Scope**: They only need to understand class structure and inheritance, not full program semantics
2. **Performance**: Processing only imports is faster than full semantic analysis
3. **Simplicity**: They don't need the complexity of reference tracking, type inference, etc.
4. **Focused Purpose**: The tool has one job - create class diagrams - so it only builds what it needs

However, this approach has risks:

- Uses internal APIs that could change
- Reimplements some of ruff's logic (binding creation, shadowing)
- May miss edge cases that ruff's full analysis would catch

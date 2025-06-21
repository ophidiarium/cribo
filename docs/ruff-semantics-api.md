# Ruff Python Semantic API Documentation

This document provides a comprehensive overview of the public API methods in the `ruff_python_semantic` crate. The crate provides semantic analysis functionality for Python code, enabling the understanding of bindings, scopes, references, and other semantic properties.

## Core Components

### SemanticModel

The `SemanticModel` is the central structure that represents the semantic information of a Python module. It tracks bindings, scopes, references, and provides methods to query semantic information.

#### Key Methods

##### Creation

- **`new(typing_modules: &'a [String], path: &Path, module: Module<'a>) -> Self`**
  - **Purpose**: Creates a new semantic model for a Python module
  - **Input**: List of custom typing modules, file path, and the module definition
  - **Output**: A fresh SemanticModel instance
  - **Why it exists**: Entry point for semantic analysis of a module

##### Binding Operations

- **`binding(&self, id: BindingId) -> &Binding<'a>`**
  - **Purpose**: Retrieves a binding by its ID
  - **Input**: BindingId
  - **Output**: Reference to the Binding
  - **Why it exists**: Core access method for binding information

- **`push_binding(&mut self, range: TextRange, kind: BindingKind<'a>, flags: BindingFlags) -> BindingId`**
  - **Purpose**: Creates a new binding and adds it to the model
  - **Input**: Source range, binding kind (e.g., Assignment, Import), and flags
  - **Output**: ID of the newly created binding
  - **Why it exists**: Records new symbol bindings during AST traversal

- **`push_builtin(&mut self) -> BindingId`**
  - **Purpose**: Creates a special binding for built-in symbols
  - **Input**: None
  - **Output**: ID of the builtin binding
  - **Why it exists**: Handles Python's built-in symbols specially

##### Symbol Resolution

- **`resolve_name(&self, name: &ast::ExprName) -> Option<BindingId>`**
  - **Purpose**: Resolves a name expression to its binding
  - **Input**: AST name expression
  - **Output**: Optional binding ID if resolved
  - **Why it exists**: Maps AST nodes to their semantic bindings

- **`resolve_qualified_name<'name>(&self, value: &'expr Expr) -> Option<QualifiedName<'name>>`**
  - **Purpose**: Resolves an expression to its fully-qualified name (e.g., `sys.version_info`)
  - **Input**: Any expression
  - **Output**: Optional qualified name if it's an imported/builtin symbol
  - **Why it exists**: Enables tracking of imported symbols and their origins

- **`resolve_load(&mut self, name: &ast::ExprName) -> ReadResult`**
  - **Purpose**: Resolves a "load" reference (when a name is read/used)
  - **Input**: Name expression being loaded
  - **Output**: Resolution result indicating if binding was found
  - **Why it exists**: Tracks variable usage and handles forward references

- **`resolve_del(&mut self, symbol: &str, range: TextRange)`**
  - **Purpose**: Handles deletion of a symbol (del statement)
  - **Input**: Symbol name and source range
  - **Output**: None (mutates internal state)
  - **Why it exists**: Tracks symbol deletions for flow analysis

##### Scope Queries

- **`lookup_symbol(&self, symbol: &str) -> Option<BindingId>`**
  - **Purpose**: Looks up a symbol in the current scope chain
  - **Input**: Symbol name
  - **Output**: Optional binding ID
  - **Why it exists**: Basic symbol resolution following Python's scoping rules

- **`lookup_attribute(&self, value: &Expr) -> Option<BindingId>`**
  - **Purpose**: Resolves attribute access (e.g., `Class.method`)
  - **Input**: Attribute expression
  - **Output**: Optional binding ID of the attribute
  - **Why it exists**: Enables class member resolution

- **`is_available(&self, member: &str) -> bool`**
  - **Purpose**: Checks if a name is available (not bound) in current scope
  - **Input**: Member name
  - **Output**: True if the name is not bound
  - **Why it exists**: Helps with name collision detection

##### Type Checking Utilities

- **`match_typing_expr(&self, expr: &Expr, target: &str) -> bool`**
  - **Purpose**: Checks if expression references a typing module member
  - **Input**: Expression and target name (e.g., "List")
  - **Output**: True if expr is typing.{target}
  - **Why it exists**: Identifies type annotations and typing constructs

- **`match_builtin_expr(&self, expr: &Expr, symbol: &str) -> bool`**
  - **Purpose**: Checks if expression references a builtin
  - **Input**: Expression and builtin name
  - **Output**: True if expr references the builtin
  - **Why it exists**: Identifies builtin usage without string comparisons

- **`resolve_builtin_symbol<'expr>(&'a self, expr: &'expr Expr) -> Option<&'a str>`**
  - **Purpose**: Extracts the builtin symbol name from an expression
  - **Input**: Expression that might reference a builtin
  - **Output**: Optional builtin name
  - **Why it exists**: Enables builtin-specific analysis rules

### Binding

Represents a name binding in Python code (variable assignments, imports, function definitions, etc.).

#### Key Methods

- **`is_unused(&self) -> bool`**
  - **Purpose**: Checks if the binding has no references
  - **Input**: None
  - **Output**: True if unused
  - **Why it exists**: Enables unused variable detection

- **`is_used(&self) -> bool`**
  - **Purpose**: Opposite of is_unused
  - **Input**: None
  - **Output**: True if binding has references
  - **Why it exists**: Convenience method

- **`references(&self) -> impl Iterator<Item = ResolvedReferenceId>`**
  - **Purpose**: Iterates over all references to this binding
  - **Input**: None
  - **Output**: Iterator of reference IDs
  - **Why it exists**: Tracks all usage sites of a binding

- **`redefines(&self, existing: &Binding) -> bool`**
  - **Purpose**: Checks if this binding redefines another (per Pyflakes rules)
  - **Input**: Another binding
  - **Output**: True if this is a redefinition
  - **Why it exists**: Implements Python's complex redefinition semantics

- **`name<'b>(&self, source: &'b str) -> &'b str`**
  - **Purpose**: Extracts the binding's name from source code
  - **Input**: Source code string
  - **Output**: The name as a string slice
  - **Why it exists**: Bindings store ranges, not strings, for efficiency

- **`statement<'b>(&self, semantic: &SemanticModel<'b>) -> Option<&'b Stmt>`**
  - **Purpose**: Gets the statement that created this binding
  - **Input**: Semantic model
  - **Output**: Optional statement AST node
  - **Why it exists**: Links bindings back to their defining statements

#### Binding Flags (boolean properties)

- **`is_explicit_export()`, `is_external()`, `is_alias()`, `is_nonlocal()`, `is_global()`, `is_deleted()`, `is_unpacked_assignment()`, `is_unbound()`, `is_private_declaration()`, `in_exception_handler()`, `in_assert_statement()`, `is_annotated_type_alias()`, `is_type_alias()`**
  - **Purpose**: Query various properties of the binding
  - **Why they exist**: Enable specific lint rules and semantic analysis

### Scope

Represents a lexical scope in Python (module, function, class, etc.).

#### Key Methods

- **`get(&self, name: &str) -> Option<BindingId>`**
  - **Purpose**: Looks up a binding by name in this scope
  - **Input**: Name to look up
  - **Output**: Optional binding ID
  - **Why it exists**: Core name resolution within a scope

- **`add(&mut self, name: &'a str, id: BindingId) -> Option<BindingId>`**
  - **Purpose**: Adds a new binding to the scope
  - **Input**: Name and binding ID
  - **Output**: Previous binding ID if name was shadowed
  - **Why it exists**: Records new bindings and tracks shadowing

- **`binding_ids(&self) -> impl Iterator<Item = BindingId>`**
  - **Purpose**: Iterates over all binding IDs in the scope
  - **Input**: None
  - **Output**: Iterator of binding IDs
  - **Why it exists**: Enables scope-wide analysis

- **`get_all(&self, name: &str) -> impl Iterator<Item = BindingId>`**
  - **Purpose**: Gets all bindings for a name (including shadowed ones)
  - **Input**: Name to look up
  - **Output**: Iterator from newest to oldest binding
  - **Why it exists**: Tracks binding history for flow analysis

- **`uses_star_imports(&self) -> bool`**
  - **Purpose**: Checks if scope contains star imports
  - **Input**: None
  - **Output**: True if any star imports exist
  - **Why it exists**: Star imports affect name resolution

### Definition

Represents a documentable definition (module, class, function).

#### Key Types

- **`Module`**: Represents a Python module
- **`Member`**: Represents a class, function, or method
- **`MemberKind`**: Enum distinguishing between classes, functions, methods, etc.

#### Key Methods (Member)

- **`name(&self) -> &'a str`**
  - **Purpose**: Gets the member's name
  - **Input**: None
  - **Output**: Name string
  - **Why it exists**: Uniform name access across member types

- **`body(&self) -> &'a [Stmt]`**
  - **Purpose**: Gets the member's body statements
  - **Input**: None
  - **Output**: Slice of statements
  - **Why it exists**: Enables analysis of member contents

### ResolvedReference

Represents a resolved reference to a binding.

#### Key Methods

- **`is_load(&self) -> bool`**
  - **Purpose**: Checks if reference is a load (read) operation
  - **Input**: None
  - **Output**: True if loading/reading
  - **Why it exists**: Distinguishes reads from writes/deletes

- **`in_typing_context(&self) -> bool`**
  - **Purpose**: Checks if reference is in a type annotation context
  - **Input**: None
  - **Output**: True if in typing context
  - **Why it exists**: Type annotations have different runtime semantics

- **`in_runtime_context(&self) -> bool`**
  - **Purpose**: Opposite of in_typing_context
  - **Input**: None
  - **Output**: True if in runtime context
  - **Why it exists**: Convenience for runtime-only analysis

## Analysis Utilities (analyze module)

### Visibility Analysis (`visibility.rs`)

Functions for analyzing decorator-based properties:

- **`is_staticmethod(decorator_list: &[Decorator], semantic: &SemanticModel) -> bool`**
  - **Purpose**: Checks if function is decorated with @staticmethod
  - **Why it exists**: Static methods have different binding behavior

- **`is_classmethod(decorator_list: &[Decorator], semantic: &SemanticModel) -> bool`**
  - **Purpose**: Checks if function is decorated with @classmethod
  - **Why it exists**: Class methods receive cls parameter

- **`is_property(decorator_list: &[Decorator], extra_properties: P, semantic: &SemanticModel) -> bool`**
  - **Purpose**: Checks if function is a property
  - **Why it exists**: Properties behave like attributes, not methods

- **`is_overload(decorator_list: &[Decorator], semantic: &SemanticModel) -> bool`**
  - **Purpose**: Checks for @overload decorator
  - **Why it exists**: Overloads are typing-only constructs

### Import Analysis (`imports.rs`)

Utilities for analyzing import statements and their targets.

### Type Inference (`type_inference.rs`)

Basic type inference capabilities for simple cases.

### Typing Utilities (`typing.rs`)

Helpers for working with typing module constructs.

## Design Philosophy

The API is designed around several key principles:

1. **Efficiency**: Uses integer IDs (BindingId, ScopeId, etc.) instead of storing strings or AST nodes directly
2. **Completeness**: Tracks all semantic information needed for accurate Python analysis
3. **Queryability**: Provides both forward lookups (name → binding) and reverse lookups (binding → references)
4. **Context Awareness**: Distinguishes between runtime and typing contexts, which have different semantics in Python
5. **Scope Accuracy**: Faithfully models Python's complex scoping rules including nonlocal, global, and class scopes

The crate exists to provide a semantic layer over Python AST that enables sophisticated static analysis while maintaining high performance on large codebases.

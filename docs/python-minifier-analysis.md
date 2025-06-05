# Python-Minifier Type Annotation Removal Analysis

## Overview

Python-minifier uses an AST-based approach to remove type annotations from Python code. This is implemented in the `RemoveAnnotations` transform class.

## Key Design Decisions

### 1. Granular Control

Python-minifier offers fine-grained control over which annotations to remove:

- **Variable annotations** (`a: int = 1`)
- **Return annotations** (`def foo() -> int:`)
- **Argument annotations** (`def foo(a: str):`)
- **Class attribute annotations** (`class A: b: int`)

Each can be independently enabled/disabled via `RemoveAnnotationsOptions`.

### 2. AST Visitor Pattern

The implementation uses a visitor pattern that extends `SuiteTransformer`:

- Visits specific AST nodes (`FunctionDef`, `AnnAssign`, `arguments`)
- Modifies the AST in-place by removing or replacing annotation attributes
- Uses parent tracking to understand context (e.g., is this annotation in a class?)

### 3. Special Cases Handling

#### Valueless Annotations

When encountering annotations without values (e.g., `a: int`), the transformer:

- For variables: Replaces with `a: 0` to preserve local variable semantics
- For class attributes: Same replacement strategy
- This preserves Python's behavior where annotations create local bindings

#### Type-Sensitive Classes

Annotations are preserved for classes that require them:

- `@dataclass` decorated classes
- Classes inheriting from `typing.NamedTuple`
- Classes inheriting from `typing.TypedDict`

Detection uses decorator and base class inspection via the parent-tracked AST.

### 4. Implementation Details

#### Key Methods:

- `visit_FunctionDef`: Handles function annotations (return type via `node.returns`)
- `visit_arguments`: Processes argument annotations for all argument types
- `visit_AnnAssign`: Handles variable and class attribute annotations
- `visit_arg`: Removes individual argument annotations

#### AST Transformation Patterns:

1. **AnnAssign with value**: `a: int = 1` → `a = 1` (converted to regular Assign)
2. **AnnAssign without value**: `a: int` → `a: 0` (keeps AnnAssign with dummy annotation)
3. **Function returns**: Set `node.returns = None`
4. **Arguments**: Set `node.annotation = None` on arg nodes

### 5. Parent Tracking System

Uses a custom parent tracking system (`add_parent`, `get_parent`) to:

- Determine if an annotation is inside a class
- Check decorators on parent classes
- Maintain AST relationships during transformation

### 6. AST Compatibility Layer

The `ast_compat` module provides:

- Backwards compatibility for different Python versions
- Shims for missing AST node types in older Python versions
- Consistent interface across Python 2 and 3

## Advantages of This Approach

1. **Precision**: Can target specific annotation types without affecting others
2. **Context Awareness**: Understands when annotations are semantically important
3. **Safety**: Preserves variable binding semantics with the `: 0` pattern
4. **Extensibility**: Easy to add new special cases or patterns

## Potential Adaptations for RustPython-Parser

1. **Visitor Pattern**: Implement a similar AST visitor in Rust
2. **Parent Tracking**: Add parent references to AST nodes for context
3. **Special Case Detection**: Port the dataclass/NamedTuple detection logic
4. **Valueless Annotation Handling**: Use the `: 0` pattern or equivalent
5. **Granular Options**: Provide similar fine-grained control over removal

## Key Differences from Token-Based Approaches

- **Semantic Understanding**: AST approach understands code structure
- **Reliable Transformation**: No risk of breaking syntax through token manipulation
- **Context Preservation**: Can make decisions based on surrounding code
- **Type Safety**: Working with structured AST nodes vs. raw tokens

## Additional Findings

### Complex Type Handling

- The implementation handles all annotation complexity at the AST level
- Complex types like `List[int]`, `Optional[str]`, `Dict[str, int]` are removed entirely
- No special parsing needed for nested generic types

### Import Management

- Python-minifier does NOT automatically remove typing imports when annotations are removed
- This is left as a separate concern (likely handled by other transforms like unused import removal)
- Separation of concerns: annotation removal is independent of import management

### Python Version Compatibility

- Extensive use of version checking to handle Python 3.6+ features
- Special handling for Python 3.12+ generic syntax (PEP 695)
- Graceful degradation for older Python versions

### Edge Cases Handled

1. **Forward references**: Handled naturally by AST (strings in annotations)
2. **Complex decorators**: `@dataclass(frozen=True)` detection
3. **Multiple inheritance**: Checks all base classes for special types
4. **Nested classes**: Parent tracking ensures correct context
5. **Generic type parameters**: Preserved in class/function definitions when needed

### Limitations Observed

1. No automatic typing import cleanup
2. The `: 0` pattern for valueless annotations might be unexpected
3. No partial annotation removal (e.g., keep return types but remove argument types in same function)

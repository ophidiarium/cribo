# Type Stripping System Design for Serpen

## Overview

This document outlines the design and implementation strategy for adding type hint stripping functionality to Serpen. Type hint stripping removes static type annotations from Python code during bundling, reducing bundle size and eliminating runtime overhead of type-checking infrastructure.

## Requirements

Based on the task specification and research findings, the type stripping system must handle:

1. **Syntactic type annotations** - Parameter and return type hints (`def func(x: int) -> str:`)
2. **Variable annotations** - Type hints on variables (`x: List[int] = []`)
3. **TYPE_CHECKING blocks** - Dead code elimination (`if TYPE_CHECKING:`)
4. **Typing imports** - Remove unused typing module imports
5. **Future annotations** - Remove `from __future__ import annotations`
6. **Typing.cast calls** - Rewrite `typing.cast(T, value)` to just `value`

## Research Findings

### Existing Tools Analysis

#### 1. strip-hints (Token-based approach)

- **Approach**: Token-level processing, preserves line numbers by replacing with whitespace
- **Strengths**: Simple, preserves source structure, well-tested edge cases
- **Limitations**: Cannot handle TYPE_CHECKING blocks or typing.cast(), limited semantic understanding
- **Key insight**: Extensive test cases for edge cases like line breaks in type hints

#### 2. python-minifier (AST-based approach)

- **Approach**: AST transformation with visitor pattern
- **Strengths**: Semantic understanding, handles complex cases, validates result
- **Key patterns**:
  - Converts `AnnAssign` to `Assign` nodes when possible
  - Preserves annotations needed for runtime (dataclasses, NamedTuple)
  - Uses parent tracking for context-aware decisions

#### 3. TypeStripper (Regex-based approach)

- **Approach**: Pure regex pattern matching
- **Limitations**: Very limited type support, fragile, no semantic understanding
- **Insight**: Shows the limitations of non-AST approaches

#### 4. Ruff (Production AST handling)

- **Approach**: Advanced semantic analysis with rustpython-ast
- **Key patterns**:
  - Sophisticated TYPE_CHECKING block detection
  - Import tracking and resolution
  - Context-aware type annotation identification
  - Semantic model for understanding symbol usage

#### 5. pyrefly (Modern AST patterns)

- **Key finding**: Uses ruff's AST modules instead of rustpython-parser directly
- **Advanced patterns**: Sophisticated visitor pattern with compile-time optimizations
- **Recommendation**: Consider migrating to ruff's AST for better compatibility

## Technical Challenges

### 1. AST Library Choice

**Challenge**: Current serpen uses `rustpython-parser`, but research shows `ruff_python_ast` is more mature and feature-complete.

**Decision**: Evaluate migrating to ruff's AST modules for type stripping, while maintaining rustpython-parser for other bundling operations if needed.

### 2. TYPE_CHECKING Block Detection

**Challenge**: Multiple patterns to detect TYPE_CHECKING conditions:

- Direct: `if TYPE_CHECKING:`
- Aliased: `if TC:` (where `from typing import TYPE_CHECKING as TC`)
- Module qualified: `if typing.TYPE_CHECKING:`

**Solution**: Implement import tracking to resolve aliases and module references.

### 3. Import Management

**Challenge**: Removing typing imports while preserving needed ones for runtime behavior.

**Solution**: Track which symbols are used for runtime vs type-checking purposes.

### 4. Edge Cases from Research

Key edge cases identified from strip-hints test suite:

- Type hints spanning multiple lines
- Comments inside type expressions
- Complex nested generic types
- Lambda parameters with type hints
- Dotted attribute assignments with types
- Protocol classes and special forms

## Implementation Strategy

### Phase 1: Core AST Infrastructure

#### 1.1 AST Library Integration

```rust
// Evaluate using ruff's AST instead of rustpython-parser
use ruff_python_ast as ast;
use ruff_python_parser::{ParseError, parse_module};

pub struct TypeStripper {
    import_tracker: ImportTracker,
    in_type_checking: bool,
    preserve_runtime_annotations: bool,
}
```

#### 1.2 Import Tracking System

```rust
pub struct ImportTracker {
    typing_aliases: HashMap<String, String>, // TC -> TYPE_CHECKING
    typing_modules: HashSet<String>,         // typing, typing_extensions
    used_symbols: HashSet<String>,           // track actual usage
}

impl ImportTracker {
    fn is_type_checking_alias(&self, name: &str) -> bool;
    fn is_typing_module_reference(&self, expr: &Expr) -> bool;
    fn track_import(&mut self, import: &StmtImport);
    fn track_import_from(&mut self, import: &StmtImportFrom);
}
```

### Phase 2: Core Transformations

#### 2.1 Function Annotation Stripping

```rust
impl TypeStripper {
    fn strip_function_annotations(&mut self, func: &mut StmtFunctionDef) {
        // Remove parameter annotations
        for arg in &mut func.args.args {
            arg.annotation = None;
        }
        for arg in &mut func.args.posonlyargs {
            arg.annotation = None;
        }
        for arg in &mut func.args.kwonlyargs {
            arg.annotation = None;
        }

        // Remove return annotation
        func.returns = None;
    }
}
```

#### 2.2 Variable Annotation Handling

```rust
fn transform_annotated_assignment(&mut self, ann_assign: &StmtAnnAssign) -> Vec<Stmt> {
    match ann_assign.value.as_ref() {
        Some(value) => {
            // Convert "x: int = 5" to "x = 5"
            vec![Stmt::Assign(StmtAssign {
                targets: vec![ann_assign.target.clone()],
                value: value.clone(),
                range: ann_assign.range,
                ..Default::default()
            })]
        }
        None => {
            // Convert "x: int" to empty (remove entirely)
            // or "pass  # x: int" for standalone type definitions
            if self.preserve_type_info_as_comments {
                vec![self.create_type_comment(&ann_assign)]
            } else {
                vec![] // Remove entirely
            }
        }
    }
}
```

#### 2.3 TYPE_CHECKING Block Removal

```rust
fn handle_if_statement(&mut self, if_stmt: &StmtIf) -> Vec<Stmt> {
    if self.is_type_checking_condition(&if_stmt.test) {
        // Remove the entire TYPE_CHECKING block
        // But preserve elif/else branches that aren't TYPE_CHECKING
        self.process_elif_else_clauses(&if_stmt.elif_else_clauses)
    } else {
        // Normal if statement processing
        vec![Stmt::If(self.transform_if(if_stmt))]
    }
}

fn is_type_checking_condition(&self, test: &Expr) -> bool {
    match test {
        Expr::Name(name) => {
            name.id == "TYPE_CHECKING" || self.import_tracker.is_type_checking_alias(&name.id)
        }
        Expr::Attribute(attr) => {
            attr.attr == "TYPE_CHECKING"
                && self.import_tracker.is_typing_module_reference(&attr.value)
        }
        _ => false,
    }
}
```

### Phase 3: Advanced Features

#### 3.1 typing.cast Rewriting

```rust
fn rewrite_typing_cast(&mut self, call: &ExprCall) -> Option<Expr> {
    // Check if this is typing.cast(Type, value)
    if self.is_typing_cast_call(call) && call.args.len() == 2 {
        // Return just the value (second argument)
        Some(call.args[1].clone())
    } else {
        None
    }
}

fn is_typing_cast_call(&self, call: &ExprCall) -> bool {
    match &call.func {
        Expr::Name(name) => {
            // Direct import: from typing import cast
            name.id == "cast" && self.import_tracker.is_typing_symbol("cast")
        }
        Expr::Attribute(attr) => {
            // Module access: typing.cast
            attr.attr == "cast" && self.import_tracker.is_typing_module_reference(&attr.value)
        }
        _ => false,
    }
}
```

#### 3.2 Import Cleanup

```rust
fn cleanup_unused_imports(&mut self, module: &mut ModModule) {
    module.body.retain(|stmt| {
        match stmt {
            Stmt::Import(import) => {
                // Remove imports of typing modules if no longer used
                import
                    .names
                    .retain(|alias| !self.import_tracker.is_unused_typing_import(&alias.name));
                !import.names.is_empty()
            }
            Stmt::ImportFrom(import_from) => {
                if let Some(module_name) = &import_from.module {
                    if module_name == "typing" || module_name == "typing_extensions" {
                        // Remove typing imports that are no longer used
                        import_from
                            .names
                            .retain(|alias| self.import_tracker.is_symbol_used(&alias.name));
                        !import_from.names.is_empty()
                    } else {
                        true
                    }
                } else {
                    true
                }
            }
            _ => true,
        }
    });
}
```

### Phase 4: Integration with Serpen

#### 4.1 Configuration

```toml
# serpen.toml
[bundler]
strip_type_hints = true
preserve_runtime_annotations = true # Keep dataclass, NamedTuple annotations
preserve_type_comments = false # Convert removed types to comments
```

#### 4.2 Integration Points

```rust
// In bundler.rs
pub struct BundlerConfig {
    pub strip_type_hints: bool,
    pub preserve_runtime_annotations: bool,
    pub preserve_type_comments: bool,
}

// In emit.rs or ast_rewriter.rs
impl Bundler {
    fn process_module(&mut self, module: &str, content: &str) -> Result<String> {
        let mut ast = parse_module(content)?;

        if self.config.strip_type_hints {
            let mut stripper = TypeStripper::new(&self.config);
            ast = stripper.transform_module(ast)?;
        }

        // Continue with existing bundling logic
        self.unparse_module(ast)
    }
}
```

## Testing Strategy

### Test Categories

#### 1. Unit Tests - Core Transformations

- Function parameter annotation removal
- Return type annotation removal
- Variable annotation handling
- TYPE_CHECKING block detection and removal
- Import cleanup logic

#### 2. Integration Tests - Real World Scenarios

Based on strip-hints test cases:

```python
# Complex function signatures
def complex_function(
    x: List[Dict[str, Union[int, float]]],
    y: Optional[Callable[[int], str]] = None
) -> Tuple[str, ...]:
    pass

# TYPE_CHECKING blocks
if TYPE_CHECKING:
    from typing import Protocol
    from mymodule import ComplexType

# Variable annotations
data: Dict[str, Any] = {}
x: int
y: str = "hello"

# typing.cast usage
result = typing.cast(List[int], some_function())
```

#### 3. Edge Case Tests

- Multi-line type annotations
- Comments within type expressions
- Nested if statements with TYPE_CHECKING
- Complex import aliases
- Generic class definitions
- Protocol and TypedDict classes

#### 4. Regression Tests

- Ensure valid Python syntax after stripping
- Verify runtime behavior is preserved
- Check bundle size reduction
- Test with real-world codebases

### Test Implementation

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_function_annotation_stripping() {
        let input = r#"
def func(x: int, y: str = "default") -> List[str]:
    return [str(x), y]
"#;
        let expected = r#"
def func(x, y = "default"):
    return [str(x), y]
"#;
        assert_eq!(strip_types(input), expected);
    }

    #[test]
    fn test_type_checking_block_removal() {
        let input = r#"
if TYPE_CHECKING:
    from typing import Protocol
    
def func():
    pass
"#;
        let expected = r#"
def func():
    pass
"#;
        assert_eq!(strip_types(input), expected);
    }

    // Use insta snapshots for complex cases
    #[test]
    fn test_complex_type_stripping() {
        let input = include_str!("fixtures/complex_types.py");
        insta::assert_snapshot!(strip_types(input));
    }
}
```

## Implementation Plan

### Milestone 1: Foundation (Week 1-2)

- [ ] Evaluate and potentially migrate to ruff's AST
- [ ] Implement basic TypeStripper structure
- [ ] Create ImportTracker system
- [ ] Set up test infrastructure with snapshot testing

### Milestone 2: Core Features (Week 3-4)

- [ ] Function annotation stripping
- [ ] Variable annotation handling
- [ ] Basic TYPE_CHECKING block detection
- [ ] Import cleanup logic
- [ ] Unit tests for core transformations

### Milestone 3: Advanced Features (Week 5-6)

- [ ] Complex TYPE_CHECKING block handling (elif/else)
- [ ] typing.cast rewriting
- [ ] Future annotations removal
- [ ] Edge case handling from research
- [ ] Integration tests with real codebases

### Milestone 4: Integration & Polish (Week 7-8)

- [ ] Integrate with Serpen bundler workflow
- [ ] Configuration system implementation
- [ ] Performance optimization
- [ ] Documentation and examples
- [ ] Bundle size benchmarking

## Decision Rationale

### AST-Based Approach

**Decision**: Use AST transformation rather than token or regex-based approaches.

**Rationale**:

- Semantic understanding enables handling of TYPE_CHECKING blocks and typing.cast
- More robust than regex approaches
- Easier to maintain and extend
- Can validate correctness of transformations

### Modular Design

**Decision**: Implement as separate module that integrates with existing bundler.

**Rationale**:

- Maintains separation of concerns
- Optional feature that can be disabled
- Easier to test in isolation
- Can be reused in other contexts

### Conservative Preservation Strategy

**Decision**: Preserve annotations that may be needed at runtime.

**Rationale**:

- Prevents breaking runtime behavior
- Safer for automated bundling
- Users can configure strictness level

This design provides a comprehensive foundation for implementing type hint stripping in Serpen while leveraging the best practices discovered through research of existing tools.

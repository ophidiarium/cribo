# NoOpsRemovalVisitor Implementation Plan

## Overview

This document outlines the implementation plan for a `NoOpsRemovalVisitor` that will remove safe-to-remove no-op patterns from Python AST during the bundling process. This feature will always be enabled (not configurable) and will only remove operations that are guaranteed to have no side effects.

## Motivation

During the bundling process, various AST transformations can introduce redundant operations that serve no purpose in the final bundled code. Removing these no-ops will:

1. Reduce bundle size
2. Improve code readability
3. Eliminate unnecessary runtime operations
4. Simplify the bundled output for better LLM comprehension

## Safe No-Op Patterns to Remove

### 1. Self-Reference Assignments at Module Level

```python
# These can be safely removed
x = x
validate = validate
process = process
```

**Safety**: Module-level simple name assignments have no side effects in Python. They merely rebind the name to the same object it already references.

### 2. Pass Statements in Non-Required Contexts

```python
# Can be removed when not syntactically required
def foo():
    x = 1
    pass  # Unnecessary
    return x

# Cannot be removed (syntactically required)
def empty():
    pass  # Required

class Empty:
    pass  # Required
```

### 3. Empty Expression Statements

```python
# Can be removed
None  # Standalone None expression
42    # Standalone literal
"string"  # Standalone string (unless docstring)
```

### 4. Identity Augmented Assignments

```python
# Can be removed
x += 0  # For numeric types
x *= 1  # For numeric types
x |= set()  # For sets
x &= x  # For sets/booleans
```

## Implementation Architecture

### 1. Visitor Pattern Using Ruff AST

We'll implement a transformer using `ruff_python_ast::visitor::transformer::Transformer` trait:

```rust
use ruff_python_ast::{
    Expr, ExprName, Operator, Stmt, StmtAssign, StmtAugAssign, StmtExpr, StmtPass,
    visitor::transformer::{Transformer, walk_stmt},
};

pub struct NoOpsRemovalTransformer {
    // Track if we're at module level
    scope_depth: usize,
    // Collect statements to remove
    statements_to_remove: Vec<usize>,
}

impl Transformer for NoOpsRemovalTransformer {
    fn visit_stmt(&self, stmt: &mut Stmt) {
        // Implementation details below
    }
}
```

### 2. Integration Point

The NoOpsRemovalTransformer will be integrated into the code generation pipeline in `code_generator.rs`:

- Run **after** import rewriting (to catch any self-references introduced)
- Run **before** final code generation
- Applied to each module's AST before bundling

### 3. Statement Filtering Strategy

Instead of modifying statements in-place, we'll:

1. Collect indices of statements to remove during traversal
2. Filter out these statements in a post-processing step
3. Preserve statement order and structure

## Test-Driven Development Plan

### Phase 1: Create Test Fixtures

Create fixtures in `crates/cribo/tests/fixtures/` to test each no-op pattern:

#### 1. `no_ops_self_references/`

```python
# main.py
x = 42
y = "hello"
x = x  # Should be removed
y = y  # Should be removed

def func():
    z = 10
    z = z  # Should be removed
    return z

class MyClass:
    attr = 1
    attr = attr  # Should be removed
```

#### 2. `no_ops_pass_statements/`

```python
# main.py
def necessary_pass():
    pass  # Should NOT be removed

def unnecessary_pass():
    x = 1
    pass  # Should be removed
    return x

class EmptyClass:
    pass  # Should NOT be removed

if True:
    x = 1
    pass  # Should be removed
```

#### 3. `no_ops_empty_expressions/`

```python
# main.py
42  # Should be removed
"not a docstring"  # Should be removed
None  # Should be removed
True  # Should be removed

def func():
    """This is a docstring"""  # Should NOT be removed
    42  # Should be removed
    return 1
```

#### 4. `no_ops_augmented_assignments/`

```python
# main.py
x = 10
y = 5.0
s = {1, 2, 3}

x += 0  # Should be removed
y *= 1  # Should be removed
s |= set()  # Should be removed

# Should NOT be removed (may have side effects with custom types)
class Counter:
    def __iadd__(self, other):
        print("Adding")
        return self

c = Counter()
c += 0  # Should NOT be removed
```

#### 5. `no_ops_combined/`

```python
# main.py
# Combination of multiple no-ops
import math

result = 42
result = result  # Should be removed

def process(data):
    data += 0  # Should be removed
    None  # Should be removed
    pass  # Should be removed
    return data

math = math  # Should be removed
```

### Phase 2: Mark Failing Tests

Initially mark these fixtures with `xfail_` prefix:

- `xfail_no_ops_self_references/`
- `xfail_no_ops_pass_statements/`
- etc.

### Phase 3: Run Tests and Capture Baseline

Run tests to see current output without no-op removal:

```bash
cargo test test_bundling_fixtures
```

## Implementation Details

### 1. Self-Reference Detection

```rust
fn is_self_reference_assignment(stmt: &StmtAssign) -> bool {
    // Check if single target, single value
    if stmt.targets.len() != 1 {
        return false;
    }

    // Check if target and value are both simple names
    if let (Expr::Name(target), Expr::Name(value)) = (&stmt.targets[0], &stmt.value) {
        target.id == value.id
            && matches!(target.ctx, ExprContext::Store)
            && matches!(value.ctx, ExprContext::Load)
    } else {
        false
    }
}
```

### 2. Pass Statement Analysis

```rust
fn is_unnecessary_pass(stmt: &StmtPass, context: &Context) -> bool {
    // Pass is necessary if it's the only statement in a block
    // or if removing it would create an empty required block
    !context.is_only_statement && !context.is_required_block
}
```

### 3. Empty Expression Detection

```rust
fn is_removable_expr_stmt(stmt: &StmtExpr) -> bool {
    match &stmt.value {
        Expr::NumberLiteral(_)
        | Expr::StringLiteral(_)
        | Expr::BytesLiteral(_)
        | Expr::BooleanLiteral(_)
        | Expr::NoneLiteral(_)
        | Expr::EllipsisLiteral(_) => !is_potential_docstring(stmt),
        _ => false,
    }
}
```

### 4. Augmented Assignment Analysis

```rust
fn is_identity_aug_assign(stmt: &StmtAugAssign) -> bool {
    // Only for simple name targets (no attributes/subscripts)
    if !matches!(stmt.target, Expr::Name(_)) {
        return false;
    }

    match (&stmt.op, &stmt.value) {
        (Operator::Add, Expr::NumberLiteral(n)) => n.value == 0,
        (Operator::Mult, Expr::NumberLiteral(n)) => n.value == 1,
        (Operator::BitOr, Expr::Call(call)) => is_empty_set_constructor(call),
        _ => false,
    }
}
```

## Safety Considerations

### What We WON'T Remove

1. **Attribute assignments**: `obj.x = obj.x` (may trigger descriptors)
2. **Subscript assignments**: `d[k] = d[k]` (may trigger `__getitem__`/`__setitem__`)
3. **Augmented assignments on custom types**: May have side effects via `__iadd__` etc.
4. **Function calls**: Even if they appear to be no-ops
5. **Import statements**: Even self-referential imports can have side effects

### Module vs. Function Scope

The transformer will track scope depth to apply different rules:

- Module-level: More aggressive removal
- Function/Class scope: More conservative

## Testing Strategy

1. **Snapshot Testing**: Use existing fixture framework
2. **Execution Testing**: Ensure bundled code executes identically
3. **Edge Cases**: Test with complex nested structures
4. **Performance**: Ensure transformer doesn't slow down bundling

## Success Criteria

1. All identified no-op patterns are removed
2. No behavioral changes in bundled code
3. Snapshots show cleaner output
4. All tests pass without xfail markers
5. No performance regression

## Future Enhancements

1. **Constant folding**: `x = 1 + 1` → `x = 2`
2. **Dead code elimination**: Unreachable code after return/raise
3. **Redundant import removal**: Beyond current unused import detection
4. **Common subexpression elimination**: Within safe boundaries

## Notes

- No-op removal will be beneficial for all fixtures by cleaning up any redundant operations introduced during bundling
- The feature will be always-on, with no configuration needed

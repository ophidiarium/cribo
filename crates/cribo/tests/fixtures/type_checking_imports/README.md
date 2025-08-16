# TYPE_CHECKING Imports

This fixture tests the handling of `typing.TYPE_CHECKING` blocks that contain imports used exclusively for type hints.

## What This Tests

TYPE_CHECKING blocks are a common Python pattern for importing types that are only needed for static type checking, not at runtime:

```python
if typing.TYPE_CHECKING:
    from module_b import TypeB  # Only used in type annotations
```

These imports are never executed at runtime (TYPE_CHECKING is always False), but are seen by type checkers like mypy.

## Test Cases

### `main.py`

Simple case with a single TYPE_CHECKING import used in function annotations.

### `advanced.py`

Multiple patterns of TYPE_CHECKING usage:

- Single import in TYPE_CHECKING block
- Multiple imports in TYPE_CHECKING block
- TYPE_CHECKING block with both imports and type aliases
- Nested TYPE_CHECKING blocks

### Supporting Modules

- `module_a.py`: Regular module with runtime functions
- `module_b.py`: Module providing type definitions
- `module_c.py`: Additional type definitions

## Expected Bundling Behavior

When bundling, TYPE_CHECKING imports require special handling since:

1. They're never executed at runtime (TYPE_CHECKING is False)
2. They're only used in string annotations or with `from __future__ import annotations`
3. The imported types may not be needed in the bundle if only used for static analysis

## Running the Test

```bash
# Bundle and run
cargo run -- --entry crates/cribo/tests/fixtures/type_checking_imports/main.py --stdout | python

# Run original (unbundled)
cd crates/cribo/tests/fixtures/type_checking_imports && python main.py
```

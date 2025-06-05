## Type Hint Stripping Task
We need to strip static type hints, as they are not meaningful in the bundled source code.
Minimal requirements:
- Strip all type hint metadata, including string literal annotations.
- Remove imports of `typing` and related modules.
- Eliminate code in `if typing.TYPE_CHECKING:` branches.
- Remove `from __future__ import annotations` statements.
- Rewrite `typing.cast(T, value)` calls to `value`.

To approach this task, we will start with research and creating an implementation strategy document / to-do list.

### Basics / Specifications

Read https://peps.python.org/pep-0484/ carefully to gain a deep and complete understanding of Python static type hints.
Use documents linked at https://peps.python.org/topic/typing/ when you need to dive deep on some particular topic.

### Previous work

Explore the codebase of Python-powered type stripping utilities source code. Your main target in this exploration is to understand complexity and collect testing examples, edge cases and non-trivial handling logic. Check the source code at following directories:

- references/type-strip/strip-hints
- references/type-strip/python-minifier
- references/type-strip/TypeStripper

### Type handling with `rustpython-parser` and `rustpython-ast`

Explore following directories which contains implementation of tools based on `rustpython-parser` and `rustpython-ast` which also dealing with static type hints. Your main objective here is to collect practical code samples to identify and efficiently handle type hints with our parsing library. Check source code at following directories:

- references/type-strip/ruff
- references/type-strip/pyrefly

### Understanding runtime behavior

## Explore the RustPython source code to gain further insight into how dead branches (type-hinting logic) are identified at runtime and to efficiently handle rewriting `if TYPE_CHECKING` branches. Check source code at:

- references/type-strip/RustPython

## Document

Output a comprehensive document at `docs/type-stripping-system-design.md` outlining requirements, challenges, and insights from the existing implementation that you have noticed.
Describe your rationale and decision on where to implement the type-stripping logic in our codebase (AST rewriter, emitter, unparser, or another component).
Document any specific considerations you learned about handling type hints with `rustpython-parser`.
Provide a list of edge-case tests required.
Write a detailed to-do list outlining how you will approach the feature implementation.

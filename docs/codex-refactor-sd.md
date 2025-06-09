# Exec-Free Bundling System Design

## Overview

Cribo currently generates a dynamic Python loader that uses `exec` at runtime to define and register inlined modules in `sys.modules`. While this mechanism simplifies the bundler implementation, it has two major drawbacks:

- **Runtime overhead**: Each module body must be parsed and executed via `exec`, increasing startup latency.
- **Reduced readability**: The final bundle relies on a runtime loader scaffold rather than a directly consumable Python source, making it harder for humans and AI models (e.g. LLMs) to read and understand.

This document proposes a refactoring to eliminate `exec` calls in the bundled output. Instead, we will statically flatten and transform all first-party modules at bundle time into a single, self-contained Python script that can be executed directly without dynamic execution scaffolding.

## Table of Contents

1. [Current Architecture and Limitations](#current-architecture-and-limitations)
2. [Proposed Architecture](#proposed-architecture)
3. [AST Flattening and Transformation Workflow](#ast-flattening-and-transformation-workflow)
4. [Edge Case Considerations](#edge-case-considerations)
5. [Detailed Implementation Plan](#detailed-implementation-plan)
6. [Testing and Validation](#testing-and-validation)
7. [Future Considerations](#future-considerations)

---

## Current Architecture and Limitations

In the existing implementation (see [`emit.rs`]), Cribo emits a loader scaffold that collects module source strings and uses `exec` to populate each module at runtime. A simplified pattern looks like:

```python
bundled_code = {
    "package.module": "...source of package/module.py...",
    "package.sub": "...source..."
}

import sys, types
for name, src in bundled_code.items():
    module = types.ModuleType(name)
    sys.modules[name] = module
    exec(src, module.__dict__)

# Invoke the entry point
from package.main import main
main()
```

While flexible, this design introduces:

- **Dynamic parsing**: Python must re-parse each module source at runtime.
- **Token indirection**: An outer CLI scaffolding obscures the actual code from static readers and LLMs.
- **Testing complexity**: Snapshot tests validate emitted loader code rather than the flattened Python AST.

---

## Proposed Architecture

Replace the runtime loader and `exec` scaffold with a static AST-based flattening stage in the Rust bundler. The high-level pipeline becomes:

```text
┌─────────────────┐
│ Dependency Graph│
│ & AST Parsing   │
└─────────────────┘
          ↓
┌───────────────────────────┐
│ AST Flattening & Rewriting│
└───────────────────────────┘
          ↓
┌───────────────────────────┐
│ Direct AST Unparsing into │
│ Single Python Script      │
└───────────────────────────┘
          ↓
┌─────────────────┐
│ Final Bundle.py │
└─────────────────┘
```

Key changes:

- **AST Flattening**: Inline module ASTs into one combined AST, rewriting import statements to resolve local references.
- **Static Unparse**: Generate final code from the combined AST, eliminating any use of `exec` or dynamic loaders.
- **Cleaner Output**: The resulting bundle is a plain Python script with inlined definitions and no runtime scaffolding.

---

## AST Flattening and Transformation Workflow

The flattening stage performs these steps for each module in topological order:

1. **Parse** each module source into an AST (using `ruff_python_parser`).
2. **Rename module-level symbols** (optional namespace prefix) to avoid collisions when inlining multiple modules into the same global scope.
3. **Rewrite import statements**:
   - Remove imports of first-party modules in the bundle.
   - Redirect imports of bundled modules to direct symbol references.
4. **Collect and append** the transformed AST nodes in module-order into a single, new AST tree.
5. **Emit** the combined AST via an unparser back into Python source.

### Symbol Namespace Handling

To avoid name collisions when flattening multiple modules, each inlined module may be prefixed with a unique namespace or use nested classes/closures. For example:

```python
# original mypkg/utils.py
def helper(): ...

# inlined with prefix
def __mypkg_utils__helper(): ...
```

Advanced name-preservation strategies may rely on AST scopes to minimize prefixing.

### Import Rewriting

Given an AST node `ImportFrom(module="mypkg.utils", names=[alias("helper")])`, the rewriter will:

- Remove the import node entirely.
- Replace `helper(...)` calls in the code with the inlined symbol `__mypkg_utils__helper(...)`.

---

## Implementation Plan

1. **Add a new module** `src/flatten.rs` (or `src/ast_flatten.rs`) containing the core flattening logic and AST rewrite utilities.
2. **Extend `bundler.rs`** to invoke the flattening stage after dependency resolution and AST parsing.
3. **Refactor `emit.rs`**:
   - Remove the runtime loader (`exec`) scaffolding.
   - Replace with a single unparse call on the combined AST.
4. **Namespace prefix strategy**:
   - Implement a configurable prefixing mechanism to avoid symbol collisions.
   - Expose CLI options (future) to control prefix patterns.
5. **CLI adjustments**:
   - Introduce `--exec-free` flag for opt-in migration.
   - Eventually flip default to exec-free and remove legacy path.
6. **Documentation**:
   - Update user-facing docs (`README.md`) to reflect the new bundle shape.
   - Link to this system design doc in `docs/`.

---

## Migration Strategy

To ensure backward compatibility and a smooth rollout:

| Stage            | Action                                             |
| ---------------- | -------------------------------------------------- |
| Opt-in testing   | Add `--exec-free` flag; legacy behavior is default |
| Snapshot updates | Add new fixtures demonstrating exec-free output    |
| Gradual rollout  | Flip default once stabilized and tested            |
| Legacy removal   | Remove exec-based code path after full adoption    |

---

## Testing and Validation

- **Snapshot Tests**: Add new fixtures under `crates/cribo/tests/fixtures/bundling/exec_free/` showing identical behavior with no exec loader.
- **Unit Tests**: Cover AST flattening, import rewriting, and prefixing logic in `flatten.rs`.
- **Performance Benchmarks**: Measure startup latency and bundle generation time before and after exec-free migration.
- **Backwards Compatibility**: Ensure existing fixture suite passes under both legacy and exec-free modes.

## Future Considerations

- **Minification & Comment Stripping**: Post-process the final AST to remove comments or compress code.
- **Source Map Generation**: Emit mapping metadata to correlate bundled lines back to original files.
- **Parallel AST Processing**: Leverage parallel parsing and flattening for large codebases.
- **Configurable Namespace Strategies**: Offer advanced scoping modes (e.g., nested modules, closures).

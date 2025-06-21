# AST Rewriting Collisions: Missing Patterns Spotter

This document analyzes three AST-rewriting test fixtures in `crates/cribo/tests/fixtures`:

- `ast_rewriting_globals_collision`: focused on basic global-variable conflicts
- `ast_rewriting_symbols_collision`: focused on naming/signature shadowing conflicts
- `ast_rewriting_mixed_collisions`: a superset combining both and exercising additional patterns

> We identify patterns exercised by the mixed fixture but missing in the simpler ones.

## 1. Dynamic Global Name Access via `globals()`

Mixed fixture uses the built-in `globals()` to retrieve module-level names dynamically:

- In `main.py` to access a shadowed global `result` after local shadowing【F:crates/cribo/tests/fixtures/ast_rewriting_mixed_collisions/main.py†L100】.
- In `models/base.py` to recover the global `validate`, `process`, and `Logger` after they are shadowed by parameters【F:crates/cribo/tests/fixtures/ast_rewriting_mixed_collisions/models/base.py†L121-L129】.
- In `services/auth/manager.py` inside `validate()` to pick up the global `validate` different from the local function【F:crates/cribo/tests/fixtures/ast_rewriting_mixed_collisions/services/auth/manager.py†L90-L99】
  and inside `AuthManager.add_user()` to grab the `User` class【F:crates/cribo/tests/fixtures/ast_rewriting_mixed_collisions/services/auth/manager.py†L131-L133】.
- In `models/user.py` to recover global `Logger` in `process_user()`【F:crates/cribo/tests/fixtures/ast_rewriting_mixed_collisions/models/user.py†L113】
  and in `complex_operation()` when shadowed by parameters【F:crates/cribo/tests/fixtures/ast_rewriting_mixed_collisions/models/user.py†L190-L200】.

The `ast_rewriting_symbols_collision` fixture contains no `globals()` calls, and the `ast_rewriting_globals_collision` fixture only uses a single shallow lookup in `main.py` to resolve a local vs global `result`【F:crates/cribo/tests/fixtures/ast_rewriting_globals_collision/main.py†L100】.

## 2. Mutation of Module-Level State with `global`

The mixed fixture thoroughly exercises `global` statements to mutate module-level variables in functions/methods:

- In `core/database/connection.py`: `global connection` in `Connection.connect()` and `global result` in `process()` plus `global connection` in the module-level `connect()`【F:crates/cribo/tests/fixtures/ast_rewriting_mixed_collisions/core/database/connection.py†L27-L31】【F:crates/cribo/tests/fixtures/ast_rewriting_mixed_collisions/core/database/connection.py†L34-L53】【F:crates/cribo/tests/fixtures/ast_rewriting_mixed_collisions/core/database/connection.py†L64-L69】.
- In `core/utils/helpers.py`: `global result` in `process()` that increments a module counter【F:crates/cribo/tests/fixtures/ast_rewriting_mixed_collisions/core/utils/helpers.py†L35-L48】.
- In `models/base.py`: multiple `global result` usages in `BaseModel.initialize()`, module-level `initialize()`, and `process()`【F:crates/cribo/tests/fixtures/ast_rewriting_mixed_collisions/models/base.py†L39-L42】【F:crates/cribo/tests/fixtures/ast_rewriting_mixed_collisions/models/base.py†L47-L58】【F:crates/cribo/tests/fixtures/ast_rewriting_mixed_collisions/models/base.py†L68-L77】.
- In `models/user.py`: `global connection` in `connect()` and `global result` in `process_user()`【F:crates/cribo/tests/fixtures/ast_rewriting_mixed_collisions/models/user.py†L103-L110】【F:crates/cribo/tests/fixtures/ast_rewriting_mixed_collisions/models/user.py†L110-L127】.
- In `services/auth/manager.py`: `global result` in `process()` that updates the auth result state【F:crates/cribo/tests/fixtures/ast_rewriting_mixed_collisions/services/auth/manager.py†L62-L81】.
- In `main.py`: `global connection` to capture the local Connection instance into module scope【F:crates/cribo/tests/fixtures/ast_rewriting_mixed_collisions/main.py†L55】.

While `ast_rewriting_globals_collision` has only a handful of `global` examples (e.g. `global process_count`, `global Connection`, `global connection` in main) and `ast_rewriting_symbols_collision` has none, the mixed fixture covers this pattern extensively.

## 3. Combined Absolute and Relative Import Patterns

The mixed fixture consolidates import patterns in a single module:

- Deep absolute import from `models.user` plus a relative import from `..utils.helpers` in `core/database/connection.py`【F:crates/cribo/tests/fixtures/ast_rewriting_mixed_collisions/core/database/connection.py†L5-L6】.

Neither simpler fixture packs both deep absolute + relative import in the same module.

## 4. Recovery of Shadowed Names via `globals()`

Beyond simply shadowing names (covered by `ast_rewriting_symbols_collision`), the mixed fixture demonstrates retrieving the original definitions:

- In `models/base.shadow_test()`, after shadowing `validate`, `process`, and `Logger` via parameters, it rebinds them to the globals【F:crates/cribo/tests/fixtures/ast_rewriting_mixed_collisions/models/base.py†L121-L129】.
- In `models/user.complex_operation()`, parameter shadowing is followed by re-import from globals to get the intended class definitions【F:crates/cribo/tests/fixtures/ast_rewriting_mixed_collisions/models/user.py†L190-L200】.
- In `AuthManager.add_user()`, the `User` reference is recovered via `globals()` despite a local assignment【F:crates/cribo/tests/fixtures/ast_rewriting_mixed_collisions/services/auth/manager.py†L131-L136】.

Neither of the first two fixtures combines shadowing with dynamic recovery via `globals()`.

## Summary of Missing Coverage

| Pattern                                    | globals_collision | symbols_collision | mixed_collisions |
| ------------------------------------------ | :---------------: | :---------------: | :--------------: |
| `globals()` dynamic lookup                 |  Partial (main)   |        ❌         |        ✅        |
| Extensive `global` assignment/mutation     |      Partial      |        ❌         |        ✅        |
| Combined deep absolute + relative imports  |        ❌         |        ✅         |        ✅        |
| Recovery of shadowed names via `globals()` |        ❌         |        ❌         |        ✅        |
| Parameter name shadowing                   |        ❌         |        ✅         |        ✅        |
| Self-referential aliasing (`x = x`)        |        ❌         |        ✅         |        ✅        |

By exercising these additional challenges in `ast_rewriting_mixed_collisions`, the mixed fixture ensures our AST rewriter can handle dynamic name lookups, comprehensive global state manipulation, and name-recovery in the face of heavy shadowing — scenarios not fully covered by either simpler fixture alone.

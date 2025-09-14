# Split Import Transformer, Part 3 — Refactor `transform_statements`

Objective

- Reduce the size and complexity of `transform_statements` by extracting each statement-kind branch into focused functions under `handlers/statements.rs`, without changing behavior.
- Use a bottom-up extraction order to avoid line-number drift in the large source file.
- After each extraction: compile (`cargo check`), run tests (`cargo nextest run`), then commit.

Scope

- File: `crates/cribo/src/code_generator/import_transformer/mod.rs`
  - Function: `fn transform_statements(&mut self, stmts: &mut Vec<Stmt>)` at lines 503–933.
  - This function iterates the top-level statements, transforms import statements in-place, and recurses into all other statement kinds.

Destination

- New file: `crates/cribo/src/code_generator/import_transformer/handlers/statements.rs`.
- Type and functions:
  - `pub struct StatementsHandler;`
  - One handler function per statement kind, taking `&mut RecursiveImportTransformer` and a mutable reference to the specific stmt node, plus any required locals (e.g., `&mut i`, `stmts`, etc.) when it needs to affect control flow.
  - Prefer signatures that don’t return values unless necessary; follow current side-effect pattern (mutating AST in-place).

Imports

- Add to `mod.rs` near existing handler imports: `handlers::statements::StatementsHandler`.
- Inside `handlers/statements.rs`, import:
  - `super::super::Bundler` if needed, but primarily `super::super::import_transformer::RecursiveImportTransformer`.
  - `ruff_python_ast::*` types used by each handler.

Bottom‑Up Extraction Steps (exact ranges and names)

General call-site shape for each arm

- Replace the inline block within the `match` arm by a single call to `StatementsHandler::<fn>(self, &mut <stmt_node>);` followed by `i += 1;` if the original arm ended with `i += 1;` and not `continue;`.
- Preserve any early `continue` behavior by returning early from the handler or having the handler mutate the required node and communicate whether the caller should `continue` (see Step 1).

Safeguard

- Do not change the loop/iterator scaffolding (`i`, `stmts.remove/insert`, import path). Only replace the arm bodies.

Step 1 — Extract Assert/Return/Raise/Expr/AugAssign/AnnAssign (bottom tail) — COMPLETED

- Source ranges in `mod.rs` inside `transform_statements` match at lines:
  - Assert: 922–927 → `StatementsHandler::handle_assert(self, assert_stmt)`
  - Raise: 914–921 → `StatementsHandler::handle_raise(self, raise_stmt)`
  - Return: 909–913 → `StatementsHandler::handle_return(self, ret_stmt)`
  - Expr: 906–908 → `StatementsHandler::handle_expr_stmt(self, expr_stmt)`
  - AugAssign: 902–905 → `StatementsHandler::handle_aug_assign(self, aug_assign)`
  - AnnAssign: 890–901 → `StatementsHandler::handle_ann_assign(self, ann_assign)`
- New functions in `handlers/statements.rs`:
  - `pub(in crate::code_generator::import_transformer) fn handle_assert(t: &mut RecursiveImportTransformer, s: &mut ruff_python_ast::StmtAssert)`
  - `pub(in crate::code_generator::import_transformer) fn handle_raise(t: &mut RecursiveImportTransformer, s: &mut ruff_python_ast::StmtRaise)`
  - `pub(in crate::code_generator::import_transformer) fn handle_return(t: &mut RecursiveImportTransformer, s: &mut ruff_python_ast::StmtReturn)`
  - `pub(in crate::code_generator::import_transformer) fn handle_expr_stmt(t: &mut RecursiveImportTransformer, s: &mut ruff_python_ast::StmtExpr)`
  - `pub(in crate::code_generator::import_transformer) fn handle_aug_assign(t: &mut RecursiveImportTransformer, s: &mut ruff_python_ast::StmtAugAssign)`
  - `pub(in crate::code_generator::import_transformer) fn handle_ann_assign(t: &mut RecursiveImportTransformer, s: &mut ruff_python_ast::StmtAnnAssign)`
- Behavior: move bodies verbatim (only replace `self` with `t`).
- Validation + Commit:
  - Completed: code moved to `handlers/statements.rs` and call sites updated.
  - Tests: `cargo nextest run` passed (132 tests, 1 skipped).
  - Commit (pending in this branch): `refactor(import_transformer): extract simple leaf stmt handlers to handlers/statements.rs`

Step 2 — Extract Try/With/For/While (loop and suite transforms) — COMPLETED

- Source ranges:
  - Try: 856–889 → `StatementsHandler::handle_try(self, try_stmt)`
  - With: 850–855 → `StatementsHandler::handle_with(self, with_stmt)`
  - For: 830–849 → `StatementsHandler::handle_for(self, for_stmt)`
  - While: 825–829 → `StatementsHandler::handle_while(self, while_stmt)`
- New functions:
  - `handle_try(t: &mut RecursiveImportTransformer, s: &mut ruff_python_ast::StmtTry)`
  - `handle_with(t: &mut RecursiveImportTransformer, s: &mut ruff_python_ast::StmtWith)`
  - `handle_for(t: &mut RecursiveImportTransformer, s: &mut ruff_python_ast::StmtFor)`
  - `handle_while(t: &mut RecursiveImportTransformer, s: &mut ruff_python_ast::StmtWhile)`
- Notes:
  - Preserve the “add pass when empty” behavior in try-body and except-body.
  - For `for`, preserve the local loop-variable tracking before recursing.
- Validation + Commit:
  - Completed: code moved and call sites updated for While, For, With, Try.
  - Tests: `cargo nextest run` passed (132 tests, 1 skipped).
  - Commit (pending in this branch): `refactor(import_transformer): extract loop/suite stmt handlers to handlers/statements.rs`

Step 3 — Extract If (including TYPE_CHECKING and elif/else handling) — COMPLETED

- Source range: 794–824 → `StatementsHandler::handle_if(self, if_stmt)`
- New function:
  - `handle_if(t: &mut RecursiveImportTransformer, s: &mut ruff_python_ast::StmtIf)`
- Notes: keep non-empty body insertions for TYPE_CHECKING, elif/else.
- Validation + Commit:
  - Completed: moved to `handlers/statements.rs` and call sites updated.
  - Tests: `cargo nextest run` passed.
  - Commit: `refactor(import_transformer): extract if-statement handler to handlers/statements.rs`

Step 4 — Extract ClassDef — COMPLETED

- Source range: 779–793 → `StatementsHandler::handle_class_def(self, class_def)`
- New function: `handle_class_def(t: &mut RecursiveImportTransformer, s: &mut ruff_python_ast::StmtClassDef)`
- Notes: keep decorator transform and delegate base classes to existing `transform_class_bases` (do not move `transform_class_bases` in this step).
- Validation + Commit:
  - Completed: moved to `handlers/statements.rs` and call sites updated.
  - Tests: `cargo nextest run` passed.
  - Commit: `refactor(import_transformer): extract class definition handler to handlers/statements.rs`

Step 5 — Extract FunctionDef (parameters, decorators, scope save/restore) — COMPLETED

- Source range: 622–778 → `StatementsHandler::handle_function_def(self, func_def)`
- New function: `handle_function_def(t: &mut RecursiveImportTransformer, s: &mut ruff_python_ast::StmtFunctionDef)`
- Notes:
  - Keep exact sequence: decorators → params/annotations/defaults → returns → save/restore locals and wrapper imports → transform body → restore state.
  - Preserve `self.state.is_wrapper_init` and locals tracking behavior intact.
- Validation + Commit:
  - Completed: moved to `handlers/statements.rs` and call sites updated.
  - Tests: `cargo nextest run` passed.
  - Commit: `refactor(import_transformer): extract function definition handler to handlers/statements.rs`

Step 6 — Extract Assign (LHS tracking, importlib handling, targets/values) — COMPLETED

- Source range: 562–621.
- This arm has a `continue;` at the end; reflect that in the call site to avoid double `i += 1;`.
- Call site change:
  - Replace the `Assign` arm with:
    - `if StatementsHandler::handle_assign(self, assign_stmt) { i += 1; } else { /* handler already advanced control flow via continue */ continue; }`
  - Alternatively, return a boolean `advance` from handler: `true` -> caller does `i += 1`, `false` -> caller `continue;`.
- New function: `handle_assign(t: &mut RecursiveImportTransformer, s: &mut ruff_python_ast::StmtAssign) -> bool`
  - Move entire block verbatim; return `false` at the end (mirrors `continue;`). If refactoring to return `true`, adjust the body to not do the early `continue` locally.
- Validation + Commit:
  - Completed: moved to `handlers/statements.rs` with return flag to preserve `continue`.
  - Tests: `cargo nextest run` passed.
  - Commit: `refactor(import_transformer): extract assignment handler to handlers/statements.rs`

Step 7 — Optional: Extract import-path handling into helpers (no behavior change)

- Top of loop (503–558) remains in place to avoid churn.
- Consider tiny helpers inside `handlers/statements.rs` that are purely functional, if needed later:
  - `is_import_stmt(&Stmt) -> bool`
  - `is_hoisted_import(&Bundler, &Stmt) -> bool` (already exists via `import_deduplicator`).
- Defer this until after major arms are extracted and tests are green.

File Skeleton for `handlers/statements.rs`

```
use ruff_python_ast::*;
use crate::code_generator::import_transformer::RecursiveImportTransformer;

pub struct StatementsHandler;

impl StatementsHandler {
    pub(in crate::code_generator::import_transformer)
    fn handle_ann_assign(t: &mut RecursiveImportTransformer, s: &mut StmtAnnAssign) { /* moved code */ }
    pub(in crate::code_generator::import_transformer)
    fn handle_aug_assign(t: &mut RecursiveImportTransformer, s: &mut StmtAugAssign) { /* moved code */ }
    pub(in crate::code_generator::import_transformer)
    fn handle_expr_stmt(t: &mut RecursiveImportTransformer, s: &mut StmtExpr) { /* moved code */ }
    pub(in crate::code_generator::import_transformer)
    fn handle_return(t: &mut RecursiveImportTransformer, s: &mut StmtReturn) { /* moved code */ }
    pub(in crate::code_generator::import_transformer)
    fn handle_raise(t: &mut RecursiveImportTransformer, s: &mut StmtRaise) { /* moved code */ }
    pub(in crate::code_generator::import_transformer)
    fn handle_assert(t: &mut RecursiveImportTransformer, s: &mut StmtAssert) { /* moved code */ }
    pub(in crate::code_generator::import_transformer)
    fn handle_try(t: &mut RecursiveImportTransformer, s: &mut StmtTry) { /* moved code */ }
    pub(in crate::code_generator::import_transformer)
    fn handle_with(t: &mut RecursiveImportTransformer, s: &mut StmtWith) { /* moved code */ }
    pub(in crate::code_generator::import_transformer)
    fn handle_for(t: &mut RecursiveImportTransformer, s: &mut StmtFor) { /* moved code */ }
    pub(in crate::code_generator::import_transformer)
    fn handle_while(t: &mut RecursiveImportTransformer, s: &mut StmtWhile) { /* moved code */ }
    pub(in crate::code_generator::import_transformer)
    fn handle_if(t: &mut RecursiveImportTransformer, s: &mut StmtIf) { /* moved code */ }
    pub(in crate::code_generator::import_transformer)
    fn handle_class_def(t: &mut RecursiveImportTransformer, s: &mut StmtClassDef) { /* moved code */ }
    pub(in crate::code_generator::import_transformer)
    fn handle_function_def(t: &mut RecursiveImportTransformer, s: &mut StmtFunctionDef) { /* moved code */ }
    pub(in crate::code_generator::import_transformer)
    fn handle_assign(t: &mut RecursiveImportTransformer, s: &mut StmtAssign) -> bool { /* moved code; return whether caller should i+=1 */ }
}
```

Testing & Commits

- After each step: `cargo check` then `cargo nextest run`.
- Keep commit messages aligned with the step descriptions.

Rollback Strategy

- If any step causes snapshot drift or failures, revert that single change and re-apply by splitting into even smaller handlers (e.g., extract only Try, then later With/For/While).
- Avoid modifying control-flow scaffolding (`i`, `continue`) more than necessary; prefer handlers that report whether the caller should advance `i`.

Post-Refactor Follow-ups

- Consider moving `transform_class_bases` and similar helpers into `handlers/statements.rs` once call sites are consolidated.
- Consider moving the import-detection top-of-loop scaffolding into a helper when the function becomes short enough that line drift is no longer a concern.

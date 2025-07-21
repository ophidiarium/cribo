//! Variable usage collection visitor
//!
//! This visitor traverses the AST to collect information about variable usage,
//! including reads, writes, deletions, and global/nonlocal declarations.

use ruff_python_ast::{
    ExceptHandler, Expr, ModModule, Stmt,
    visitor::{Visitor, walk_expr, walk_stmt},
};
use ruff_text_size::TextRange;

use crate::{
    analyzers::types::{CollectedVariables, UsageType, VariableUsage},
    types::FxIndexSet,
};

/// Variable collection visitor
pub struct VariableCollector {
    /// Collected data
    collected: CollectedVariables,
    /// Current scope stack
    scope_stack: Vec<String>,
    /// Whether we're in a deletion context
    in_deletion: bool,
    /// Whether we're on the left side of an assignment
    in_assignment_target: bool,
}

impl Default for VariableCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl VariableCollector {
    /// Create a new variable collector
    pub fn new() -> Self {
        Self {
            collected: CollectedVariables::default(),
            scope_stack: vec!["<module>".to_string()],
            in_deletion: false,
            in_assignment_target: false,
        }
    }

    /// Analyze a module and return collected variables
    pub fn analyze(module: &ModModule) -> CollectedVariables {
        let mut collector = Self::new();
        collector.visit_body(&module.body);
        collector.collected
    }

    /// Build the current scope path as a single string
    fn current_scope_path(&self) -> String {
        self.scope_stack.join(".")
    }

    /// Record a variable usage
    fn record_usage(&mut self, name: &str, usage_type: UsageType, location: TextRange) {
        self.collected.usages.push(VariableUsage {
            name: name.to_string(),
            usage_type,
            location,
            scope: self.current_scope_path(),
        });

        // Track referenced vars for quick lookup
        if matches!(usage_type, UsageType::Read | UsageType::Write) {
            self.collected.referenced_vars.insert(name.to_string());
        }
    }

    /// Process assignment targets
    fn process_assignment_target(&mut self, target: &Expr) {
        let prev_in_assignment = self.in_assignment_target;
        self.in_assignment_target = true;

        match target {
            Expr::Name(name) => {
                self.record_usage(&name.id, UsageType::Write, name.range);
            }
            Expr::Tuple(tuple) => {
                for elt in &tuple.elts {
                    self.process_assignment_target(elt);
                }
            }
            Expr::List(list) => {
                for elt in &list.elts {
                    self.process_assignment_target(elt);
                }
            }
            Expr::Subscript(sub) => {
                // The subscript target is being read, not written
                self.in_assignment_target = false;
                self.visit_expr(&sub.value);
                self.visit_expr(&sub.slice);
                self.in_assignment_target = true;
            }
            Expr::Attribute(attr) => {
                // The attribute target is being read, not written
                self.in_assignment_target = false;
                self.visit_expr(&attr.value);
                self.in_assignment_target = true;
            }
            _ => {
                // For other expressions, visit normally
                self.in_assignment_target = false;
                self.visit_expr(target);
                self.in_assignment_target = true;
            }
        }

        self.in_assignment_target = prev_in_assignment;
    }

    /// Collect variables in an expression (static helper for compatibility)
    pub fn collect_vars_in_expr(expr: &Expr, vars: &mut FxIndexSet<String>) {
        match expr {
            Expr::Name(name) => {
                vars.insert(name.id.to_string());
            }
            Expr::Call(call) => {
                Self::collect_vars_in_expr(&call.func, vars);
                for arg in call.arguments.args.iter() {
                    Self::collect_vars_in_expr(arg, vars);
                }
                for keyword in call.arguments.keywords.iter() {
                    Self::collect_vars_in_expr(&keyword.value, vars);
                }
            }
            Expr::Attribute(attr) => {
                Self::collect_vars_in_expr(&attr.value, vars);
            }
            Expr::BinOp(binop) => {
                Self::collect_vars_in_expr(&binop.left, vars);
                Self::collect_vars_in_expr(&binop.right, vars);
            }
            Expr::UnaryOp(unaryop) => {
                Self::collect_vars_in_expr(&unaryop.operand, vars);
            }
            Expr::BoolOp(boolop) => {
                for value in boolop.values.iter() {
                    Self::collect_vars_in_expr(value, vars);
                }
            }
            Expr::Compare(compare) => {
                Self::collect_vars_in_expr(&compare.left, vars);
                for comparator in compare.comparators.iter() {
                    Self::collect_vars_in_expr(comparator, vars);
                }
            }
            Expr::List(list) => {
                for elt in list.elts.iter() {
                    Self::collect_vars_in_expr(elt, vars);
                }
            }
            Expr::Tuple(tuple) => {
                for elt in tuple.elts.iter() {
                    Self::collect_vars_in_expr(elt, vars);
                }
            }
            Expr::Dict(dict) => {
                for item in dict.items.iter() {
                    if let Some(key) = &item.key {
                        Self::collect_vars_in_expr(key, vars);
                    }
                    Self::collect_vars_in_expr(&item.value, vars);
                }
            }
            Expr::Subscript(sub) => {
                Self::collect_vars_in_expr(&sub.value, vars);
                Self::collect_vars_in_expr(&sub.slice, vars);
            }
            Expr::If(if_expr) => {
                Self::collect_vars_in_expr(&if_expr.test, vars);
                Self::collect_vars_in_expr(&if_expr.body, vars);
                Self::collect_vars_in_expr(&if_expr.orelse, vars);
            }
            _ => {}
        }
    }

    /// Collect global declarations from a function body (static helper)
    pub fn collect_function_globals(body: &[Stmt]) -> FxIndexSet<String> {
        let mut function_globals = FxIndexSet::default();
        for stmt in body {
            if let Stmt::Global(global_stmt) = stmt {
                for name in &global_stmt.names {
                    function_globals.insert(name.to_string());
                }
            }
        }
        function_globals
    }

    /// Collect variables referenced in statements (static helper for compatibility)
    pub fn collect_referenced_vars(stmts: &[Stmt], vars: &mut FxIndexSet<String>) {
        for stmt in stmts {
            Self::collect_vars_in_stmt(stmt, vars);
        }
    }

    /// Collect variable names referenced in a statement (static helper)
    fn collect_vars_in_stmt(stmt: &Stmt, vars: &mut FxIndexSet<String>) {
        match stmt {
            Stmt::Expr(expr_stmt) => Self::collect_vars_in_expr(&expr_stmt.value, vars),
            Stmt::Return(ret) => {
                if let Some(value) = &ret.value {
                    Self::collect_vars_in_expr(value, vars);
                }
            }
            Stmt::Assign(assign) => {
                Self::collect_vars_in_expr(&assign.value, vars);
            }
            Stmt::If(if_stmt) => {
                Self::collect_vars_in_expr(&if_stmt.test, vars);
                Self::collect_referenced_vars(&if_stmt.body, vars);
                for clause in &if_stmt.elif_else_clauses {
                    if let Some(condition) = &clause.test {
                        Self::collect_vars_in_expr(condition, vars);
                    }
                    Self::collect_referenced_vars(&clause.body, vars);
                }
            }
            Stmt::For(for_stmt) => {
                Self::collect_vars_in_expr(&for_stmt.iter, vars);
                Self::collect_referenced_vars(&for_stmt.body, vars);
                Self::collect_referenced_vars(&for_stmt.orelse, vars);
            }
            Stmt::While(while_stmt) => {
                Self::collect_vars_in_expr(&while_stmt.test, vars);
                Self::collect_referenced_vars(&while_stmt.body, vars);
                Self::collect_referenced_vars(&while_stmt.orelse, vars);
            }
            Stmt::Try(try_stmt) => {
                Self::collect_referenced_vars(&try_stmt.body, vars);
                for handler in &try_stmt.handlers {
                    let ExceptHandler::ExceptHandler(eh) = handler;
                    Self::collect_referenced_vars(&eh.body, vars);
                }
                Self::collect_referenced_vars(&try_stmt.orelse, vars);
                Self::collect_referenced_vars(&try_stmt.finalbody, vars);
            }
            Stmt::With(with_stmt) => {
                for item in &with_stmt.items {
                    Self::collect_vars_in_expr(&item.context_expr, vars);
                }
                Self::collect_referenced_vars(&with_stmt.body, vars);
            }
            _ => {}
        }
    }
}

impl<'a> Visitor<'a> for VariableCollector {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(func) => {
                // Enter function scope
                self.scope_stack.push(func.name.to_string());

                // Collect global declarations
                for stmt in &func.body {
                    if let Stmt::Global(global_stmt) = stmt {
                        let func_name = func.name.to_string();
                        let globals_set = self
                            .collected
                            .function_globals
                            .entry(func_name)
                            .or_default();
                        for name in &global_stmt.names {
                            globals_set.insert(name.to_string());
                        }
                    }
                }

                // Visit function body
                walk_stmt(self, stmt);

                // Exit function scope
                self.scope_stack.pop();
            }
            Stmt::ClassDef(class) => {
                // Enter class scope
                self.scope_stack.push(class.name.to_string());
                walk_stmt(self, stmt);
                self.scope_stack.pop();
            }
            Stmt::Assign(assign) => {
                // Visit value first (it's being read)
                self.visit_expr(&assign.value);

                // Process targets (they're being written)
                for target in &assign.targets {
                    self.process_assignment_target(target);
                }
            }
            Stmt::AnnAssign(ann_assign) => {
                // Visit annotation
                self.visit_expr(&ann_assign.annotation);

                // Visit value if present
                if let Some(value) = &ann_assign.value {
                    self.visit_expr(value);
                }

                // Process target
                self.process_assignment_target(&ann_assign.target);
            }
            Stmt::AugAssign(aug_assign) => {
                // For augmented assignment, target is both read and written
                if let Expr::Name(name) = &*aug_assign.target {
                    self.record_usage(&name.id, UsageType::Read, name.range);
                }

                // Visit value
                self.visit_expr(&aug_assign.value);

                // Process target for write
                self.process_assignment_target(&aug_assign.target);
            }
            Stmt::Delete(delete) => {
                self.in_deletion = true;
                for target in &delete.targets {
                    if let Expr::Name(name) = target {
                        self.record_usage(&name.id, UsageType::Delete, name.range);
                    } else {
                        self.visit_expr(target);
                    }
                }
                self.in_deletion = false;
            }
            Stmt::Global(global_stmt) => {
                for name in &global_stmt.names {
                    self.record_usage(name, UsageType::GlobalDeclaration, global_stmt.range);
                }
            }
            Stmt::Nonlocal(nonlocal_stmt) => {
                for name in &nonlocal_stmt.names {
                    self.record_usage(name, UsageType::NonlocalDeclaration, nonlocal_stmt.range);
                }
            }
            _ => walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Name(name) => {
                if !self.in_assignment_target && !self.in_deletion {
                    self.record_usage(&name.id, UsageType::Read, name.range);
                }
            }
            _ => walk_expr(self, expr),
        }
    }
}

#[cfg(test)]
mod tests {
    use ruff_python_parser::parse_module;

    use super::*;

    #[test]
    fn test_variable_reads() {
        let code = r#"
x = 1
y = x + 2
print(y)
"#;
        let parsed = parse_module(code).unwrap();
        let module = parsed.into_syntax();
        let collected = VariableCollector::analyze(&module);

        // Check that x and y are referenced
        assert!(collected.referenced_vars.contains("x"));
        assert!(collected.referenced_vars.contains("y"));
        assert!(collected.referenced_vars.contains("print"));

        // Check usage types
        let x_usages: Vec<_> = collected.usages.iter().filter(|u| u.name == "x").collect();
        assert_eq!(x_usages.len(), 2); // 1 write, 1 read

        let y_usages: Vec<_> = collected.usages.iter().filter(|u| u.name == "y").collect();
        assert_eq!(y_usages.len(), 2); // 1 write, 1 read
    }

    #[test]
    fn test_global_declarations() {
        let code = r#"
def foo():
    global x, y
    x = 1
    y = 2
"#;
        let parsed = parse_module(code).unwrap();
        let module = parsed.into_syntax();
        let collected = VariableCollector::analyze(&module);

        // Check function globals
        let foo_globals = collected.function_globals.get("foo").unwrap();
        assert!(foo_globals.contains("x"));
        assert!(foo_globals.contains("y"));

        // Check global declarations
        let global_decls: Vec<_> = collected
            .usages
            .iter()
            .filter(|u| matches!(u.usage_type, UsageType::GlobalDeclaration))
            .collect();
        assert_eq!(global_decls.len(), 2);
    }

    #[test]
    fn test_augmented_assignment() {
        let code = r#"
x = 1
x += 2
"#;
        let parsed = parse_module(code).unwrap();
        let module = parsed.into_syntax();
        let collected = VariableCollector::analyze(&module);

        let x_usages: Vec<_> = collected.usages.iter().filter(|u| u.name == "x").collect();
        assert_eq!(x_usages.len(), 3); // 1 initial write, 1 read + 1 write from +=
    }

    #[test]
    fn test_deletion() {
        let code = r#"
x = 1
del x
"#;
        let parsed = parse_module(code).unwrap();
        let module = parsed.into_syntax();
        let collected = VariableCollector::analyze(&module);

        let delete_usage = collected
            .usages
            .iter()
            .find(|u| matches!(u.usage_type, UsageType::Delete))
            .unwrap();
        assert_eq!(delete_usage.name, "x");
    }

    #[test]
    fn test_static_collect_function_globals() {
        let code = r#"
def foo():
    global x, y
    x = 1
"#;
        let parsed = parse_module(code).unwrap();
        let module = parsed.into_syntax();

        if let Stmt::FunctionDef(func) = &module.body[0] {
            let globals = VariableCollector::collect_function_globals(&func.body);
            assert_eq!(globals.len(), 2);
            assert!(globals.contains("x"));
            assert!(globals.contains("y"));
        }
    }
}

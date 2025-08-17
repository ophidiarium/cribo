//! Variable usage collection visitor
//!
//! This visitor traverses the AST to collect information about variable usage,
//! including reads, writes, deletions, and global/nonlocal declarations.

#[cfg(test)]
use ruff_python_ast::ModModule;
use ruff_python_ast::{
    Expr, Stmt,
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
    /// Current function name (if inside a function)
    current_function: Option<String>,
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
            current_function: None,
        }
    }

    /// Analyze a module and return collected variables (used in tests)
    #[cfg(test)]
    fn analyze(module: &ModModule) -> CollectedVariables {
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

    /// Check if a statement references a specific variable
    pub fn statement_references_variable(stmt: &Stmt, var_name: &str) -> bool {
        struct VarChecker<'a> {
            var_name: &'a str,
            found: bool,
        }

        impl<'a> Visitor<'a> for VarChecker<'_> {
            fn visit_expr(&mut self, expr: &'a Expr) {
                if let Expr::Name(name) = expr
                    && name.id.as_str() == self.var_name
                {
                    self.found = true;
                    return;
                }
                walk_expr(self, expr);
            }
        }

        let mut checker = VarChecker {
            var_name,
            found: false,
        };
        checker.visit_stmt(stmt);
        checker.found
    }

    /// Collect variables referenced in statements (static helper for compatibility)
    pub fn collect_referenced_vars(stmts: &[Stmt], vars: &mut FxIndexSet<String>) {
        struct SimpleStmtCollector<'a> {
            vars: &'a mut FxIndexSet<String>,
        }

        impl<'a> Visitor<'a> for SimpleStmtCollector<'_> {
            fn visit_stmt(&mut self, stmt: &'a Stmt) {
                walk_stmt(self, stmt);
            }

            fn visit_expr(&mut self, expr: &'a Expr) {
                if let Expr::Name(name) = expr {
                    self.vars.insert(name.id.to_string());
                }
                walk_expr(self, expr);
            }
        }

        let mut collector = SimpleStmtCollector { vars };
        for stmt in stmts {
            collector.visit_stmt(stmt);
        }
    }
}

impl<'a> Visitor<'a> for VariableCollector {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(func) => {
                // Enter function scope
                self.scope_stack.push(func.name.to_string());
                let prev_function = self.current_function.clone();
                self.current_function = Some(func.name.to_string());

                // Visit function body
                walk_stmt(self, stmt);

                // Exit function scope
                self.current_function = prev_function;
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
                // First visit the target in read context to record reads
                let prev_in_assignment = self.in_assignment_target;
                self.in_assignment_target = false;
                self.visit_expr(&aug_assign.target);
                self.in_assignment_target = prev_in_assignment;

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
                // Record global declarations
                for name in &global_stmt.names {
                    self.record_usage(name, UsageType::GlobalDeclaration, global_stmt.range);
                }

                // If we're in a function scope, also track in function_globals
                if let Some(ref func_name) = self.current_function {
                    let globals_set = self
                        .collected
                        .function_globals
                        .entry(func_name.clone())
                        .or_default();
                    for name in &global_stmt.names {
                        globals_set.insert(name.to_string());
                    }
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
        let code = r"
x = 1
y = x + 2
print(y)
";
        let parsed = parse_module(code).expect("Test code should parse successfully");
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
        let code = r"
def foo():
    global x, y
    x = 1
    y = 2
";
        let parsed = parse_module(code).expect("Test code should parse successfully");
        let module = parsed.into_syntax();
        let collected = VariableCollector::analyze(&module);

        // Check function globals
        let foo_globals = collected
            .function_globals
            .get("foo")
            .expect("Function 'foo' should exist in function_globals");
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
        let code = r"
x = 1
x += 2
";
        let parsed = parse_module(code).expect("Test code should parse successfully");
        let module = parsed.into_syntax();
        let collected = VariableCollector::analyze(&module);

        let x_usages: Vec<_> = collected.usages.iter().filter(|u| u.name == "x").collect();
        assert_eq!(x_usages.len(), 3); // 1 initial write, 1 read + 1 write from +=
    }

    #[test]
    fn test_augmented_assignment_complex_targets() {
        let code = r"
obj = {'attr': 5}
arr = [1, 2, 3]
i = 0
obj.attr += 1
arr[i] += 10
";
        let parsed = parse_module(code).expect("Test code should parse successfully");
        let module = parsed.into_syntax();
        let collected = VariableCollector::analyze(&module);

        // Check that 'obj' is recorded as read during obj.attr += 1
        let obj_usages: Vec<_> = collected
            .usages
            .iter()
            .filter(|u| u.name == "obj")
            .collect();
        // Should have: 1 write (assignment), 1 read (in augmented assignment)
        assert!(
            obj_usages
                .iter()
                .any(|u| matches!(u.usage_type, UsageType::Read)),
            "obj should be read in obj.attr += 1"
        );

        // Check that 'arr' is recorded as read during arr[i] += 10
        let arr_usages: Vec<_> = collected
            .usages
            .iter()
            .filter(|u| u.name == "arr")
            .collect();
        // Should have: 1 write (assignment), 1 read (in augmented assignment)
        assert!(
            arr_usages
                .iter()
                .any(|u| matches!(u.usage_type, UsageType::Read)),
            "arr should be read in arr[i] += 10"
        );

        // Check that 'i' is recorded as read during arr[i] += 10
        let i_usages: Vec<_> = collected.usages.iter().filter(|u| u.name == "i").collect();
        // Should have: 1 write (assignment), 1 read (in augmented assignment)
        assert!(
            i_usages
                .iter()
                .any(|u| matches!(u.usage_type, UsageType::Read)),
            "i should be read in arr[i] += 10"
        );
    }

    #[test]
    fn test_deletion() {
        let code = r"
x = 1
del x
";
        let parsed = parse_module(code).expect("Test code should parse successfully");
        let module = parsed.into_syntax();
        let collected = VariableCollector::analyze(&module);

        let delete_usage = collected
            .usages
            .iter()
            .find(|u| matches!(u.usage_type, UsageType::Delete))
            .expect("Delete usage should exist in collected usages");
        assert_eq!(delete_usage.name, "x");
    }

    #[test]
    fn test_static_collect_function_globals() {
        let code = r"
def foo():
    global x, y
    x = 1
";
        let parsed = parse_module(code).expect("Test code should parse successfully");
        let module = parsed.into_syntax();

        if let Stmt::FunctionDef(func) = &module.body[0] {
            let globals = VariableCollector::collect_function_globals(&func.body);
            assert_eq!(globals.len(), 2);
            assert!(globals.contains("x"));
            assert!(globals.contains("y"));
        }
    }

    #[test]
    fn test_nested_function_globals() {
        let code = r"
def outer():
    global x
    def inner():
        global y
        y = 2
    x = 1
";
        let parsed = parse_module(code).expect("Test code should parse successfully");
        let module = parsed.into_syntax();
        let collected = VariableCollector::analyze(&module);

        // Check outer function globals
        let outer_globals = collected
            .function_globals
            .get("outer")
            .expect("Function 'outer' should exist in function_globals");
        assert_eq!(outer_globals.len(), 1);
        assert!(outer_globals.contains("x"));

        // Check inner function globals
        let inner_globals = collected
            .function_globals
            .get("inner")
            .expect("Function 'inner' should exist in function_globals");
        assert_eq!(inner_globals.len(), 1);
        assert!(inner_globals.contains("y"));
    }
}

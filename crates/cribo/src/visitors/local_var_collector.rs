//! Local variable collector that respects global and nonlocal declarations
//!
//! This visitor traverses the AST in source order to collect local variable
//! declarations while excluding variables that have been declared as global,
//! and including variables declared as nonlocal.

use crate::types::FxIndexSet;
use ruff_python_ast::visitor::source_order::{self, SourceOrderVisitor};
use ruff_python_ast::{ExceptHandler, Expr, Stmt};

/// Visitor that collects local variable declarations in source order,
/// excluding variables that have been declared as global and including
/// variables declared as nonlocal
pub struct LocalVarCollector<'a> {
    /// Set to collect local variables
    local_vars: &'a mut FxIndexSet<String>,
    /// Set of global variables to exclude from local collection
    global_vars: &'a FxIndexSet<String>,
}

impl<'a> LocalVarCollector<'a> {
    /// Create a new local variable collector
    pub fn new(
        local_vars: &'a mut FxIndexSet<String>,
        global_vars: &'a FxIndexSet<String>,
    ) -> Self {
        Self {
            local_vars,
            global_vars,
        }
    }

    /// Collect local variables from a list of statements
    pub fn collect_from_stmts(&mut self, stmts: &'a [Stmt]) {
        source_order::walk_body(self, stmts);
    }

    /// Helper to check and insert a variable name if it's not global
    fn insert_if_not_global(&mut self, var_name: &str) {
        if !self.global_vars.contains(var_name) {
            self.local_vars.insert(var_name.to_string());
        }
    }

    /// Extract variable name from assignment target
    fn collect_from_target(&mut self, target: &Expr) {
        match target {
            Expr::Name(name) => {
                self.insert_if_not_global(&name.id);
            }
            Expr::Tuple(tuple) => {
                for elt in &tuple.elts {
                    self.collect_from_target(elt);
                }
            }
            Expr::List(list) => {
                for elt in &list.elts {
                    self.collect_from_target(elt);
                }
            }
            _ => {}
        }
    }
}

impl<'a> SourceOrderVisitor<'a> for LocalVarCollector<'a> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::Assign(assign) => {
                // Collect assignment targets as local variables
                for target in &assign.targets {
                    self.collect_from_target(target);
                }
            }
            Stmt::AnnAssign(ann_assign) => {
                // Collect annotated assignment targets
                self.collect_from_target(&ann_assign.target);
            }
            Stmt::For(for_stmt) => {
                // Collect for loop targets
                self.collect_from_target(&for_stmt.target);
                // Continue default traversal for body
                source_order::walk_stmt(self, stmt);
            }
            Stmt::With(with_stmt) => {
                // Collect with statement targets
                for item in &with_stmt.items {
                    if let Some(ref optional_vars) = item.optional_vars {
                        self.collect_from_target(optional_vars);
                    }
                }
                // Continue default traversal for body
                source_order::walk_stmt(self, stmt);
            }
            Stmt::FunctionDef(func_def) => {
                // Function definitions create local names (unless declared global)
                self.insert_if_not_global(&func_def.name);
                // Don't walk into the function body - we're only collecting local vars at the current scope
            }
            Stmt::ClassDef(class_def) => {
                // Class definitions create local names (unless declared global)
                self.insert_if_not_global(&class_def.name);
                // Don't walk into the class body - we're only collecting local vars at the current scope
            }
            Stmt::Nonlocal(nonlocal_stmt) => {
                // Nonlocal declarations create local names in the enclosing scope
                // This prevents incorrect module attribute rewrites in nested functions
                for name in &nonlocal_stmt.names {
                    self.insert_if_not_global(name);
                }
                // Continue default traversal
                source_order::walk_stmt(self, stmt);
            }
            _ => {
                // For all other statement types, use default traversal
                source_order::walk_stmt(self, stmt);
            }
        }
    }

    fn visit_except_handler(&mut self, handler: &'a ExceptHandler) {
        let ExceptHandler::ExceptHandler(eh) = handler;
        // Collect exception name if present
        if let Some(ref name) = eh.name {
            // Exception names are always local in their handler scope
            self.local_vars.insert(name.to_string());
        }
        // Continue default traversal for the handler body
        source_order::walk_except_handler(self, handler);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruff_python_parser::parse_module;

    fn parse_test_module(source: &str) -> ruff_python_ast::ModModule {
        let parsed = parse_module(source).expect("Failed to parse");
        parsed.into_syntax()
    }

    #[test]
    fn test_collect_basic_locals() {
        let source = r"
x = 1
y = 2
def foo():
    pass
class Bar:
    pass
";
        let module = parse_test_module(source);
        let mut local_vars = FxIndexSet::default();
        let global_vars = FxIndexSet::default();

        let mut collector = LocalVarCollector::new(&mut local_vars, &global_vars);
        collector.collect_from_stmts(&module.body);

        assert!(local_vars.contains("x"));
        assert!(local_vars.contains("y"));
        assert!(local_vars.contains("foo"));
        assert!(local_vars.contains("Bar"));
    }

    #[test]
    fn test_respect_globals() {
        let source = r"
global x
x = 1
y = 2
";
        let module = parse_test_module(source);
        let mut local_vars = FxIndexSet::default();
        let mut global_vars = FxIndexSet::default();
        global_vars.insert("x".to_string());

        let mut collector = LocalVarCollector::new(&mut local_vars, &global_vars);
        collector.collect_from_stmts(&module.body);

        assert!(!local_vars.contains("x")); // x is global
        assert!(local_vars.contains("y"));
    }

    #[test]
    fn test_for_loop_vars() {
        let source = r"
for i in range(10):
    j = i * 2
";
        let module = parse_test_module(source);
        let mut local_vars = FxIndexSet::default();
        let global_vars = FxIndexSet::default();

        let mut collector = LocalVarCollector::new(&mut local_vars, &global_vars);
        collector.collect_from_stmts(&module.body);

        assert!(local_vars.contains("i"));
        assert!(local_vars.contains("j"));
    }

    #[test]
    fn test_with_statement() {
        let source = r"
with open('file') as f:
    content = f.read()
";
        let module = parse_test_module(source);
        let mut local_vars = FxIndexSet::default();
        let global_vars = FxIndexSet::default();

        let mut collector = LocalVarCollector::new(&mut local_vars, &global_vars);
        collector.collect_from_stmts(&module.body);

        assert!(local_vars.contains("f"));
        assert!(local_vars.contains("content"));
    }

    #[test]
    fn test_exception_handling() {
        let source = r"
try:
    x = 1
except Exception as e:
    y = 2
";
        let module = parse_test_module(source);
        let mut local_vars = FxIndexSet::default();
        let global_vars = FxIndexSet::default();

        let mut collector = LocalVarCollector::new(&mut local_vars, &global_vars);
        collector.collect_from_stmts(&module.body);

        assert!(local_vars.contains("x"));
        assert!(local_vars.contains("e"));
        assert!(local_vars.contains("y"));
    }

    #[test]
    fn test_tuple_unpacking() {
        let source = r"
a, b = 1, 2
(c, d) = (3, 4)
[e, f] = [5, 6]
";
        let module = parse_test_module(source);
        let mut local_vars = FxIndexSet::default();
        let global_vars = FxIndexSet::default();

        let mut collector = LocalVarCollector::new(&mut local_vars, &global_vars);
        collector.collect_from_stmts(&module.body);

        assert!(local_vars.contains("a"));
        assert!(local_vars.contains("b"));
        assert!(local_vars.contains("c"));
        assert!(local_vars.contains("d"));
        assert!(local_vars.contains("e"));
        assert!(local_vars.contains("f"));
    }

    #[test]
    fn test_nonlocal_declarations() {
        let source = r"
def outer():
    x = 1
    def inner():
        nonlocal x
        x = 2
nonlocal y
y = 3
";
        let module = parse_test_module(source);
        let mut local_vars = FxIndexSet::default();
        let global_vars = FxIndexSet::default();

        let mut collector = LocalVarCollector::new(&mut local_vars, &global_vars);
        collector.collect_from_stmts(&module.body);

        // The function definition creates a local name
        assert!(local_vars.contains("outer"));
        // Nonlocal y at module level creates a local name
        assert!(local_vars.contains("y"));
        // x is not collected because it's inside the function definition
        assert!(!local_vars.contains("x"));
    }

    #[test]
    fn test_nonlocal_with_globals() {
        let source = r"
global x
nonlocal x
x = 1
nonlocal y
y = 2
";
        let module = parse_test_module(source);
        let mut local_vars = FxIndexSet::default();
        let mut global_vars = FxIndexSet::default();
        global_vars.insert("x".to_string());

        let mut collector = LocalVarCollector::new(&mut local_vars, &global_vars);
        collector.collect_from_stmts(&module.body);

        // x is global, so even though it's declared nonlocal, it shouldn't be collected
        assert!(!local_vars.contains("x"));
        // y is nonlocal and not global, so it should be collected
        assert!(local_vars.contains("y"));
    }
}

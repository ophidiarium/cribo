//! Local variable collector that respects `global` and `nonlocal` declarations
//!
//! This visitor traverses the AST in source order to collect local variable
//! names at module level, excluding names declared as `global`, and treating names
//! declared `nonlocal` as locals for collection (to prevent module-attr rewrites).

use ruff_python_ast::{
    AnyNodeRef, ExceptHandler, Expr, Stmt,
    visitor::source_order::{
        self, SourceOrderVisitor, TraversalSignal, walk_except_handler, walk_stmt,
    },
};

use crate::types::FxIndexSet;

/// Visitor that collects local variable names at module level,
/// excluding names declared as `global`, and treating `nonlocal` names as locals
pub(crate) struct LocalVarCollector<'a> {
    /// Set to collect local variable names
    local_vars: &'a mut FxIndexSet<String>,
    /// Set of global names to exclude from collection
    global_vars: &'a FxIndexSet<String>,
}

impl<'a> LocalVarCollector<'a> {
    /// Create a new local variable collector
    pub(crate) const fn new(
        local_vars: &'a mut FxIndexSet<String>,
        global_vars: &'a FxIndexSet<String>,
    ) -> Self {
        Self {
            local_vars,
            global_vars,
        }
    }

    /// Collect local variable names from a list of statements
    pub(crate) fn collect_from_stmts(&mut self, stmts: &'a [Stmt]) {
        source_order::walk_body(self, stmts);
    }

    /// Helper to check and insert a name if it's not global
    fn insert_if_not_global(&mut self, var_name: &str) {
        if !self.global_vars.contains(var_name) {
            self.local_vars.insert(var_name.to_owned());
        }
    }

    /// Extract variable names from assignment target
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
            Expr::Starred(starred) => {
                // Handle starred expressions like *rest in (a, *rest) = ...
                self.collect_from_target(&starred.value);
            }
            _ => {}
        }
    }
}

impl<'a> SourceOrderVisitor<'a> for LocalVarCollector<'a> {
    fn enter_node(&mut self, node: AnyNodeRef<'a>) -> TraversalSignal {
        // Skip traversing into functions and classes - we only collect module-level variables
        match node {
            AnyNodeRef::StmtFunctionDef(func_def) => {
                // Collect the function name itself as a module-level variable
                self.insert_if_not_global(&func_def.name);
                // Skip traversing into the function body
                TraversalSignal::Skip
            }
            AnyNodeRef::StmtClassDef(class_def) => {
                // Collect the class name itself as a module-level variable
                self.insert_if_not_global(&class_def.name);
                // Skip traversing into the class body
                TraversalSignal::Skip
            }
            _ => TraversalSignal::Traverse,
        }
    }

    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        // Process statements to collect variable bindings
        match stmt {
            Stmt::Assign(assign) => {
                for target in &assign.targets {
                    self.collect_from_target(target);
                }
            }
            Stmt::AnnAssign(ann_assign) => {
                self.collect_from_target(&ann_assign.target);
            }
            Stmt::AugAssign(aug_assign) => {
                self.collect_from_target(&aug_assign.target);
            }
            Stmt::For(for_stmt) => {
                self.collect_from_target(&for_stmt.target);
            }
            Stmt::With(with_stmt) => {
                for item in &with_stmt.items {
                    if let Some(ref optional_vars) = item.optional_vars {
                        self.collect_from_target(optional_vars);
                    }
                }
            }
            Stmt::Nonlocal(nonlocal_stmt) => {
                // Nonlocal declarations create local names in the enclosing scope
                // This prevents incorrect module attribute rewrites in nested functions
                for name in &nonlocal_stmt.names {
                    self.insert_if_not_global(name);
                }
            }
            Stmt::Import(import_stmt) => {
                for alias in &import_stmt.names {
                    let name = alias.asname.as_ref().map_or_else(
                        || {
                            // For dotted imports like 'import a.b.c', bind only 'a'
                            let full_name = alias.name.to_string();
                            full_name.split('.').next().unwrap_or(&full_name).to_owned()
                        },
                        ToString::to_string,
                    );
                    self.insert_if_not_global(&name);
                }
            }
            Stmt::ImportFrom(from_stmt) => {
                for alias in &from_stmt.names {
                    // Skip wildcard imports (from m import *)
                    if alias.name.as_str() == "*" {
                        continue;
                    }
                    let binding = alias
                        .asname
                        .as_ref()
                        .map_or_else(|| alias.name.to_string(), ToString::to_string);
                    self.insert_if_not_global(&binding);
                }
            }
            _ => {}
        }

        // Continue traversal to children
        walk_stmt(self, stmt);
    }

    fn visit_except_handler(&mut self, handler: &'a ExceptHandler) {
        let ExceptHandler::ExceptHandler(eh) = handler;
        // Collect exception name if present, respecting global declarations
        if let Some(ref name) = eh.name {
            self.insert_if_not_global(name);
        }
        // Continue traversal to children
        walk_except_handler(self, handler);
    }
}

#[cfg(test)]
mod tests {
    use ruff_python_parser::parse_module;

    use super::*;

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
        global_vars.insert("x".to_owned());

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
finally:
    z = 3
";
        let module = parse_test_module(source);
        let mut local_vars = FxIndexSet::default();
        let global_vars = FxIndexSet::default();

        let mut collector = LocalVarCollector::new(&mut local_vars, &global_vars);
        collector.collect_from_stmts(&module.body);

        assert!(local_vars.contains("x"));
        assert!(local_vars.contains("e"));
        assert!(local_vars.contains("y"));
        assert!(local_vars.contains("z"));
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
        global_vars.insert("x".to_owned());

        let mut collector = LocalVarCollector::new(&mut local_vars, &global_vars);
        collector.collect_from_stmts(&module.body);

        // x is global, so even though it's declared nonlocal, it shouldn't be collected
        assert!(!local_vars.contains("x"));
        // y is nonlocal and not global, so it should be collected
        assert!(local_vars.contains("y"));
    }

    #[test]
    fn test_augmented_assignment() {
        let source = r"
x = 0
x += 1
y -= 2
z *= 3
";
        let module = parse_test_module(source);
        let mut local_vars = FxIndexSet::default();
        let global_vars = FxIndexSet::default();

        let mut collector = LocalVarCollector::new(&mut local_vars, &global_vars);
        collector.collect_from_stmts(&module.body);

        assert!(local_vars.contains("x"));
        assert!(local_vars.contains("y"));
        assert!(local_vars.contains("z"));
    }

    #[test]
    fn test_async_constructs() {
        let source = r"
async def async_func():
    pass

async with open('file') as f:
    pass

async for i in range(10):
    pass
";
        let module = parse_test_module(source);
        let mut local_vars = FxIndexSet::default();
        let global_vars = FxIndexSet::default();

        let mut collector = LocalVarCollector::new(&mut local_vars, &global_vars);
        collector.collect_from_stmts(&module.body);

        assert!(local_vars.contains("async_func"));
        assert!(local_vars.contains("f"));
        assert!(local_vars.contains("i"));
    }

    #[test]
    fn test_import_statements() {
        let source = r"
import os
import sys as system
import a.b.c
from math import sin
from math import cos as cosine
";
        let module = parse_test_module(source);
        let mut local_vars = FxIndexSet::default();
        let global_vars = FxIndexSet::default();

        let mut collector = LocalVarCollector::new(&mut local_vars, &global_vars);
        collector.collect_from_stmts(&module.body);

        assert!(local_vars.contains("os"));
        assert!(local_vars.contains("system")); // alias for sys
        assert!(local_vars.contains("a")); // top-level package for a.b.c
        assert!(local_vars.contains("sin"));
        assert!(local_vars.contains("cosine")); // alias for cos
    }

    #[test]
    fn test_import_with_globals() {
        let source = r"
global os
import os
from math import sin
global sin
";
        let module = parse_test_module(source);
        let mut local_vars = FxIndexSet::default();
        let mut global_vars = FxIndexSet::default();
        global_vars.insert("os".to_owned());
        global_vars.insert("sin".to_owned());

        let mut collector = LocalVarCollector::new(&mut local_vars, &global_vars);
        collector.collect_from_stmts(&module.body);

        // Both os and sin are global, so they shouldn't be collected
        assert!(!local_vars.contains("os"));
        assert!(!local_vars.contains("sin"));
    }

    #[test]
    fn test_starred_targets() {
        let source = r"
a, *rest = [1, 2, 3]
[*xs, y] = [1, 2, 3]
for *items, last in [[1, 2, 3]]:
    pass
";
        let module = parse_test_module(source);
        let mut local_vars = FxIndexSet::default();
        let global_vars = FxIndexSet::default();

        let mut collector = LocalVarCollector::new(&mut local_vars, &global_vars);
        collector.collect_from_stmts(&module.body);

        assert!(local_vars.contains("a"));
        assert!(local_vars.contains("rest"));
        assert!(local_vars.contains("xs"));
        assert!(local_vars.contains("y"));
        assert!(local_vars.contains("items"));
        assert!(local_vars.contains("last"));
    }

    #[test]
    fn test_augmented_assignment_with_globals() {
        let source = r"
global x
x += 1
y += 2
";
        let module = parse_test_module(source);
        let mut local_vars = FxIndexSet::default();
        let mut global_vars = FxIndexSet::default();
        global_vars.insert("x".to_owned());

        let mut collector = LocalVarCollector::new(&mut local_vars, &global_vars);
        collector.collect_from_stmts(&module.body);

        // x is global, so it shouldn't be collected even with augmented assignment
        assert!(!local_vars.contains("x"));
        // y is not global, so it should be collected
        assert!(local_vars.contains("y"));
    }

    #[test]
    fn test_wildcard_imports() {
        let source = r"
from math import *
from os import path
from sys import argv
";
        let module = parse_test_module(source);
        let mut local_vars = FxIndexSet::default();
        let global_vars = FxIndexSet::default();

        let mut collector = LocalVarCollector::new(&mut local_vars, &global_vars);
        collector.collect_from_stmts(&module.body);

        // Wildcard imports don't bind any names
        assert!(!local_vars.contains("*"));
        // But specific imports do
        assert!(local_vars.contains("path"));
        assert!(local_vars.contains("argv"));
    }

    #[test]
    fn test_exception_with_global() {
        let source = r"
global e
try:
    x = 1
except Exception as e:
    y = 2
except ValueError as v:
    z = 3
";
        let module = parse_test_module(source);
        let mut local_vars = FxIndexSet::default();
        let mut global_vars = FxIndexSet::default();
        global_vars.insert("e".to_owned());

        let mut collector = LocalVarCollector::new(&mut local_vars, &global_vars);
        collector.collect_from_stmts(&module.body);

        // e is global, so it shouldn't be collected
        assert!(!local_vars.contains("e"));
        // v is not global, so it should be collected
        assert!(local_vars.contains("v"));
        // Regular assignments are collected
        assert!(local_vars.contains("x"));
        assert!(local_vars.contains("y"));
        assert!(local_vars.contains("z"));
    }
}

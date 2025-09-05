//! Local variable collector that respects `global` and `nonlocal` declarations
//!
//! This visitor traverses the AST in source order to collect local variable
//! names, excluding names declared as `global`, and treating names
//! declared `nonlocal` as locals for collection (to prevent module-attr rewrites).

use ruff_python_ast::{
    ExceptHandler, Expr, Stmt,
    visitor::source_order::{self, SourceOrderVisitor},
};

use crate::types::FxIndexSet;

/// Visitor that collects local variable names in source order,
/// excluding names declared as `global`, and treating `nonlocal` names as locals
/// for the purposes of collection
pub struct LocalVarCollector<'a> {
    /// Set to collect local variable names
    local_vars: &'a mut FxIndexSet<String>,
    /// Set of global names to exclude from collection
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

    /// Collect local variable names from a list of statements
    pub fn collect_from_stmts(&mut self, stmts: &'a [Stmt]) {
        source_order::walk_body(self, stmts);
    }

    /// Helper to check and insert a name if it's not global
    fn insert_if_not_global(&mut self, var_name: &str) {
        if !self.global_vars.contains(var_name) {
            self.local_vars.insert(var_name.to_string());
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
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::Assign(assign) => {
                // Collect names from assignment targets
                for target in &assign.targets {
                    self.collect_from_target(target);
                }
            }
            Stmt::AnnAssign(ann_assign) => {
                // Collect names from annotated assignment targets
                self.collect_from_target(&ann_assign.target);
            }
            Stmt::AugAssign(aug_assign) => {
                // Augmented assignments (x += 1) bind names in current scope
                self.collect_from_target(&aug_assign.target);
                // Continue default traversal
                source_order::walk_stmt(self, stmt);
            }
            Stmt::For(for_stmt) => {
                // Collect for loop targets (including async for)
                // Note: is_async is a flag on For, not a separate statement type
                self.collect_from_target(&for_stmt.target);
                // Continue default traversal for body
                source_order::walk_stmt(self, stmt);
            }
            Stmt::With(with_stmt) => {
                // Collect with statement targets (including async with)
                // Note: is_async is a flag on With, not a separate statement type
                for item in &with_stmt.items {
                    if let Some(ref optional_vars) = item.optional_vars {
                        self.collect_from_target(optional_vars);
                    }
                }
                // Continue default traversal for body
                source_order::walk_stmt(self, stmt);
            }
            Stmt::FunctionDef(func_def) => {
                // Function definitions (including async) create local names (unless declared
                // global) Note: is_async is a flag on FunctionDef, not a separate
                // statement type
                self.insert_if_not_global(&func_def.name);
                // Don't walk into the function body - we're only collecting local vars at the
                // current scope
            }
            Stmt::ClassDef(class_def) => {
                // Class definitions create local names (unless declared global)
                self.insert_if_not_global(&class_def.name);
                // Don't walk into the class body - we're only collecting local vars at the current
                // scope
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
            Stmt::Import(import_stmt) => {
                // Import statements bind names in the current scope
                for alias in &import_stmt.names {
                    let name = if let Some(asname) = &alias.asname {
                        asname.to_string()
                    } else {
                        // For dotted imports like 'import a.b.c', bind only the top-level package
                        // 'a'
                        let full_name = alias.name.to_string();
                        full_name
                            .split('.')
                            .next()
                            .unwrap_or(&full_name)
                            .to_string()
                    };
                    self.insert_if_not_global(&name);
                }
            }
            Stmt::ImportFrom(from_stmt) => {
                // From imports bind the imported names or their aliases
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
            _ => {
                // For all other statement types, use default traversal
                source_order::walk_stmt(self, stmt);
            }
        }
    }

    fn visit_except_handler(&mut self, handler: &'a ExceptHandler) {
        let ExceptHandler::ExceptHandler(eh) = handler;
        // Collect exception name if present, respecting global declarations
        if let Some(ref name) = eh.name {
            // Exception names can be global if previously declared
            self.insert_if_not_global(name);
        }
        // Continue default traversal for the handler body
        source_order::walk_except_handler(self, handler);
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
        global_vars.insert("x".to_string());

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
        global_vars.insert("os".to_string());
        global_vars.insert("sin".to_string());

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
        global_vars.insert("x".to_string());

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
        global_vars.insert("e".to_string());

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

//! Visitor for detecting side effects in Python AST
//!
//! This module implements a visitor pattern for traversing Python AST nodes
//! and detecting whether they contain side effects.

use ruff_python_ast::visitor::{Visitor, walk_expr, walk_stmt};
use ruff_python_ast::{Expr, ModModule, Stmt, StmtAssign};
use rustc_hash::FxHashSet;

/// Visitor for detecting side effects in Python code
pub struct SideEffectDetector {
    /// Names that were imported and may have side effects when used
    imported_names: FxHashSet<String>,
    /// Flag indicating if side effects were found
    has_side_effects: bool,
    /// Whether we're currently analyzing an expression for side effects
    in_expression_context: bool,
}

/// Simple expression visitor for checking side effects in a single expression
pub struct ExpressionSideEffectDetector {
    has_side_effects: bool,
}

impl SideEffectDetector {
    /// Create a new side effect detector
    pub fn new() -> Self {
        Self {
            imported_names: FxHashSet::default(),
            has_side_effects: false,
            in_expression_context: false,
        }
    }

    /// Check if a module has side effects
    pub fn module_has_side_effects(mut self, module: &ModModule) -> bool {
        // First pass: collect imported names
        self.collect_imported_names(module);

        // Second pass: check for side effects
        self.visit_body(&module.body);

        self.has_side_effects
    }

    /// Collect all imported names from the module
    fn collect_imported_names(&mut self, module: &ModModule) {
        for stmt in &module.body {
            match stmt {
                Stmt::Import(import_stmt) => {
                    for alias in &import_stmt.names {
                        let name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
                        self.imported_names.insert(name.to_string());
                    }
                }
                Stmt::ImportFrom(import_from) => {
                    for alias in &import_from.names {
                        let name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
                        self.imported_names.insert(name.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    /// Check if an assignment is to __all__
    fn is_all_assignment(&self, assign: &StmtAssign) -> bool {
        if assign.targets.len() != 1 {
            return false;
        }
        matches!(&assign.targets[0], Expr::Name(name) if name.id.as_str() == "__all__")
    }
}

impl Default for SideEffectDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Visitor<'a> for SideEffectDetector {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        // Skip if we already found side effects
        if self.has_side_effects {
            return;
        }

        match stmt {
            // These statements are pure definitions, no side effects
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) | Stmt::AnnAssign(_) => {
                // Don't recurse into function/class bodies
                // Their execution is deferred
                return; // Important: don't call walk_stmt
            }

            // Assignments need checking
            Stmt::Assign(assign) => {
                // Special case: __all__ assignments are metadata, not side effects
                if !self.is_all_assignment(assign) {
                    // Check if the assignment value has side effects
                    self.in_expression_context = true;
                    self.visit_expr(&assign.value);
                    self.in_expression_context = false;
                }
            }

            // Import statements are handled separately by the bundler
            Stmt::Import(_) | Stmt::ImportFrom(_) => {
                return; // Don't call walk_stmt
            }

            // Type alias statements are safe
            Stmt::TypeAlias(_) => {
                return; // Don't call walk_stmt
            }

            // Pass statements are no-ops
            Stmt::Pass(_) => {
                return; // Don't call walk_stmt
            }

            // Expression statements
            Stmt::Expr(expr_stmt) => {
                // Docstrings are safe
                if !matches!(expr_stmt.value.as_ref(), Expr::StringLiteral(_)) {
                    // Other expression statements have side effects
                    self.has_side_effects = true;
                    return;
                }
            }

            // These are definitely side effects
            Stmt::If(_)
            | Stmt::While(_)
            | Stmt::For(_)
            | Stmt::With(_)
            | Stmt::Match(_)
            | Stmt::Raise(_)
            | Stmt::Try(_)
            | Stmt::Assert(_)
            | Stmt::Global(_)
            | Stmt::Nonlocal(_)
            | Stmt::Delete(_) => {
                self.has_side_effects = true;
                return;
            }

            // Any other statement type is considered a side effect
            _ => {
                self.has_side_effects = true;
                return;
            }
        }

        // Continue walking for statements we want to analyze deeper
        walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        // Skip if we already found side effects
        if self.has_side_effects {
            return;
        }

        // Only check for side effects if we're in expression context
        if self.in_expression_context {
            match expr {
                // Names might be imported and have side effects
                Expr::Name(name) => {
                    if self.imported_names.contains(name.id.as_str()) {
                        self.has_side_effects = true;
                        return;
                    }
                }

                // These expressions have side effects
                Expr::Call(_) | Expr::Attribute(_) | Expr::Subscript(_) => {
                    self.has_side_effects = true;
                    return;
                }

                // For other expressions, continue walking to check nested expressions
                _ => {}
            }
        }

        // Continue walking
        walk_expr(self, expr);
    }
}

impl ExpressionSideEffectDetector {
    /// Create a new expression side effect detector
    pub fn new() -> Self {
        Self {
            has_side_effects: false,
        }
    }

    /// Check if an expression has side effects
    pub fn expression_has_side_effects(mut self, expr: &Expr) -> bool {
        self.visit_expr(expr);
        self.has_side_effects
    }
}

impl Default for ExpressionSideEffectDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Visitor<'a> for ExpressionSideEffectDetector {
    fn visit_expr(&mut self, expr: &'a Expr) {
        // Skip if we already found side effects
        if self.has_side_effects {
            return;
        }

        match expr {
            // These expressions have side effects
            Expr::Call(_) | Expr::Attribute(_) | Expr::Subscript(_) => {
                self.has_side_effects = true;
                return;
            }

            // For other expressions, continue walking to check nested expressions
            _ => {}
        }

        // Continue walking
        walk_expr(self, expr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruff_python_parser::{ParseError, parse_module};

    fn parse_python(source: &str) -> Result<ModModule, ParseError> {
        parse_module(source).map(|parsed| parsed.into_syntax())
    }

    #[test]
    fn test_no_side_effects_simple() {
        let source = r#"
def foo():
    pass

class Bar:
    pass

x = 42
y = "hello"
z = [1, 2, 3]
"#;
        let module = parse_python(source).expect("Failed to parse test Python code");
        let detector = SideEffectDetector::new();
        assert!(!detector.module_has_side_effects(&module));
    }

    #[test]
    fn test_side_effects_function_call() {
        let source = r#"
def foo():
    pass

foo()  # This is a side effect
"#;
        let module = parse_python(source).expect("Failed to parse test Python code");
        let detector = SideEffectDetector::new();
        assert!(detector.module_has_side_effects(&module));
    }

    #[test]
    fn test_side_effects_imported_name() {
        let source = r#"
import os
x = os  # Using imported name is a potential side effect
"#;
        let module = parse_python(source).expect("Failed to parse test Python code");
        let detector = SideEffectDetector::new();
        assert!(detector.module_has_side_effects(&module));
    }

    #[test]
    fn test_no_side_effects_all_assignment() {
        let source = r#"
__all__ = ["foo", "bar"]
"#;
        let module = parse_python(source).expect("Failed to parse test Python code");
        let detector = SideEffectDetector::new();
        assert!(!detector.module_has_side_effects(&module));
    }

    #[test]
    fn test_no_side_effects_docstring() {
        let source = r#"
"""This is a module docstring."""

def foo():
    """This is a function docstring."""
    pass
"#;
        let module = parse_python(source).expect("Failed to parse test Python code");
        let detector = SideEffectDetector::new();
        assert!(!detector.module_has_side_effects(&module));
    }

    #[test]
    fn test_side_effects_control_flow() {
        let source = r#"
if True:
    x = 1
"#;
        let module = parse_python(source).expect("Failed to parse test Python code");
        let detector = SideEffectDetector::new();
        assert!(detector.module_has_side_effects(&module));
    }

    #[test]
    fn test_side_effects_in_assignment() {
        let source = r#"
def get_value():
    return 42

x = get_value()  # Function call in assignment is a side effect
"#;
        let module = parse_python(source).expect("Failed to parse test Python code");
        let detector = SideEffectDetector::new();
        assert!(detector.module_has_side_effects(&module));
    }

    #[test]
    fn test_no_side_effects_complex_literals() {
        let source = r#"
x = {
    "a": 1,
    "b": [1, 2, 3],
    "c": {"nested": True}
}
y = [(i, i * 2) for i in [1, 2, 3]]
"#;
        let module = parse_python(source).expect("Failed to parse test Python code");
        let detector = SideEffectDetector::new();
        assert!(!detector.module_has_side_effects(&module));
    }
}

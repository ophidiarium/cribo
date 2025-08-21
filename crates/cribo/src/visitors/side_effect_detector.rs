//! Visitor for detecting side effects in Python AST
//!
//! This module implements a visitor pattern for traversing Python AST nodes
//! and detecting whether they contain side effects.

use ruff_python_ast::{
    Expr, ModModule, Stmt, StmtAssign,
    visitor::{Visitor, walk_expr, walk_stmt},
};
use rustc_hash::FxHashSet;

/// Visitor for detecting side effects in Python code
pub struct SideEffectDetector {
    /// Names that were imported and may have side effects when used
    imported_names: FxHashSet<String>,
    /// Flag indicating if side effects were found
    has_side_effects: bool,
    /// Whether we're currently analyzing an expression for side effects
    in_expression_context: bool,
    /// Whether we're currently analyzing a type annotation
    in_annotation_context: bool,
    /// Python version for stdlib detection
    python_version: u8,
}

/// Simple expression visitor for checking side effects in a single expression
pub struct ExpressionSideEffectDetector {
    has_side_effects: bool,
}

impl SideEffectDetector {
    /// Create a new side effect detector
    pub fn new(python_version: u8) -> Self {
        Self {
            imported_names: FxHashSet::default(),
            has_side_effects: false,
            in_expression_context: false,
            in_annotation_context: false,
            python_version,
        }
    }

    /// Check if a module has side effects (static method to avoid allocation in caller)
    pub fn check_module(module: &ModModule, python_version: u8) -> bool {
        let mut detector = Self::new(python_version);
        detector.module_has_side_effects(module)
    }

    /// Check if a module has side effects (instance method)
    fn module_has_side_effects(&mut self, module: &ModModule) -> bool {
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
                    self.collect_import_names(import_stmt);
                }
                Stmt::ImportFrom(import_from) => {
                    self.collect_import_from_names(import_from);
                }
                _ => {}
            }
        }
    }

    /// Helper to collect names from import statements
    fn collect_import_names(&mut self, import_stmt: &ruff_python_ast::StmtImport) {
        for alias in &import_stmt.names {
            let module_name = alias.name.as_str();
            let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

            // Skip imports that don't have side effects
            if !crate::side_effects::import_has_side_effects(module_name, self.python_version) {
                continue;
            }

            // For imports like "import xml.etree.ElementTree",
            // we need to track both the full path and the root binding
            self.imported_names.insert(local_name.to_string());

            // Also track the root module name (e.g., "xml" from "xml.etree.ElementTree")
            if let Some(root) = local_name.split('.').next() {
                self.imported_names.insert(root.to_string());
            }
        }
    }

    /// Helper to collect names from import-from statements
    fn collect_import_from_names(&mut self, import_from: &ruff_python_ast::StmtImportFrom) {
        // Skip imports that don't have side effects
        if !crate::side_effects::from_import_has_side_effects(import_from, self.python_version) {
            return;
        }

        for alias in &import_from.names {
            let name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

            // Track the imported name
            self.imported_names.insert(name.to_string());

            // For "from x import y", the binding is just "y", but
            // if it's a dotted name, also track the root
            if name.contains('.')
                && let Some(root) = name.split('.').next()
            {
                self.imported_names.insert(root.to_string());
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

    /// Check if a statement is an augmented assignment to __all__
    fn is_all_augmented_assignment(&self, stmt: &Stmt) -> bool {
        if let Stmt::AugAssign(aug_assign) = stmt {
            matches!(&*aug_assign.target, Expr::Name(name) if name.id.as_str() == "__all__")
        } else {
            false
        }
    }

    /// Check if an expression is a method call on __all__
    fn is_all_method_call(&self, expr: &Expr) -> bool {
        if let Expr::Call(call) = expr
            && let Expr::Attribute(attr) = &*call.func
            && let Expr::Name(name) = &*attr.value
        {
            // Check for __all__.extend(), __all__.append(), etc.
            return name.id.as_str() == "__all__";
        }
        false
    }
}

impl<'a> Visitor<'a> for SideEffectDetector {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        // Skip if we already found side effects
        if self.has_side_effects {
            return;
        }

        match stmt {
            // Function definitions need to check decorators and defaults for side effects
            Stmt::FunctionDef(func) => {
                // Check decorators for side effects (executed at import time)
                self.in_expression_context = true;
                for decorator in &func.decorator_list {
                    self.visit_expr(&decorator.expression);
                    if self.has_side_effects {
                        self.in_expression_context = false;
                        return;
                    }
                }

                // Check return annotation for side effects
                if let Some(returns) = &func.returns {
                    self.in_annotation_context = true;
                    self.visit_expr(returns);
                    self.in_annotation_context = false;
                    if self.has_side_effects {
                        self.in_expression_context = false;
                        return;
                    }
                }

                // Check default argument values for side effects
                for param in &func.parameters.args {
                    if let Some(default) = &param.default {
                        self.visit_expr(default);
                        if self.has_side_effects {
                            self.in_expression_context = false;
                            return;
                        }
                    }
                }

                for param in &func.parameters.posonlyargs {
                    if let Some(default) = &param.default {
                        self.visit_expr(default);
                        if self.has_side_effects {
                            self.in_expression_context = false;
                            return;
                        }
                    }
                }

                for param in &func.parameters.kwonlyargs {
                    if let Some(default) = &param.default {
                        self.visit_expr(default);
                        if self.has_side_effects {
                            self.in_expression_context = false;
                            return;
                        }
                    }
                }

                // Check parameter annotations for side effects
                for param in &func.parameters.args {
                    if let Some(annotation) = &param.parameter.annotation {
                        self.in_annotation_context = true;
                        self.visit_expr(annotation);
                        self.in_annotation_context = false;
                        if self.has_side_effects {
                            self.in_expression_context = false;
                            return;
                        }
                    }
                }

                for param in &func.parameters.posonlyargs {
                    if let Some(annotation) = &param.parameter.annotation {
                        self.in_annotation_context = true;
                        self.visit_expr(annotation);
                        self.in_annotation_context = false;
                        if self.has_side_effects {
                            self.in_expression_context = false;
                            return;
                        }
                    }
                }

                for param in &func.parameters.kwonlyargs {
                    if let Some(annotation) = &param.parameter.annotation {
                        self.in_annotation_context = true;
                        self.visit_expr(annotation);
                        self.in_annotation_context = false;
                        if self.has_side_effects {
                            self.in_expression_context = false;
                            return;
                        }
                    }
                }

                self.in_expression_context = false;

                // Skip the function body - its execution is deferred
                return; // Important: don't call walk_stmt
            }

            // Class definitions need special handling
            Stmt::ClassDef(class_def) => {
                // Check if the class has a metaclass - metaclasses can execute arbitrary code
                // during class creation, making them a side effect
                if let Some(ref arguments) = class_def.arguments
                    && arguments
                        .keywords
                        .iter()
                        .any(|kw| kw.arg.as_deref() == Some("metaclass"))
                {
                    // Class with metaclass is a side effect
                    self.has_side_effects = true;
                    return;
                }

                // Check class body for module-level side effects
                // (but not method bodies - those are only executed when called)
                for stmt in &class_def.body {
                    match stmt {
                        // Method definitions are not side effects
                        Stmt::FunctionDef(_) => {}

                        // Assignments in class body could be side effects if they call functions
                        Stmt::Assign(assign) => {
                            self.in_expression_context = true;
                            self.visit_expr(&assign.value);
                            self.in_expression_context = false;
                            if self.has_side_effects {
                                return;
                            }
                        }

                        // Other statements in class body
                        _ => {
                            self.visit_stmt(stmt);
                            if self.has_side_effects {
                                return;
                            }
                        }
                    }
                }
                return; // Don't call walk_stmt
            }

            // Annotated assignments need checking if they have a value
            Stmt::AnnAssign(ann_assign) => {
                if let Some(value) = &ann_assign.value {
                    // Check if the assignment value has side effects
                    self.in_expression_context = true;
                    self.visit_expr(value);
                    self.in_expression_context = false;
                }
                return; // Don't call walk_stmt
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
            // Type alias statements are safe
            // Pass statements are no-ops
            Stmt::Import(_) | Stmt::ImportFrom(_) | Stmt::TypeAlias(_) | Stmt::Pass(_) => {
                return; // Don't call walk_stmt
            }

            // Expression statements
            Stmt::Expr(expr_stmt) => {
                // Docstrings and constant expressions are safe
                if matches!(
                    expr_stmt.value.as_ref(),
                    Expr::StringLiteral(_)
                        | Expr::NumberLiteral(_)
                        | Expr::BooleanLiteral(_)
                        | Expr::NoneLiteral(_)
                        | Expr::BytesLiteral(_)
                        | Expr::EllipsisLiteral(_)
                ) {
                    return; // Safe, no side effects
                }

                // Method calls on __all__ are also safe (e.g., __all__.extend([...]))
                if self.is_all_method_call(&expr_stmt.value) {
                    return; // Safe metadata operation
                }

                // Other expression statements have side effects
                self.has_side_effects = true;
                return;
            }

            // Augmented assignments need special handling
            Stmt::AugAssign(_) => {
                // Check if this is an augmented assignment to __all__
                if !self.is_all_augmented_assignment(stmt) {
                    // Regular augmented assignments have side effects
                    self.has_side_effects = true;
                }
                return; // Don't walk further
            }

            // All remaining statement types are conservatively considered to have side effects.
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
                // Lambda expressions are considered to have side effects to match old behavior
                Expr::Call(_) | Expr::Lambda(_) => {
                    self.has_side_effects = true;
                    return;
                }

                // Subscript expressions can have side effects (e.g., __getitem__ can have side
                // effects)
                Expr::Subscript(_) => {
                    if !self.in_annotation_context {
                        self.has_side_effects = true;
                        return;
                    }
                }

                // Attribute access is only a side effect if the base object might have side effects
                // For now, we'll walk into it to check if the base has side effects
                // For other expressions, continue walking to check nested expressions
                _ => {
                    // Continue walking to check the base object/nested expressions
                }
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

    /// Check if an expression has side effects (static method to avoid allocation in caller)
    pub fn check(expr: &Expr) -> bool {
        let mut detector = Self::new();
        detector.expression_has_side_effects(expr)
    }

    /// Check if an expression has side effects (instance method)
    fn expression_has_side_effects(&mut self, expr: &Expr) -> bool {
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
            Expr::Call(_)
            | Expr::Subscript(_)
            | Expr::Await(_)
            | Expr::Yield(_)
            | Expr::YieldFrom(_)
            | Expr::Lambda(_) => {
                self.has_side_effects = true;
                return;
            }

            // Attribute access only has side effects if:
            // 1. It's on something other than a simple name (e.g., func().attr)
            // 2. It's deeply nested (e.g., a.b.c.d might trigger multiple __getattr__)
            Expr::Attribute(attr) => {
                // Check if this is a simple attribute access on a name
                if let Expr::Name(_) = &*attr.value {
                    // Simple attribute access like messages.message or abc.ABC
                    // These are common patterns that rarely have side effects
                    // Continue walking to check the base name
                } else {
                    // Complex attribute access on non-name expressions
                    // e.g., func().attr, (a + b).attr, etc.
                    self.has_side_effects = true;
                    return;
                }
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
    use ruff_python_parser::{ParseError, parse_module};

    use super::*;

    fn parse_python(source: &str) -> Result<ModModule, ParseError> {
        parse_module(source).map(ruff_python_parser::Parsed::into_syntax)
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
        assert!(!SideEffectDetector::check_module(&module, 10));
    }

    #[test]
    fn test_side_effects_function_call() {
        let source = r"
def foo():
    pass

foo()  # This is a side effect
";
        let module = parse_python(source).expect("Failed to parse test Python code");
        assert!(SideEffectDetector::check_module(&module, 10));
    }

    #[test]
    fn test_side_effects_imported_name() {
        let source = r"
import django.setup
x = django  # Using imported name that has side effects
";
        let module = parse_python(source).expect("Failed to parse test Python code");
        assert!(SideEffectDetector::check_module(&module, 10));
    }

    #[test]
    fn test_no_side_effects_all_assignment() {
        let source = r#"
__all__ = ["foo", "bar"]
"#;
        let module = parse_python(source).expect("Failed to parse test Python code");
        assert!(!SideEffectDetector::check_module(&module, 10));
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
        assert!(!SideEffectDetector::check_module(&module, 10));
    }

    #[test]
    fn test_side_effects_control_flow() {
        let source = r"
if True:
    x = 1
";
        let module = parse_python(source).expect("Failed to parse test Python code");
        assert!(SideEffectDetector::check_module(&module, 10));
    }

    #[test]
    fn test_side_effects_in_assignment() {
        let source = r"
def get_value():
    return 42

x = get_value()  # Function call in assignment is a side effect
";
        let module = parse_python(source).expect("Failed to parse test Python code");
        assert!(SideEffectDetector::check_module(&module, 10));
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
        assert!(!SideEffectDetector::check_module(&module, 10));
    }

    #[test]
    fn test_side_effects_annotated_assignment() {
        let source = r"
def get_value():
    return 42

x: int = get_value()  # Function call in annotation assignment is a side effect
";
        let module = parse_python(source).expect("Failed to parse test Python code");
        assert!(SideEffectDetector::check_module(&module, 10));
    }

    #[test]
    fn test_no_side_effects_annotated_assignment_without_value() {
        let source = r"
x: int  # Just annotation, no value, no side effect
";
        let module = parse_python(source).expect("Failed to parse test Python code");
        assert!(!SideEffectDetector::check_module(&module, 10));
    }

    #[test]
    fn test_no_side_effects_all_augmented_assignment() {
        let source = r#"
__all__ = ["foo"]
__all__ += ["bar"]  # Augmented assignment to __all__ is safe
__all__ |= {"baz"}  # Set union is also safe
"#;
        let module = parse_python(source).expect("Failed to parse test Python code");
        assert!(!SideEffectDetector::check_module(&module, 10));
    }

    #[test]
    fn test_no_side_effects_all_method_calls() {
        let source = r#"
__all__ = []
__all__.extend(["foo", "bar"])  # Method call on __all__ is safe
__all__.append("baz")  # Also safe
"#;
        let module = parse_python(source).expect("Failed to parse test Python code");
        assert!(!SideEffectDetector::check_module(&module, 10));
    }

    #[test]
    fn test_side_effects_regular_augmented_assignment() {
        let source = r"
x = 0
x += 1  # Regular augmented assignment is a side effect
";
        let module = parse_python(source).expect("Failed to parse test Python code");
        assert!(SideEffectDetector::check_module(&module, 10));
    }

    #[test]
    fn test_no_side_effects_constant_expressions() {
        let source = r#"
42  # Bare number
"hello"  # Bare string
True  # Bare boolean
None  # Bare None
b"bytes"  # Bare bytes
...  # Bare ellipsis
"#;
        let module = parse_python(source).expect("Failed to parse test Python code");
        assert!(!SideEffectDetector::check_module(&module, 10));
    }

    #[test]
    fn test_imported_name_root_binding() {
        let source = r"
import requests.adapters
x = requests  # Using the root binding of a third-party module (always a side effect)
";
        let module = parse_python(source).expect("Failed to parse test Python code");
        assert!(SideEffectDetector::check_module(&module, 10));
    }

    #[test]
    fn test_no_side_effects_safe_stdlib_import() {
        let source = r"
import os
import json
import typing
x = os  # Safe stdlib module usage is not a side effect
y = json
z = typing
";
        let module = parse_python(source).expect("Failed to parse test Python code");
        assert!(!SideEffectDetector::check_module(&module, 10));
    }

    #[test]
    fn test_lambda_assignment_has_side_effects() {
        let source = r#"
# Lambda assignments are considered to have side effects
validate = lambda x: f"validate: {x}"
process = lambda data: data.upper()
"#;
        let module = parse_python(source).expect("Failed to parse test Python code");
        assert!(SideEffectDetector::check_module(&module, 10));
    }

    #[test]
    fn test_decorator_side_effects() {
        let source = r#"
def side_effect_decorator():
    print("Side effect!")  # This would execute at import time
    return lambda f: f

@side_effect_decorator()  # Function call in decorator - side effect!
def my_function():
    pass
"#;
        let module = parse_python(source).expect("Failed to parse test Python code");
        assert!(SideEffectDetector::check_module(&module, 10));
    }

    #[test]
    fn test_default_argument_side_effects() {
        let source = r#"
def get_default():
    print("Getting default value")  # Side effect!
    return 42

def my_function(x=get_default()):  # This executes at import time
    pass
"#;
        let module = parse_python(source).expect("Failed to parse test Python code");
        assert!(SideEffectDetector::check_module(&module, 10));
    }

    #[test]
    fn test_annotation_side_effects() {
        let source = r#"
def get_type():
    print("Getting type")  # Side effect!
    return int

def my_function(x: get_type()) -> get_type():  # Annotations execute at import time
    pass
"#;
        let module = parse_python(source).expect("Failed to parse test Python code");
        assert!(SideEffectDetector::check_module(&module, 10));
    }

    #[test]
    fn test_no_side_effects_simple_decorator() {
        let source = r"
@property  # Built-in decorator, no side effect
def my_property(self):
    return self._value

@staticmethod  # Built-in decorator, no side effect
def my_static_method():
    pass
";
        let module = parse_python(source).expect("Failed to parse test Python code");
        assert!(!SideEffectDetector::check_module(&module, 10));
    }

    #[test]
    fn test_no_side_effects_simple_defaults() {
        let source = r#"
def my_function(x=42, y="hello", z=None, w=[]):  # Literal defaults, no side effects
    pass
"#;
        let module = parse_python(source).expect("Failed to parse test Python code");
        assert!(!SideEffectDetector::check_module(&module, 10));
    }
}

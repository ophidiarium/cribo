//! Statement categorizer for analyzing and grouping Python statements
//!
//! This module categorizes statements based on their type and dependencies,
//! identifying which statements need to be declared before others. It handles
//! various Python constructs including class inheritance, decorators, metaclasses,
//! and module namespaces.

use indexmap::IndexSet as FxIndexSet;
use ruff_python_ast::{Expr, Stmt};

/// Result of analyzing statements for reordering
#[derive(Debug, Default, Clone)]
pub struct StatementCategories {
    /// Import statements (import and from...import)
    pub imports: Vec<Stmt>,
    /// Assignments that define symbols used as base classes, metaclasses, or decorators
    pub dependency_assignments: Vec<Stmt>,
    /// Regular assignments (variables, non-class attributes)
    pub regular_assignments: Vec<Stmt>,
    /// Self-assignments (e.g., validate = validate)
    pub self_assignments: Vec<Stmt>,
    /// Class definitions
    pub classes: Vec<Stmt>,
    /// Function definitions
    pub functions: Vec<Stmt>,
    /// Other statements (class attribute assignments, expressions, etc.)
    pub other_statements: Vec<Stmt>,
}

/// Extended categories for cross-module statement reordering
#[derive(Debug, Default, Clone)]
pub struct CrossModuleStatementCategories {
    /// Import statements
    pub imports: Vec<Stmt>,
    /// Built-in type restorations (e.g., bytes = bytes)
    pub builtin_restorations: Vec<Stmt>,
    /// Namespace built-in assignments (e.g., compat.bytes = bytes)
    pub namespace_builtin_assignments: Vec<Stmt>,
    /// Assignments that define base classes
    pub base_class_assignments: Vec<Stmt>,
    /// Regular assignments
    pub regular_assignments: Vec<Stmt>,
    /// Class definitions
    pub classes: Vec<Stmt>,
    /// Function definitions
    pub functions: Vec<Stmt>,
    /// Other statements
    pub other_statements: Vec<Stmt>,
}

/// Categorizer for analyzing and grouping statements by type and dependencies
pub struct StatementCategorizer {
    /// Python version for built-in detection
    python_version: u8,
}

impl StatementCategorizer {
    /// Create a new categorizer
    pub fn new(python_version: u8) -> Self {
        Self { python_version }
    }

    /// Analyze and categorize statements for proper declaration order
    ///
    /// This performs two passes:
    /// 1. First pass: Collect symbols used as dependencies (base classes, metaclasses, decorators)
    /// 2. Second pass: Categorize statements based on their role and dependencies
    pub fn analyze_statements(&self, statements: Vec<Stmt>) -> StatementCategories {
        // First pass: identify all symbols used as dependencies
        let dependency_symbols =
            crate::visitors::ClassDefDependencyCollector::collect_from_statements(&statements);

        // Second pass: categorize statements using visitor
        let mut visitor = StatementCategorizationVisitor {
            categories: StatementCategories::default(),
            dependency_symbols,
        };

        for stmt in statements {
            visitor.categorize_statement(stmt);
        }

        visitor.categories
    }

    /// Analyze statements for cross-module reordering
    ///
    /// This handles additional categories needed when combining statements from multiple modules,
    /// such as built-in type restorations and namespace assignments.
    pub fn analyze_cross_module_statements(
        &self,
        statements: Vec<Stmt>,
    ) -> CrossModuleStatementCategories {
        // First pass: identify all symbols used as dependencies
        let dependency_symbols =
            crate::visitors::ClassDefDependencyCollector::collect_from_statements(&statements);

        // Second pass: categorize statements using visitor
        let mut visitor = CrossModuleCategorizationVisitor {
            categories: CrossModuleStatementCategories::default(),
            dependency_symbols,
            python_version: self.python_version,
        };

        for stmt in statements {
            visitor.categorize_statement(stmt);
        }

        visitor.categories
    }
}

/// Internal visitor for categorizing statements
struct StatementCategorizationVisitor {
    categories: StatementCategories,
    dependency_symbols: FxIndexSet<String>,
}

impl StatementCategorizationVisitor {
    fn categorize_statement(&mut self, stmt: Stmt) {
        match stmt {
            Stmt::Import(_) | Stmt::ImportFrom(_) => {
                self.categories.imports.push(stmt);
            }
            Stmt::Assign(ref assign) => {
                // Check if this is a class attribute assignment (e.g., MyClass.__module__ = 'foo')
                if self.is_class_attribute_assignment(assign) {
                    self.categories.other_statements.push(stmt);
                    return;
                }

                // Check if this is a self-assignment (e.g., validate = validate)
                if self.is_self_assignment(assign) {
                    self.categories.self_assignments.push(stmt);
                    return;
                }

                // Check if this assignment defines a dependency symbol
                if self.defines_dependency(assign) {
                    self.categories.dependency_assignments.push(stmt);
                } else {
                    self.categories.regular_assignments.push(stmt);
                }
            }
            Stmt::AnnAssign(ref ann_assign) => {
                // Check if this annotated assignment defines a dependency symbol
                if let Expr::Name(target) = ann_assign.target.as_ref()
                    && self.dependency_symbols.contains(target.id.as_str())
                {
                    self.categories.dependency_assignments.push(stmt);
                    return;
                }
                self.categories.regular_assignments.push(stmt);
            }
            Stmt::FunctionDef(_) => {
                self.categories.functions.push(stmt);
            }
            Stmt::ClassDef(_) => {
                self.categories.classes.push(stmt);
            }
            _ => {
                self.categories.other_statements.push(stmt);
            }
        }
    }

    fn is_class_attribute_assignment(&self, assign: &ruff_python_ast::StmtAssign) -> bool {
        if assign.targets.len() != 1 {
            return false;
        }

        if let Expr::Attribute(attr) = &assign.targets[0] {
            matches!(attr.value.as_ref(), Expr::Name(_))
        } else {
            false
        }
    }

    fn is_self_assignment(&self, assign: &ruff_python_ast::StmtAssign) -> bool {
        if assign.targets.len() != 1 {
            return false;
        }

        if let (Expr::Name(target), Expr::Name(value)) = (&assign.targets[0], assign.value.as_ref())
        {
            target.id == value.id
        } else {
            false
        }
    }

    fn defines_dependency(&self, assign: &ruff_python_ast::StmtAssign) -> bool {
        if assign.targets.len() != 1 {
            return false;
        }

        if let Expr::Name(target) = &assign.targets[0] {
            if self.dependency_symbols.contains(target.id.as_str()) {
                // Check if the value looks like it could be a class
                match assign.value.as_ref() {
                    Expr::Attribute(_) => true, // e.g., json.JSONDecodeError
                    Expr::Name(name) => {
                        // Check if it looks like a class name (starts with uppercase)
                        name.id.chars().next().is_some_and(char::is_uppercase)
                    }
                    _ => false,
                }
            } else {
                false
            }
        } else {
            false
        }
    }
}

/// Internal visitor for cross-module categorization
struct CrossModuleCategorizationVisitor {
    categories: CrossModuleStatementCategories,
    dependency_symbols: FxIndexSet<String>,
    python_version: u8,
}

impl CrossModuleCategorizationVisitor {
    fn categorize_statement(&mut self, stmt: Stmt) {
        match stmt {
            Stmt::Import(_) | Stmt::ImportFrom(_) => {
                self.categories.imports.push(stmt);
            }
            Stmt::Assign(ref assign) => {
                // Check if this is an attribute assignment
                if self.is_attribute_assignment(assign) {
                    // Check for namespace built-in assignment (e.g., compat.bytes = bytes)
                    if self.is_namespace_builtin_assignment(assign) {
                        self.categories.namespace_builtin_assignments.push(stmt);
                        return;
                    }

                    // Check for module namespace assignment
                    if self.is_module_namespace_assignment(assign) {
                        self.categories.regular_assignments.push(stmt);
                        return;
                    }

                    // Other attribute assignments come after class definitions
                    self.categories.other_statements.push(stmt);
                    return;
                }

                // Check for built-in type restoration (e.g., bytes = bytes)
                if self.is_builtin_restoration(assign) {
                    self.categories.builtin_restorations.push(stmt);
                    return;
                }

                // Check if this defines a base class symbol
                if self.defines_base_class_for_cross_module(assign) {
                    self.categories.base_class_assignments.push(stmt);
                } else {
                    self.categories.regular_assignments.push(stmt);
                }
            }
            Stmt::ClassDef(_) => {
                self.categories.classes.push(stmt);
            }
            Stmt::FunctionDef(_) => {
                self.categories.functions.push(stmt);
            }
            _ => {
                self.categories.other_statements.push(stmt);
            }
        }
    }

    fn is_attribute_assignment(&self, assign: &ruff_python_ast::StmtAssign) -> bool {
        assign.targets.len() == 1 && matches!(&assign.targets[0], Expr::Attribute(_))
    }

    fn is_namespace_builtin_assignment(&self, assign: &ruff_python_ast::StmtAssign) -> bool {
        if let (Expr::Attribute(_), Expr::Name(value_name)) =
            (&assign.targets[0], assign.value.as_ref())
        {
            ruff_python_stdlib::builtins::is_python_builtin(
                value_name.id.as_str(),
                self.python_version,
                false,
            )
        } else {
            false
        }
    }

    fn is_module_namespace_assignment(&self, assign: &ruff_python_ast::StmtAssign) -> bool {
        if let Expr::Attribute(attr) = &assign.targets[0] {
            if let Expr::Name(name) = attr.value.as_ref() {
                let parent_name = name.id.as_str();
                let child_name = attr.attr.as_str();

                if let Expr::Name(value_name) = assign.value.as_ref() {
                    value_name.id.as_str() == child_name
                        || value_name.id.as_str() == format!("{parent_name}_{child_name}")
                        || value_name
                            .id
                            .as_str()
                            .starts_with(&format!("{child_name}_"))
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        }
    }

    fn is_builtin_restoration(&self, assign: &ruff_python_ast::StmtAssign) -> bool {
        if let ([Expr::Name(target)], Expr::Name(value)) =
            (assign.targets.as_slice(), assign.value.as_ref())
        {
            target.id == value.id
                && ruff_python_stdlib::builtins::is_python_builtin(
                    target.id.as_str(),
                    self.python_version,
                    false,
                )
        } else {
            false
        }
    }

    fn defines_base_class_for_cross_module(&self, assign: &ruff_python_ast::StmtAssign) -> bool {
        if assign.targets.len() != 1 {
            return false;
        }

        if let Expr::Name(target) = &assign.targets[0] {
            if self.dependency_symbols.contains(target.id.as_str()) {
                // Only check for attribute access patterns
                matches!(assign.value.as_ref(), Expr::Attribute(_))
            } else {
                false
            }
        } else {
            false
        }
    }
}

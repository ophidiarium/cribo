use crate::types::FxIndexSet;
use ruff_python_ast::visitor::source_order::{self, SourceOrderVisitor};
use ruff_python_ast::{Arguments, Decorator, Expr, Stmt};

/// Visitor that collects all symbols used as dependencies in class definitions.
///
/// This visitor performs a single pass through the statements to identify
/// which symbols are referenced as:
/// - Base classes
/// - Metaclasses
/// - Decorators (on both classes and functions)
///
/// This information is used during statement reordering to ensure that
/// dependency assignments are placed before the definitions that use them.
#[derive(Debug, Default)]
pub struct ClassDefDependencyCollector {
    /// Set of symbol names that are used as dependencies (base classes, metaclasses, decorators)
    dependency_symbols: FxIndexSet<String>,
}

impl ClassDefDependencyCollector {
    /// Create a new collector
    pub fn new() -> Self {
        Self::default()
    }

    /// Collect class dependencies from a list of statements
    pub fn collect_from_statements<'a, I>(statements: I) -> FxIndexSet<String>
    where
        I: IntoIterator<Item = &'a Stmt>,
    {
        let mut collector = Self::new();
        for stmt in statements {
            collector.visit_stmt(stmt);
        }
        collector.dependency_symbols
    }
}

impl<'a> SourceOrderVisitor<'a> for ClassDefDependencyCollector {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        // Let the visitor pattern handle all traversal
        source_order::walk_stmt(self, stmt);
    }

    fn visit_decorator(&mut self, decorator: &'a Decorator) {
        // Collect symbols from the decorator expression
        self.collect_decorator_symbols(decorator);

        // Continue visiting the decorator expression
        source_order::walk_decorator(self, decorator);
    }

    fn visit_arguments(&mut self, arguments: &'a Arguments) {
        // Collect base class symbols
        for base_expr in &arguments.args {
            if let Expr::Name(name_expr) = base_expr {
                self.dependency_symbols.insert(name_expr.id.to_string());
            }
        }

        // Collect metaclass symbol from keyword arguments
        for keyword in &arguments.keywords {
            if let Some(arg) = &keyword.arg
                && arg.as_str() == "metaclass"
                && let Expr::Name(name_expr) = &keyword.value
            {
                self.dependency_symbols.insert(name_expr.id.to_string());
            }
        }

        // Continue visiting arguments
        source_order::walk_arguments(self, arguments);
    }
}

impl ClassDefDependencyCollector {
    /// Collect symbols from decorators
    fn collect_decorator_symbols(&mut self, decorator: &Decorator) {
        // Collect simple decorator names (e.g., @my_decorator)
        if let Expr::Name(name_expr) = &decorator.expression {
            self.dependency_symbols.insert(name_expr.id.to_string());
        }
        // For decorator calls (e.g., @my_decorator(args)), collect the function name
        else if let Expr::Call(call) = &decorator.expression
            && let Expr::Name(name_expr) = call.func.as_ref()
        {
            self.dependency_symbols.insert(name_expr.id.to_string());
        }
        // Note: We don't collect attribute decorators like @module.decorator
        // as those don't need reordering
    }
}

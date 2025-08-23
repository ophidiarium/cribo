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
    fn visit_decorator(&mut self, decorator: &'a Decorator) {
        // Collect symbols from the decorator expression
        self.collect_decorator_symbols(decorator);

        // Continue visiting the decorator expression
        source_order::walk_decorator(self, decorator);
    }

    fn visit_arguments(&mut self, arguments: &'a Arguments) {
        // Collect base class symbols
        for base_expr in &arguments.args {
            if let Some(name) = Self::head_name(base_expr) {
                self.dependency_symbols.insert(name.to_string());
            }
        }

        // Collect metaclass symbol from keyword arguments
        for keyword in &arguments.keywords {
            if let Some(arg) = &keyword.arg
                && arg.as_str() == "metaclass"
                && let Some(name) = Self::head_name(&keyword.value)
            {
                self.dependency_symbols.insert(name.to_string());
            }
        }

        // Continue visiting arguments
        source_order::walk_arguments(self, arguments);
    }
}

impl ClassDefDependencyCollector {
    /// Collect symbols from decorators
    fn collect_decorator_symbols(&mut self, decorator: &Decorator) {
        if let Some(name) = Self::head_name(&decorator.expression) {
            self.dependency_symbols.insert(name.to_string());
        }
    }

    /// Return the head identifier of a name/attribute/call chain:
    /// - `Name` -> that name
    /// - `pkg.attr.sub` -> `pkg`
    /// - `call_or_attr(...)` -> head of `func`
    fn head_name(expr: &Expr) -> Option<&str> {
        match expr {
            Expr::Name(n) => Some(n.id.as_str()),
            Expr::Attribute(a) => {
                let mut cur = a.value.as_ref();
                loop {
                    match cur {
                        Expr::Name(n) => break Some(n.id.as_str()),
                        Expr::Attribute(inner) => cur = inner.value.as_ref(),
                        _ => break None,
                    }
                }
            }
            Expr::Call(c) => Self::head_name(c.func.as_ref()),
            Expr::Subscript(s) => Self::head_name(s.value.as_ref()),
            _ => None,
        }
    }
}

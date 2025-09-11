//! Symbol usage visitor for tracking which symbols are actually used in function bodies
//!
//! This visitor analyzes function bodies to determine which imported symbols are
//! actually used in runtime code (excluding type annotations which are evaluated
//! at module level in wrapper modules).

use ruff_python_ast::{
    Expr, Stmt,
    visitor::source_order::{self, SourceOrderVisitor},
};

use crate::types::FxIndexSet;

/// Common type hint identifiers that are typically used in subscript expressions
/// like List[str], Dict[str, int], etc. These are often not runtime values.
///
/// Using a const array for better performance and deterministic ordering.
const TYPE_HINT_IDENTIFIERS: &[&str] = &[
    // Built-in generic types (typing module)
    "List",
    "Dict",
    "Set",
    "Tuple",
    // PEP 585 built-in generic types (lowercase)
    "list",
    "dict",
    "set",
    "tuple",
    // Optional and Union types
    "Optional",
    "Union",
    // Callable and function types
    "Callable",
    "Type",
    // Generic type system
    "Any",
    "TypeVar",
    "Generic",
    // Literal and final types
    "Literal",
    "Final",
    "ClassVar",
    // Metadata and annotations
    "Annotated",
    "Self",
];

/// Visitor that collects symbols that are actually used in a function body
#[derive(Default)]
pub struct SymbolUsageVisitor {
    /// Set of symbol names that are used in the body
    used_names: FxIndexSet<String>,
    /// Whether we're currently inside a type annotation context
    in_annotation: bool,
    /// Track depth of annotation nesting (for complex annotations)
    annotation_depth: usize,
}

impl SymbolUsageVisitor {
    /// Create a new symbol usage visitor
    pub fn new() -> Self {
        Self::default()
    }

    /// Collect all symbols used in a function body
    pub fn collect_used_symbols(body: &[Stmt]) -> FxIndexSet<String> {
        let mut visitor = Self::new();
        visitor.visit_body(body);
        visitor.used_names
    }

    /// Track a name usage if we're not in an annotation context
    fn track_name(&mut self, name: &str) {
        if !self.in_annotation {
            self.used_names.insert(name.to_string());
        }
    }

    /// Start annotation context
    fn enter_annotation(&mut self) {
        if self.annotation_depth == 0 {
            self.in_annotation = true;
        }
        self.annotation_depth += 1;
    }

    /// End annotation context
    fn exit_annotation(&mut self) {
        if self.annotation_depth > 0 {
            self.annotation_depth -= 1;
            if self.annotation_depth == 0 {
                self.in_annotation = false;
            }
        }
    }
}

impl<'a> SourceOrderVisitor<'a> for SymbolUsageVisitor {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            // Handle annotated assignments - annotation is not runtime code
            Stmt::AnnAssign(ann_assign) => {
                // Visit annotation in annotation context
                self.enter_annotation();
                self.visit_expr(&ann_assign.annotation);
                self.exit_annotation();

                // Visit target normally (it's a runtime assignment target)
                self.visit_expr(&ann_assign.target);

                // Visit value if present (runtime code)
                if let Some(value) = &ann_assign.value {
                    self.visit_expr(value);
                }
            }
            // Handle function definitions - annotations are not runtime
            Stmt::FunctionDef(func) => {
                // Don't track the function name itself as "used"
                // (it's being defined, not used)

                // Visit parameter annotations in annotation context
                for param in &func.parameters.args {
                    if let Some(annotation) = &param.parameter.annotation {
                        self.enter_annotation();
                        self.visit_expr(annotation);
                        self.exit_annotation();
                    }
                    // Visit default value normally (it's runtime code)
                    if let Some(default) = &param.default {
                        self.visit_expr(default);
                    }
                }

                // Handle other parameter types similarly
                for param in &func.parameters.posonlyargs {
                    if let Some(annotation) = &param.parameter.annotation {
                        self.enter_annotation();
                        self.visit_expr(annotation);
                        self.exit_annotation();
                    }
                    if let Some(default) = &param.default {
                        self.visit_expr(default);
                    }
                }

                for param in &func.parameters.kwonlyargs {
                    if let Some(annotation) = &param.parameter.annotation {
                        self.enter_annotation();
                        self.visit_expr(annotation);
                        self.exit_annotation();
                    }
                    if let Some(default) = &param.default {
                        self.visit_expr(default);
                    }
                }

                if let Some(param) = &func.parameters.vararg
                    && let Some(annotation) = &param.annotation
                {
                    self.enter_annotation();
                    self.visit_expr(annotation);
                    self.exit_annotation();
                }

                if let Some(param) = &func.parameters.kwarg
                    && let Some(annotation) = &param.annotation
                {
                    self.enter_annotation();
                    self.visit_expr(annotation);
                    self.exit_annotation();
                }

                // Visit return annotation in annotation context
                if let Some(returns) = &func.returns {
                    self.enter_annotation();
                    self.visit_expr(returns);
                    self.exit_annotation();
                }

                // Visit decorators normally (they're runtime code)
                for decorator in &func.decorator_list {
                    self.visit_expr(&decorator.expression);
                }

                // Visit function body normally
                self.visit_body(&func.body);
            }
            // Handle class definitions similarly
            Stmt::ClassDef(class) => {
                // Visit decorators (runtime)
                for decorator in &class.decorator_list {
                    self.visit_expr(&decorator.expression);
                }

                // Visit base classes (runtime - they're evaluated when class is created)
                for base in class.bases() {
                    self.visit_expr(base);
                }

                // Visit keywords (runtime)
                for keyword in class.keywords() {
                    self.visit_expr(&keyword.value);
                }

                // Visit class body
                self.visit_body(&class.body);
            }
            // Handle type alias statements (PEP 695) - available in Python 3.12+
            Stmt::TypeAlias(type_alias) => {
                // The alias name itself is not "used" (it's being defined)
                // The RHS expression is annotation-only and should not count as runtime usage
                self.enter_annotation();
                self.visit_expr(&type_alias.value);
                self.exit_annotation();

                // Visit type parameters if present (also annotation-only)
                if let Some(type_params) = &type_alias.type_params {
                    self.enter_annotation();
                    self.visit_type_params(type_params);
                    self.exit_annotation();
                }
            }
            _ => {
                // For all other statements, use default traversal
                source_order::walk_stmt(self, stmt);
            }
        }
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Name(name) => {
                // Track the name if we're not in an annotation
                self.track_name(&name.id);
            }
            // For subscript expressions like List[str], the subscript part is annotation-like
            Expr::Subscript(subscript) if self.could_be_type_hint(&subscript.value) => {
                // Visit the value part normally
                self.visit_expr(&subscript.value);

                // Visit the slice in annotation context if this looks like a type hint
                self.enter_annotation();
                self.visit_expr(&subscript.slice);
                self.exit_annotation();
            }
            _ => {
                // For all other expressions, use default traversal
                source_order::walk_expr(self, expr);
            }
        }
    }
}

impl SymbolUsageVisitor {
    /// Check if an expression could be a type hint base (like List, Dict, Optional, etc.)
    ///
    /// This uses pattern matching on the AST structure to detect common type hint patterns:
    /// - Direct names like `List`, `Dict`, `Optional` (typing module)
    /// - PEP 585 builtins like `list`, `dict`, `tuple` (lowercase)
    /// - Qualified names like `typing.List`, `typing_extensions.Literal`
    fn could_be_type_hint(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Name(name) => {
                // Check against our const array of known type hint identifiers
                TYPE_HINT_IDENTIFIERS.contains(&name.id.as_str())
            }
            Expr::Attribute(attr) => {
                // Handle typing.* and typing_extensions.* qualified names
                match attr.value.as_ref() {
                    Expr::Name(module_name) => {
                        module_name.id == "typing" || module_name.id == "typing_extensions"
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use ruff_python_parser::{Mode, parse};

    use super::*;

    fn parse_and_collect(code: &str) -> FxIndexSet<String> {
        let parsed = parse(code, Mode::Module.into()).expect("Failed to parse");
        match parsed.into_syntax() {
            ruff_python_ast::Mod::Module(module) => {
                SymbolUsageVisitor::collect_used_symbols(&module.body)
            }
            _ => panic!("Expected module"),
        }
    }

    #[test]
    fn test_basic_name_usage() {
        let code = r"
x = 1
y = x + 2
print(y)
";
        let used = parse_and_collect(code);
        assert!(used.contains("x"));
        assert!(used.contains("y"));
        assert!(used.contains("print"));
    }

    #[test]
    fn test_annotation_not_counted() {
        let code = r"
def foo(x: MyType) -> MyReturnType:
    return x
";
        let used = parse_and_collect(code);
        assert!(used.contains("x"));
        assert!(!used.contains("MyType"));
        assert!(!used.contains("MyReturnType"));
    }

    #[test]
    fn test_annassign_annotation_not_counted() {
        let code = r"
x: MyType = 5
y = x + 1
";
        let used = parse_and_collect(code);
        assert!(used.contains("x"));
        assert!(!used.contains("MyType"));
    }

    #[test]
    fn test_decorator_is_counted() {
        let code = r"
@my_decorator
def foo():
    pass
";
        let used = parse_and_collect(code);
        assert!(used.contains("my_decorator"));
    }

    #[test]
    fn test_class_bases_counted() {
        let code = r"
class MyClass(BaseClass, metaclass=MetaClass):
    pass
";
        let used = parse_and_collect(code);
        assert!(used.contains("BaseClass"));
        assert!(used.contains("MetaClass"));
    }

    #[test]
    fn test_type_alias_annotation_not_counted() {
        // Note: type aliases are PEP 695 (Python 3.12+)
        let code = r"
type MyAlias = list[str]
x = MyAlias()
";
        let used = parse_and_collect(code);
        assert!(used.contains("MyAlias")); // Runtime usage
        assert!(!used.contains("list")); // Type annotation - not runtime usage 
        assert!(!used.contains("str")); // Type annotation - not runtime usage
    }
}

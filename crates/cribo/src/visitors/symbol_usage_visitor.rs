//! Symbol usage visitor for tracking which symbols are actually used in function bodies
//!
//! This visitor analyzes function bodies to determine which imported symbols are
//! actually used in runtime code (excluding type annotations which are evaluated
//! at module level in wrapper modules).

use ruff_python_ast::{
    Expr, Stmt,
    visitor::{self, Visitor},
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
pub(crate) struct SymbolUsageVisitor {
    /// Set of symbol names that are used in the body
    used_names: FxIndexSet<String>,
}

impl SymbolUsageVisitor {
    /// Create a new symbol usage visitor
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Collect all symbols used in a function body
    pub(crate) fn collect_used_symbols(body: &[Stmt]) -> FxIndexSet<String> {
        let mut visitor = Self::new();
        visitor.visit_body(body);
        visitor.used_names
    }

    /// Track a name usage
    fn track_name(&mut self, name: &str) {
        self.used_names.insert(name.to_owned());
    }
}

impl<'a> Visitor<'a> for SymbolUsageVisitor {
    fn visit_annotation(&mut self, _expr: &'a Expr) {
        // Don't track names in annotations - they're not runtime usage
        // By not calling the default walk, we skip all names in annotations
        // Note: This is only called for function returns, parameters, and AnnAssign
    }

    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            // Type alias values and type parameters are not covered by visit_annotation
            // so we need to skip them manually
            Stmt::TypeAlias(_) => {
                // The alias name itself is not "used" (it's being defined)
                // The value and type params are type annotations, not runtime
                // Don't visit them at all
            }
            _ => {
                // For other statements, use default traversal
                // The framework will call visit_annotation for actual annotations
                visitor::walk_stmt(self, stmt);
            }
        }
    }

    fn visit_type_params(&mut self, _type_params: &'a ruff_python_ast::TypeParams) {
        // Don't track anything in type parameters - they're not runtime usage
        // By not calling the default walk, we skip all names in type parameters
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Name(name) => {
                // Track the name - we're in runtime context
                // (annotations are handled separately by visit_annotation)
                self.track_name(&name.id);
            }
            // For subscript expressions like List[str], the subscript part is annotation-like
            Expr::Subscript(subscript) if self.could_be_type_hint(&subscript.value) => {
                // Visit the value part normally
                self.visit_expr(&subscript.value);
                // Don't visit the slice if this looks like a type hint
                // (skip the subscript part of type hints like List[str])
            }
            // typing.cast(T, expr) — treat T as annotation-only
            Expr::Call(call) if self.is_typing_cast(&call.func) => {
                // Visit callee (runtime)
                self.visit_expr(&call.func);
                // Skip first positional arg (type annotation)
                // Visit remaining args/keywords (runtime)
                for arg in call.arguments.args.iter().skip(1) {
                    self.visit_expr(arg);
                }
                for kw in &call.arguments.keywords {
                    self.visit_expr(&kw.value);
                }
            }
            _ => {
                // For all other expressions, use default traversal
                visitor::walk_expr(self, expr);
            }
        }
    }
}

impl SymbolUsageVisitor {
    /// Check if a function call is typing.cast or `typing_extensions.cast`
    ///
    /// Recognizes both direct imports (cast) and qualified calls (typing.cast)
    fn is_typing_cast(&self, func: &Expr) -> bool {
        match func {
            Expr::Name(name) => name.id.as_str() == "cast",
            Expr::Attribute(attr) => {
                self.is_attribute_from_known_typing_module(attr) && attr.attr.as_str() == "cast"
            }
            _ => false,
        }
    }

    /// Check if an attribute expression comes from a known typing-related module
    ///
    /// This walks the attribute chain to find the root module name and checks if it's
    /// from a known typing-related module like `typing`, `typing_extensions`, or `collections`.
    fn is_attribute_from_known_typing_module(&self, attr: &ruff_python_ast::ExprAttribute) -> bool {
        // Walk the attribute chain to find the root module name
        let root_name = Self::get_root_module_name(&attr.value);

        match root_name.as_deref() {
            Some("typing" | "typing_extensions") => true,
            // Only collections.abc.* are typing-related
            Some("collections") => matches!(
                &*attr.value,
                Expr::Attribute(inner)
                    if matches!(&*inner.value, Expr::Name(root) if root.id == "collections")
                    && inner.attr.as_str() == "abc"
            ),
            _ => false,
        }
    }

    /// Get the root module name from a potentially nested attribute expression
    ///
    /// For example:
    /// - `collections.abc.Callable` -> Some("collections")
    /// - `typing.List` -> Some("typing")
    /// - `SomeClass.method` -> Some("SomeClass")
    fn get_root_module_name(expr: &Expr) -> Option<String> {
        match expr {
            Expr::Name(name) => Some(name.id.to_string()),
            Expr::Attribute(attr) => Self::get_root_module_name(&attr.value),
            _ => None,
        }
    }

    /// Check if an expression could be a type hint base (like List, Dict, Optional, etc.)
    ///
    /// This uses pattern matching on the AST structure to detect common type hint patterns:
    /// - Direct names like `List`, `Dict`, `Optional` (typing module)
    /// - PEP 585 builtins like `list`, `dict`, `tuple` (lowercase)
    /// - Qualified names like `typing.List`, `typing_extensions.Literal`,
    ///   `collections.abc.Callable`
    fn could_be_type_hint(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Name(name) => {
                // Check against our const array of known type hint identifiers
                TYPE_HINT_IDENTIFIERS.contains(&name.id.as_str())
            }
            Expr::Attribute(attr) => {
                // Handle qualified names from known typing modules or with type hint attribute
                // names
                self.is_attribute_from_known_typing_module(attr)
                    || TYPE_HINT_IDENTIFIERS.contains(&attr.attr.as_str())
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
            ruff_python_ast::Mod::Expression(_) => panic!("Expected module"),
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

    #[test]
    fn test_collections_abc_type_hints_not_counted() {
        let code = r"
from collections.abc import Callable
from typing import List
x: List[Callable[[int], str]] = []
y = x
";
        let used = parse_and_collect(code);
        assert!(used.contains("x")); // Runtime usage
        assert!(used.contains("y")); // Runtime usage  
        assert!(!used.contains("List")); // Type annotation - not runtime usage
        assert!(!used.contains("Callable")); // Type annotation - not runtime usage
        assert!(!used.contains("int")); // Type annotation - not runtime usage
        assert!(!used.contains("str")); // Type annotation - not runtime usage
    }

    #[test]
    fn test_annotation_context_balance() {
        // This test ensures that annotations are properly excluded
        // even with complex nested annotation patterns
        let code = r"
from typing import Dict, List, Optional
def func(
    x: Dict[str, List[Optional[int]]],
    y: Optional[Dict[str, int]] = None
) -> List[str]:
    return [str(x), str(y)]
";
        let used = parse_and_collect(code);

        // Verify runtime symbols are tracked correctly
        assert!(used.contains("str"));
        assert!(used.contains("x"));
        assert!(used.contains("y"));
        // Verify type annotations are not tracked
        assert!(!used.contains("Dict"));
        assert!(!used.contains("List"));
        assert!(!used.contains("Optional"));
        assert!(!used.contains("int"));
    }

    #[test]
    fn test_class_type_parameters_not_counted() {
        // Test PEP 695 class type parameters (Python 3.12+)
        let code = r"
from typing import TypeVar
class Container[T: TypeVar]:
    def __init__(self, value: T):
        self.value = value
    def get(self) -> T:
        return self.value
x = Container('hello')
";
        let used = parse_and_collect(code);
        assert!(used.contains("Container")); // Runtime usage
        assert!(used.contains("x")); // Runtime usage
        assert!(used.contains("self")); // Runtime usage
        assert!(!used.contains("T")); // Type parameter - not runtime usage
        assert!(!used.contains("TypeVar")); // Type annotation - not runtime usage
    }

    #[test]
    fn test_collections_abc_vs_collections_distinction() {
        // Test that collections.abc.* is treated as type hint but collections.* is not
        let code = r"
from collections.abc import Callable
from collections import deque
x: Callable[[int], str] = lambda n: str(n)
y = deque([1, 2, 3])
print(x, y)
";
        let used = parse_and_collect(code);
        assert!(used.contains("str")); // Runtime usage (function call in lambda)
        assert!(used.contains("deque")); // Runtime usage
        assert!(used.contains("x")); // Runtime usage (variable access in print)
        assert!(used.contains("y")); // Runtime usage (variable access in print)
        assert!(used.contains("print")); // Runtime usage
        assert!(!used.contains("Callable")); // Type annotation - not runtime usage
        assert!(!used.contains("int")); // Type annotation - not runtime usage
    }

    #[test]
    fn test_async_function_annotations_not_counted() {
        // Test that async function annotations are treated the same as regular function annotations
        let code = r"
async def async_func(x: MyType, y: int = 42) -> MyReturnType:
    return str(x) + str(y)

def regular_func(x: MyType, y: int = 42) -> MyReturnType:
    return str(x) + str(y)
";
        let used = parse_and_collect(code);
        // Runtime usage in both functions
        assert!(used.contains("str")); // Runtime usage (function calls)
        assert!(used.contains("x")); // Runtime usage (parameter access)
        assert!(used.contains("y")); // Runtime usage (parameter access)
        // Type annotations should not be counted for either function
        assert!(!used.contains("MyType")); // Type annotation - not runtime usage
        assert!(!used.contains("MyReturnType")); // Type annotation - not runtime usage
        assert!(!used.contains("int")); // Type annotation - not runtime usage
    }

    #[test]
    fn test_typing_cast_first_argument_not_counted() {
        // Test that typing.cast first argument is treated as annotation-only
        let code = r"
from typing import cast
import typing
value = cast(MyType, some_expression)
value2 = typing.cast(AnotherType, another_expression)
result = str(value) + str(value2)
";
        let used = parse_and_collect(code);
        // Runtime usage
        assert!(used.contains("cast")); // Runtime usage (function call)
        assert!(used.contains("typing")); // Runtime usage (module access)
        assert!(used.contains("some_expression")); // Runtime usage (second argument)
        assert!(used.contains("another_expression")); // Runtime usage (second argument)
        assert!(used.contains("value")); // Runtime usage (variable access)
        assert!(used.contains("value2")); // Runtime usage (variable access)
        assert!(used.contains("str")); // Runtime usage (function call)
        assert!(used.contains("result")); // Runtime usage (variable assignment)
        // Type annotations should not be counted
        assert!(!used.contains("MyType")); // Type annotation (first arg to cast) - not runtime usage
        assert!(!used.contains("AnotherType")); // Type annotation (first arg to cast) - not runtime usage
    }

    #[test]
    fn test_function_type_parameters_not_counted() {
        // Test PEP 695 function type parameters (Python 3.12+)
        let code = r"
def generic_func[T, U: Bound](x: T, y: U) -> tuple[T, U]:
    return (x, y)
result = generic_func(42, 'hello')
";
        let used = parse_and_collect(code);
        // Runtime usage
        assert!(used.contains("x")); // Runtime usage (parameter access)
        assert!(used.contains("y")); // Runtime usage (parameter access)
        assert!(used.contains("generic_func")); // Runtime usage (function call)
        assert!(used.contains("result")); // Runtime usage (variable assignment)
        // Type parameters should not be counted
        assert!(!used.contains("T")); // Type parameter - not runtime usage
        assert!(!used.contains("U")); // Type parameter - not runtime usage
        assert!(!used.contains("Bound")); // Type parameter bound - not runtime usage
        assert!(!used.contains("tuple")); // Type annotation - not runtime usage
    }
}

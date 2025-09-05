//! Expression AST node factory functions
//!
//! This module provides factory functions for creating various types of expression AST nodes.
//! All expressions are created with `TextRange::default()` and `AtomicNodeIndex::dummy()`
//! to indicate their synthetic nature.

use ruff_python_ast::{
    AtomicNodeIndex, BoolOp, CmpOp, Expr, ExprAttribute, ExprBinOp, ExprBoolOp, ExprBooleanLiteral,
    ExprCall, ExprCompare, ExprContext, ExprIf, ExprList, ExprName, ExprNoneLiteral,
    ExprStringLiteral, ExprSubscript, ExprTuple, ExprUnaryOp, FStringFlags, FStringPart,
    FStringValue, Keyword, Operator, StringLiteral, StringLiteralFlags, StringLiteralValue,
    UnaryOp,
};
use ruff_text_size::TextRange;

/// Creates a name expression node.
///
/// # Arguments
/// * `name` - The identifier name
/// * `ctx` - The expression context (Load, Store, Del)
///
/// # Example
/// ```rust
/// // Creates: `variable_name`
/// let expr = name("variable_name", ExprContext::Load);
/// ```
pub fn name(name: &str, ctx: ExprContext) -> Expr {
    Expr::Name(ExprName {
        id: name.to_string().into(),
        ctx,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates an attribute access expression node.
///
/// # Arguments
/// * `value` - The base expression being accessed
/// * `attr` - The attribute name
/// * `ctx` - The expression context (Load, Store, Del)
///
/// # Example
/// ```rust
/// // Creates: `module.attribute`
/// let base = name("module", ExprContext::Load);
/// let expr = attribute(base, "attribute", ExprContext::Load);
/// ```
pub fn attribute(value: Expr, attr: &str, ctx: ExprContext) -> Expr {
    Expr::Attribute(ExprAttribute {
        value: Box::new(value),
        attr: ruff_python_ast::Identifier::new(attr, TextRange::default()),
        ctx,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates an attribute access expression from a name.
///
/// This is a convenience function for the common pattern of accessing
/// an attribute on a simple name expression.
///
/// # Arguments
/// * `base_name` - The name of the base object
/// * `attr` - The attribute name
/// * `ctx` - The expression context (Load, Store, Del) for the attribute access
///
/// # Example
/// ```rust
/// // Creates: `module.attribute`
/// let expr = name_attribute("module", "attribute", ExprContext::Load);
/// ```
///
/// # Note
/// The base name is always loaded (`ExprContext::Load`) regardless of the
/// context of the attribute access, as per Python AST semantics.
pub fn name_attribute(base_name: &str, attr: &str, ctx: ExprContext) -> Expr {
    attribute(name(base_name, ExprContext::Load), attr, ctx)
}

/// Creates a string literal expression node.
///
/// # Arguments
/// * `value` - The string value
///
/// # Example
/// ```rust
/// // Creates: `"hello world"`
/// let expr = string_literal("hello world");
/// ```
pub fn string_literal(value: &str) -> Expr {
    Expr::StringLiteral(ExprStringLiteral {
        value: StringLiteralValue::single(StringLiteral {
            range: TextRange::default(),
            value: value.into(),
            flags: StringLiteralFlags::empty(),
            node_index: AtomicNodeIndex::dummy(),
        }),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates a None literal expression node.
///
/// # Example
/// ```rust
/// // Creates: `None`
/// let expr = none_literal();
/// ```
pub fn none_literal() -> Expr {
    Expr::NoneLiteral(ExprNoneLiteral {
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates a boolean literal expression node.
///
/// # Arguments
/// * `value` - The boolean value
///
/// # Example
/// ```rust
/// // Creates: `True` or `False`
/// let true_expr = bool_literal(true);
/// let false_expr = bool_literal(false);
/// ```
pub fn bool_literal(value: bool) -> Expr {
    Expr::BooleanLiteral(ExprBooleanLiteral {
        value,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates a function call expression node.
///
/// # Arguments
/// * `func` - The function being called
/// * `args` - Positional arguments
/// * `keywords` - Keyword arguments
///
/// # Example
/// ```rust
/// // Creates: `func(arg1, key=value)`
/// let func_expr = name("func", ExprContext::Load);
/// let arg = string_literal("arg1");
/// let keyword = keyword("key", string_literal("value"));
/// let expr = call(func_expr, vec![arg], vec![keyword]);
/// ```
pub fn call(func: Expr, args: Vec<Expr>, keywords: Vec<Keyword>) -> Expr {
    Expr::Call(ExprCall {
        func: Box::new(func),
        arguments: ruff_python_ast::Arguments {
            args: args.into_boxed_slice(),
            keywords: keywords.into_boxed_slice(),
            range: TextRange::default(),
            node_index: AtomicNodeIndex::dummy(),
        },
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates a dotted name expression by chaining attribute accesses.
///
/// # Arguments
/// * `parts` - The parts of the dotted name (e.g., `["sys", "modules"]` for `sys.modules`)
/// * `ctx` - The expression context
///
/// # Example
/// ```rust
/// // Creates: `sys.modules.get`
/// let expr = dotted_name(&["sys", "modules", "get"], ExprContext::Load);
/// ```
pub fn dotted_name(parts: &[&str], ctx: ExprContext) -> Expr {
    assert!(
        !parts.is_empty(),
        "Cannot create a dotted name: the 'parts' array must contain at least one string. Ensure \
         the input is non-empty before calling this function."
    );

    let mut result = name(
        parts[0],
        if parts.len() == 1 {
            ctx
        } else {
            ExprContext::Load
        },
    );
    for (i, &part) in parts.iter().enumerate().skip(1) {
        if i == parts.len() - 1 {
            result = attribute(result, part, ctx);
        } else {
            result = attribute(result, part, ExprContext::Load);
        }
    }
    result
}

/// Creates a module reference expression, handling both simple and dotted names.
///
/// This is a convenience function that automatically chooses between creating
/// a simple name expression or a dotted name expression based on whether the
/// module name contains dots.
///
/// # Arguments
/// * `module_name` - The module name (e.g., "math" or "os.path")
/// * `ctx` - The expression context
///
/// # Example
/// ```rust
/// // Creates: `math` (simple name)
/// let expr = module_reference("math", ExprContext::Load);
///
/// // Creates: `os.path` (dotted name)
/// let expr = module_reference("os.path", ExprContext::Load);
/// ```
pub fn module_reference(module_name: &str, ctx: ExprContext) -> Expr {
    if module_name.contains('.') {
        let parts: Vec<&str> = module_name.split('.').collect();
        dotted_name(&parts, ctx)
    } else {
        name(module_name, ctx)
    }
}

/// Creates a list expression node.
///
/// # Arguments
/// * `elts` - The list elements
/// * `ctx` - The expression context
///
/// # Example
/// ```rust
/// // Creates: `[a, b, c]`
/// let elements = vec![
///     name("a", ExprContext::Load),
///     name("b", ExprContext::Load),
///     name("c", ExprContext::Load),
/// ];
/// let expr = list(elements, ExprContext::Load);
/// ```
pub fn list(elts: Vec<Expr>, ctx: ExprContext) -> Expr {
    Expr::List(ExprList {
        elts,
        ctx,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates a unary operation expression node.
///
/// # Arguments
/// * `op` - The unary operator
/// * `operand` - The operand expression
///
/// # Example
/// ```rust
/// // Creates: `not x`
/// let operand = name("x", ExprContext::Load);
/// let expr = unary_op(UnaryOp::Not, operand);
/// ```
pub fn unary_op(op: UnaryOp, operand: Expr) -> Expr {
    Expr::UnaryOp(ExprUnaryOp {
        op,
        operand: Box::new(operand),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates a types.SimpleNamespace constructor expression.
///
/// This is a common pattern used throughout the bundling process for creating
/// namespace objects. With the stdlib proxy, this always uses
/// `_cribo.types.SimpleNamespace` (resolved lazily at runtime).
///
/// # Example
/// ```rust
/// // Creates: `_cribo.types.SimpleNamespace` (via proxy)
/// let ctor = simple_namespace_ctor();
/// ```
#[inline]
pub(crate) fn simple_namespace_ctor() -> Expr {
    // Resolve via the runtime stdlib proxy
    dotted_name(
        &[super::CRIBO_PREFIX, "types", "SimpleNamespace"],
        ExprContext::Load,
    )
}

/// Extracts the original flags from an f-string value.
///
/// This function searches through the f-string parts to find the first `FString` part
/// and returns its flags. If no `FString` part is found, returns empty flags.
///
/// # Arguments
/// * `value` - The `FStringValue` to extract flags from
///
/// # Example
/// ```rust
/// let flags = get_fstring_flags(&fstring_expr.value);
/// ```
pub fn get_fstring_flags(value: &FStringValue) -> FStringFlags {
    value
        .iter()
        .find_map(|part| {
            if let FStringPart::FString(f) = part {
                Some(f.flags)
            } else {
                None
            }
        })
        .unwrap_or_else(FStringFlags::empty)
}

/// Create a subscript expression: obj[key]
pub fn subscript(value: Expr, slice: Expr, ctx: ExprContext) -> Expr {
    Expr::Subscript(ExprSubscript {
        node_index: AtomicNodeIndex::dummy(),
        value: Box::new(value),
        slice: Box::new(slice),
        ctx,
        range: TextRange::default(),
    })
}

/// Creates a tuple expression node.
///
/// # Arguments
/// * `elts` - The tuple elements
///
/// # Example
/// ```rust
/// // Creates: `(a, b, c)`
/// let elements = vec![
///     name("a", ExprContext::Load),
///     name("b", ExprContext::Load),
///     name("c", ExprContext::Load),
/// ];
/// let expr = tuple(elements);
/// ```
pub fn tuple(elts: Vec<Expr>) -> Expr {
    Expr::Tuple(ExprTuple {
        elts,
        ctx: ExprContext::Load,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
        parenthesized: true,
    })
}

/// Creates a binary operation expression node.
///
/// # Arguments
/// * `left` - The left operand
/// * `op` - The binary operator
/// * `right` - The right operand
///
/// # Example
/// ```rust
/// // Creates: `a + b`
/// let left = name("a", ExprContext::Load);
/// let right = name("b", ExprContext::Load);
/// let expr = bin_op(left, Operator::Add, right);
/// ```
pub fn bin_op(left: Expr, op: Operator, right: Expr) -> Expr {
    Expr::BinOp(ExprBinOp {
        left: Box::new(left),
        op,
        right: Box::new(right),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates a boolean operation expression node.
///
/// # Arguments
/// * `op` - The boolean operator (And/Or)
/// * `values` - The operand expressions
///
/// # Example
/// ```rust
/// // Creates: `a or b`
/// let values = vec![name("a", ExprContext::Load), name("b", ExprContext::Load)];
/// let expr = bool_op(BoolOp::Or, values);
/// ```
pub fn bool_op(op: BoolOp, values: Vec<Expr>) -> Expr {
    Expr::BoolOp(ExprBoolOp {
        op,
        values,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates an if-expression (ternary conditional) node.
///
/// # Arguments
/// * `test` - The condition expression
/// * `body` - The expression to evaluate if true
/// * `orelse` - The expression to evaluate if false
///
/// # Example
/// ```rust
/// // Creates: `a if condition else b`
/// let condition = name("condition", ExprContext::Load);
/// let body = name("a", ExprContext::Load);
/// let orelse = name("b", ExprContext::Load);
/// let expr = if_exp(condition, body, orelse);
/// ```
pub fn if_exp(condition: Expr, body: Expr, orelse: Expr) -> Expr {
    Expr::If(ExprIf {
        test: Box::new(condition),
        body: Box::new(body),
        orelse: Box::new(orelse),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates an 'in' comparison expression node.
///
/// # Arguments
/// * `left` - The value to check
/// * `right` - The container to check in
///
/// # Example
/// ```rust
/// // Creates: `x in [1, 2, 3]`
/// let left = name("x", ExprContext::Load);
/// let right = list(vec![...], ExprContext::Load);
/// let expr = in_op(left, right);
/// ```
pub fn in_op(left: Expr, right: Expr) -> Expr {
    Expr::Compare(ExprCompare {
        left: Box::new(left),
        ops: Box::new([CmpOp::In]),
        comparators: Box::new([right]),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates a keyword argument node.
///
/// # Arguments
/// * `arg` - The keyword argument name (None for **kwargs)
/// * `value` - The value expression
///
/// # Example
/// ```rust
/// // Creates: `key=value` (as part of a function call)
/// let value_expr = name("value", ExprContext::Load);
/// let kw = keyword(Some("key"), value_expr);
/// ```
pub fn keyword(arg: Option<&str>, value: Expr) -> Keyword {
    Keyword {
        node_index: AtomicNodeIndex::dummy(),
        arg: arg.map(|s| ruff_python_ast::Identifier::new(s, TextRange::default())),
        value,
        range: TextRange::default(),
    }
}

//! Expression AST node factory functions
//!
//! This module provides factory functions for creating various types of expression AST nodes.
//! All expressions are created with `TextRange::default()` and `AtomicNodeIndex::dummy()`
//! to indicate their synthetic nature.

use ruff_python_ast::{
    AtomicNodeIndex, BoolOp, Expr, ExprAttribute, ExprBinOp, ExprBoolOp, ExprCall, ExprContext,
    ExprList, ExprName, ExprNoneLiteral, ExprSlice, ExprStringLiteral, ExprSubscript, ExprTuple,
    ExprUnaryOp, Keyword, Operator, StringLiteral, StringLiteralFlags, StringLiteralValue, UnaryOp,
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
    if parts.is_empty() {
        panic!(
            "Cannot create a dotted name: the 'parts' array must contain at least one string. \
             Ensure the input is non-empty before calling this function."
        );
    }

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

/// Creates a subscript expression node.
///
/// # Arguments
/// * `value` - The object being subscripted
/// * `slice` - The slice expression
/// * `ctx` - The expression context
///
/// # Example
/// ```rust
/// // Creates: `obj[key]`
/// let obj = name("obj", ExprContext::Load);
/// let key = string_literal("key");
/// let expr = subscript(obj, key, ExprContext::Load);
/// ```
pub fn subscript(value: Expr, slice: Expr, ctx: ExprContext) -> Expr {
    Expr::Subscript(ExprSubscript {
        value: Box::new(value),
        slice: Box::new(slice),
        ctx,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
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

/// Creates a tuple expression node.
///
/// # Arguments
/// * `elts` - The tuple elements
/// * `ctx` - The expression context
///
/// # Example
/// ```rust
/// // Creates: `(a, b, c)`
/// let elements = vec![
///     name("a", ExprContext::Load),
///     name("b", ExprContext::Load),
///     name("c", ExprContext::Load),
/// ];
/// let expr = tuple(elements, ExprContext::Load);
/// ```
pub fn tuple(elts: Vec<Expr>, ctx: ExprContext) -> Expr {
    Expr::Tuple(ExprTuple {
        elts,
        ctx,
        parenthesized: true,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates a boolean operation expression node.
///
/// # Arguments
/// * `op` - The boolean operator (And, Or)
/// * `values` - The operand expressions
///
/// # Example
/// ```rust
/// // Creates: `a and b`
/// let operands = vec![name("a", ExprContext::Load), name("b", ExprContext::Load)];
/// let expr = bool_op(BoolOp::And, operands);
/// ```
pub fn bool_op(op: BoolOp, values: Vec<Expr>) -> Expr {
    Expr::BoolOp(ExprBoolOp {
        op,
        values,
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
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

/// Creates a slice expression node.
///
/// # Arguments
/// * `lower` - The lower bound (optional)
/// * `upper` - The upper bound (optional)
/// * `step` - The step value (optional)
///
/// # Example
/// ```rust
/// // Creates: `1:10:2`
/// let expr = slice(
///     Some(string_literal("1")),
///     Some(string_literal("10")),
///     Some(string_literal("2")),
/// );
/// ```
pub fn slice(lower: Option<Expr>, upper: Option<Expr>, step: Option<Expr>) -> Expr {
    Expr::Slice(ExprSlice {
        lower: lower.map(Box::new),
        upper: upper.map(Box::new),
        step: step.map(Box::new),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Creates a types.SimpleNamespace constructor expression.
///
/// This is a common pattern used throughout the bundling process for creating
/// namespace objects.
///
/// # Example
/// ```rust
/// // Creates: `types.SimpleNamespace`
/// let ctor = simple_namespace_ctor();
/// ```
#[inline]
pub(crate) fn simple_namespace_ctor() -> Expr {
    dotted_name(&["types", "SimpleNamespace"], ExprContext::Load)
}

use std::fmt::Write;

use ruff_python_ast::{
    AnyStringFlags, Expr, ExprFString, ExprStringLiteral, FString, FStringPart,
    InterpolatedElement, InterpolatedStringElement, InterpolatedStringLiteralElement, Stmt,
    StringFlags, StringLiteral,
    helpers::is_docstring_stmt,
    str::{Quote, TripleQuotes},
    str_prefix::StringLiteralPrefix,
    visitor::{Visitor, walk_expr, walk_stmt},
};
use ruff_python_codegen::{Generator, Stylist};
use ruff_python_literal::{
    char::is_printable,
    escape::{Escape, UnicodeEscape},
};

use crate::types::FxIndexMap;

#[derive(Debug)]
struct StringReplacement {
    original: String,
    desired: String,
    should_indent: bool,
}

struct StringExprReplacementCollector<'sty> {
    stylist: &'sty Stylist<'sty>,
    replacements: FxIndexMap<String, ReplacementEntry>,
    in_docstring: bool,
}

#[derive(Debug)]
struct ReplacementEntry {
    desired: String,
    should_indent: bool,
}

impl StringExprReplacementCollector<'_> {
    fn handle_expr(&mut self, expr: &Expr) {
        let generated = Generator::from(self.stylist).expr(expr);
        let desired = match expr {
            Expr::StringLiteral(expr_lit) => {
                render_string_literal_expr(expr_lit).map(|s| (s, self.in_docstring))
            }
            Expr::FString(expr_f) => render_f_string_expr(self.stylist, expr_f),
            _ => None,
        };
        let Some((desired, should_indent)) = desired else {
            return;
        };
        if generated != desired {
            self.replacements.insert(
                generated,
                ReplacementEntry {
                    desired,
                    should_indent,
                },
            );
        }
    }
}

impl<'ast> Visitor<'ast> for StringExprReplacementCollector<'_> {
    fn visit_stmt(&mut self, stmt: &'ast Stmt) {
        let previous = self.in_docstring;
        self.in_docstring = is_docstring_stmt(stmt);
        walk_stmt(self, stmt);
        self.in_docstring = previous;
    }

    fn visit_expr(&mut self, expr: &'ast Expr) {
        if matches!(
            expr,
            Expr::StringLiteral(_) | Expr::FString(_) | Expr::TString(_)
        ) {
            self.handle_expr(expr);
        }
        walk_expr(self, expr);
    }
}

/// Render a statement while preserving multiline string formatting.
pub fn render_statement(stylist: &Stylist, stmt: &Stmt) -> String {
    let generator = Generator::from(stylist);
    let mut rendered = generator.stmt(stmt);

    let replacements = collect_string_expr_replacements(stylist, stmt);
    for replacement in replacements {
        rendered = apply_replacement(rendered, &replacement);
    }

    rendered
}

fn collect_string_expr_replacements(stylist: &Stylist, stmt: &Stmt) -> Vec<StringReplacement> {
    let mut collector = StringExprReplacementCollector {
        stylist,
        replacements: FxIndexMap::default(),
        in_docstring: false,
    };
    collector.visit_stmt(stmt);

    collector
        .replacements
        .into_iter()
        .map(|(original, entry)| StringReplacement {
            original,
            desired: entry.desired,
            should_indent: entry.should_indent,
        })
        .collect()
}

fn apply_replacement(mut rendered: String, replacement: &StringReplacement) -> String {
    let mut search_start = 0;
    while let Some(relative_pos) = rendered[search_start..].find(&replacement.original) {
        let absolute_pos = search_start + relative_pos;

        let line_start = rendered[..absolute_pos]
            .rfind('\n')
            .map_or(0, |idx| idx + 1);
        let indent = rendered[line_start..absolute_pos]
            .chars()
            .take_while(|c| c.is_whitespace())
            .collect::<String>();

        let adjusted = if replacement.should_indent {
            apply_indent(&replacement.desired, &indent)
        } else {
            replacement.desired.clone()
        };
        rendered.replace_range(
            absolute_pos..absolute_pos + replacement.original.len(),
            &adjusted,
        );
        search_start = absolute_pos + adjusted.len();
    }
    rendered
}

fn apply_indent(desired: &str, indent: &str) -> String {
    if indent.is_empty() || !desired.contains('\n') {
        return desired.to_string();
    }

    let mut lines = desired.split('\n');
    let mut result = String::new();

    if let Some(first_line) = lines.next() {
        result.push_str(first_line);
    }

    for line in lines {
        result.push('\n');
        result.push_str(indent);
        result.push_str(line);
    }

    result
}

fn render_string_literal_expr(expr: &ExprStringLiteral) -> Option<String> {
    let mut rendered_parts = Vec::new();
    for literal in &expr.value {
        let rendered = render_string_literal_part(literal)?;
        rendered_parts.push(rendered);
    }
    Some(rendered_parts.join(" "))
}

fn render_string_literal_part(literal: &StringLiteral) -> Option<String> {
    if matches!(literal.flags.prefix(), StringLiteralPrefix::Raw { .. }) {
        return None;
    }

    if literal.flags.triple_quotes() == TripleQuotes::Yes {
        Some(render_triple_quoted_literal(
            literal.value.as_ref(),
            literal.flags.quote_style(),
            literal.flags.prefix().as_str(),
        ))
    } else {
        render_non_triple_literal(
            literal.value.as_ref(),
            literal.flags.quote_style(),
            literal.flags.prefix().as_str(),
        )
    }
}

fn render_f_string_expr(stylist: &Stylist, expr: &ExprFString) -> Option<(String, bool)> {
    let mut rendered_parts = Vec::new();
    for part in &expr.value {
        match part {
            FStringPart::Literal(literal) => {
                let rendered = render_string_literal_part(literal)?;
                rendered_parts.push(rendered);
            }
            FStringPart::FString(fstring) => {
                let rendered = render_f_string_part(stylist, fstring)?;
                rendered_parts.push(rendered);
            }
        }
    }
    Some((rendered_parts.join(" "), false))
}

fn render_f_string_part(stylist: &Stylist, fstring: &FString) -> Option<String> {
    let flags: AnyStringFlags = fstring.flags.into();
    if flags.is_raw_string() {
        return None;
    }

    let mut result = String::new();
    result.push_str(fstring.flags.prefix().as_str());
    let quote_token = quote_token(flags.quote_style(), flags.triple_quotes());
    result.push_str(quote_token);

    let body = render_interpolated_elements(stylist, &fstring.elements, flags)?;
    result.push_str(&body);
    result.push_str(quote_token);

    Some(result)
}

fn render_interpolated_elements(
    stylist: &Stylist,
    elements: &[InterpolatedStringElement],
    flags: AnyStringFlags,
) -> Option<String> {
    let mut result = String::new();
    for element in elements {
        match element {
            InterpolatedStringElement::Literal(literal) => {
                let rendered = render_interpolated_literal(literal, flags)?;
                result.push_str(&rendered);
            }
            InterpolatedStringElement::Interpolation(interpolation) => {
                let rendered = render_interpolation(stylist, interpolation, flags)?;
                result.push_str(&rendered);
            }
        }
    }
    Some(result)
}

fn render_interpolated_literal(
    literal: &InterpolatedStringLiteralElement,
    flags: AnyStringFlags,
) -> Option<String> {
    let mut escaped = String::with_capacity(literal.value.len());
    for ch in literal.value.chars() {
        match ch {
            '{' => {
                escaped.push('{');
                escaped.push('{');
            }
            '}' => {
                escaped.push('}');
                escaped.push('}');
            }
            other => escaped.push(other),
        }
    }

    if flags.is_raw_string() {
        return Some(escaped);
    }

    if flags.triple_quotes() == TripleQuotes::Yes {
        let mut body = String::new();
        append_triple_content(
            &escaped,
            quote_token(flags.quote_style(), flags.triple_quotes()),
            flags.quote_style(),
            &mut body,
        );
        return Some(body);
    }

    let escape = UnicodeEscape::with_preferred_quote(&escaped, flags.quote_style());
    let mut body = String::new();
    if let Some(len) = escape.layout().len {
        body.reserve(len);
    }
    escape
        .write_body(&mut body)
        .expect("writing to string should not fail");
    Some(body)
}

fn render_interpolation(
    stylist: &Stylist,
    element: &InterpolatedElement,
    flags: AnyStringFlags,
) -> Option<String> {
    let mut result = String::new();
    let expr_text = Generator::from(stylist).expr(&element.expression);

    if expr_text.starts_with('{') {
        result.push_str("{ ");
    } else {
        result.push('{');
    }

    if let Some(debug_text) = &element.debug_text {
        result.push_str(debug_text.leading.as_str());
        result.push_str(&expr_text);
        result.push_str(debug_text.trailing.as_str());
    } else {
        result.push_str(&expr_text);
    }

    if let Some(ch) = element.conversion.to_char() {
        result.push('!');
        result.push(ch);
    }

    if let Some(spec) = &element.format_spec {
        result.push(':');
        let spec_text = render_interpolated_elements(stylist, &spec.elements, flags)?;
        result.push_str(&spec_text);
    }

    result.push('}');
    Some(result)
}

fn render_triple_quoted_literal(content: &str, quote: Quote, prefix: &str) -> String {
    let mut result = String::new();
    result.push_str(prefix);
    let token = quote_token(quote, TripleQuotes::Yes);
    result.push_str(token);
    append_triple_content(content, token, quote, &mut result);
    result.push_str(token);
    result
}

fn render_non_triple_literal(content: &str, quote: Quote, prefix: &str) -> Option<String> {
    let escape = UnicodeEscape::with_preferred_quote(content, quote);
    let repr = escape.str_repr(TripleQuotes::No).to_string()?;
    let mut result = String::new();
    result.push_str(prefix);
    result.push_str(&repr);
    Some(result)
}

fn append_triple_content(content: &str, triple: &str, quote: Quote, buf: &mut String) {
    let mut index = 0;
    while index < content.len() {
        if content[index..].starts_with(triple) {
            buf.push('\\');
            buf.push_str(triple);
            index += triple.len();
            continue;
        }

        let ch = content[index..].chars().next().unwrap();
        index += ch.len_utf8();
        push_triple_char(ch, quote, buf);
    }
}

fn push_triple_char(ch: char, _quote: Quote, buf: &mut String) {
    match ch {
        '\n' => buf.push('\n'),
        '\t' => buf.push_str("\\t"),
        '\r' => buf.push_str("\\r"),
        '\\' => buf.push_str("\\\\"),
        other if should_escape(other) => push_unicode_escape(other, buf),
        other => buf.push(other),
    }
}

fn should_escape(ch: char) -> bool {
    matches!(ch, '\0'..='\x08' | '\x0b' | '\x0c' | '\x0e'..='\x1f' | '\x7f')
        || (!ch.is_ascii() && !is_printable(ch))
}

fn push_unicode_escape(ch: char, buf: &mut String) {
    let code = ch as u32;
    if code < 0x100 {
        let _ = write!(buf, "\\x{code:02x}");
    } else if code < 0x10000 {
        let _ = write!(buf, "\\u{code:04x}");
    } else {
        let _ = write!(buf, "\\U{code:08x}");
    }
}

fn quote_token(quote: Quote, triple: TripleQuotes) -> &'static str {
    match (quote, triple) {
        (Quote::Single, TripleQuotes::Yes) => "'''",
        (Quote::Double, TripleQuotes::Yes) => "\"\"\"",
        (Quote::Single, TripleQuotes::No) => "'",
        (Quote::Double, TripleQuotes::No) => "\"",
    }
}

// Expression rewriting module - extracted from the main import transformer
use ruff_python_ast::{
    AtomicNodeIndex, Expr, ExprAttribute, ExprContext, ExprFString, FString, FStringValue,
    InterpolatedElement, InterpolatedStringElement, InterpolatedStringElements,
};

use crate::{
    ast_builder::{CRIBO_PREFIX, expressions},
    code_generator::{
        import_transformer::{
            RecursiveImportTransformer, handlers::dynamic::DynamicHandler, state::TransformerState,
        },
        module_transformer::SELF_PARAM,
    },
};

/// Expression rewriting logic for the import transformer
pub(super) struct ExpressionRewriter;

impl ExpressionRewriter {
    /// Recursively transform expressions within the import transformer context
    pub(super) fn transform_expr(
        transformer: &mut RecursiveImportTransformer<'_>,
        expr: &mut Expr,
    ) {
        // First check if this is an attribute expression and collect the path
        let attribute_info = if matches!(expr, Expr::Attribute(_)) {
            let info = Self::collect_attribute_path(expr);
            log::debug!(
                "transform_expr: Found attribute expression - base: {:?}, path: {:?}, \
                 is_entry_module: {}",
                info.0,
                info.1,
                transformer.state.is_entry_module()
            );

            Some(info)
        } else {
            None
        };

        match expr {
            Expr::Attribute(attr_expr) => {
                // Special case: inside a wrapper module's init function, rewrite references
                // to the current module accessed via the parent namespace (e.g.,
                // rich.console.X inside rich.console __init__) to use `self` directly.
                // Do this before any other handling to avoid re-entrancy issues.
                if transformer.state.is_wrapper_init {
                    let current = transformer.state.get_module_name();
                    let segments: Vec<&str> = current.split('.').collect();
                    if segments.len() >= 2 {
                        let root = segments[0];
                        let rel_first = segments[1];
                        // Try to find the inner attribute node where value is Name(root)
                        // and attribute equals the first relative segment (rel_first).
                        let mut cursor: &mut ExprAttribute = attr_expr; // start at outer attribute
                        while let Expr::Attribute(inner) = cursor.value.as_mut() {
                            let is_base = matches!(
                                inner.value.as_ref(),
                                Expr::Name(n) if n.id.as_str() == root
                            ) && inner.attr.as_str() == rel_first;
                            if is_base {
                                inner.value =
                                    Box::new(expressions::name(SELF_PARAM, ExprContext::Load));
                                return;
                            }
                            cursor = inner;
                        }
                    }
                }

                // First check if the base of this attribute is a wrapper module import
                if let Expr::Name(base_name) = &*attr_expr.value {
                    let name = base_name.id.as_str();

                    // Check if this is a stdlib module reference (e.g., collections.abc)
                    if crate::resolver::is_stdlib_module(name, transformer.state.python_version) {
                        // Check if this stdlib name is shadowed by local variables or imports
                        // In wrapper modules, we only track local_variables which includes imported
                        // names
                        let is_shadowed = transformer.state.local_variables.contains(name)
                            || transformer.state.import_aliases.contains_key(name);

                        if !is_shadowed {
                            if !matches!(attr_expr.ctx, ExprContext::Load) {
                                return;
                            }
                            // Transform stdlib module attribute access to use _cribo proxy
                            // e.g., collections.abc -> _cribo.collections.abc
                            log::debug!(
                                "Transforming stdlib attribute access: {}.{} -> _cribo.{}.{}",
                                name,
                                attr_expr.attr.as_str(),
                                name,
                                attr_expr.attr.as_str()
                            );

                            // Create _cribo.module.attr
                            let attr_name = attr_expr.attr.to_string();
                            let attr_ctx = attr_expr.ctx;
                            let attr_range = attr_expr.range;

                            // Create _cribo.name.attr_name
                            let base =
                                expressions::name_attribute(CRIBO_PREFIX, name, ExprContext::Load);
                            let mut new_expr = expressions::attribute(base, &attr_name, attr_ctx);
                            // Preserve the original range
                            if let Expr::Attribute(attr) = &mut new_expr {
                                attr.range = attr_range;
                            }
                            *expr = new_expr;
                            return;
                        }
                    }

                    if let Some((wrapper_module, imported_name)) =
                        transformer.state.wrapper_module_imports.get(name)
                    {
                        if !matches!(attr_expr.ctx, ExprContext::Load) {
                            return;
                        }
                        // The base is a wrapper module import, rewrite the entire attribute access
                        // e.g., cookielib.CookieJar -> myrequests.compat.cookielib.CookieJar
                        log::debug!(
                            "Rewriting attribute '{}.{}' to '{}.{}.{}'",
                            name,
                            attr_expr.attr.as_str(),
                            wrapper_module,
                            imported_name,
                            attr_expr.attr.as_str()
                        );

                        // Create wrapper_module.imported_name.attr
                        let base = expressions::name_attribute(
                            wrapper_module,
                            imported_name,
                            ExprContext::Load,
                        );
                        let mut new_expr =
                            expressions::attribute(base, attr_expr.attr.as_str(), attr_expr.ctx);
                        // Preserve the original range
                        if let Expr::Attribute(attr) = &mut new_expr {
                            attr.range = attr_expr.range;
                        }
                        *expr = new_expr;
                        return; // Don't process further
                    }
                }

                // Handle nested attribute access using the pre-collected path
                if let Some((base_name, attr_path)) = attribute_info {
                    if let Some(base) = base_name {
                        // In the entry module, check if this is accessing a namespace object
                        // created by a dotted import
                        if transformer.state.is_entry_module() && attr_path.len() >= 2 {
                            // For "greetings.greeting.get_greeting()", we have:
                            // base: "greetings", attr_path: ["greeting", "get_greeting"]
                            // Check if "greetings.greeting" is a bundled module (created by "import
                            // greetings.greeting")
                            let namespace_path = format!("{}.{}", base, attr_path[0]);

                            if transformer
                                .state
                                .bundler
                                .get_module_id(&namespace_path)
                                .is_some_and(|id| {
                                    transformer.state.bundler.bundled_modules.contains(&id)
                                })
                            {
                                // This is accessing a method/attribute on a namespace object
                                // created by a dotted import
                                // Don't transform it - let the namespace object handle it
                                log::debug!(
                                    "Not transforming {base}.{} - accessing namespace object \
                                     created by dotted import",
                                    attr_path.join(".")
                                );
                                // Don't recursively transform - the whole expression should remain
                                // as-is
                                return;
                            }
                        }

                        // First check if the base is a variable assigned from
                        // importlib.import_module()
                        if let Some(module_name) =
                            transformer.state.importlib_inlined_modules.get(&base)
                        {
                            // This is accessing attributes on a variable that was assigned from
                            // importlib.import_module() of an inlined module
                            if attr_path.len() == 1 && matches!(attr_expr.ctx, ExprContext::Load) {
                                let attr_name = &attr_path[0];
                                log::debug!(
                                    "Transforming {base}.{attr_name} - {base} was assigned from \
                                     importlib.import_module('{module_name}') [inlined module]"
                                );
                                *expr = DynamicHandler::rewrite_attr_for_importlib_var(
                                    attr_expr,
                                    &base,
                                    module_name,
                                    transformer.state.bundler,
                                    transformer.state.symbol_renames,
                                );
                                return;
                            }
                        }
                        // Check if the base is a stdlib import alias (e.g., j for json)
                        else if let Some(stdlib_path) =
                            transformer.state.import_aliases.get(&base)
                        {
                            // Check if this name is shadowed by a local variable
                            let is_shadowed = transformer.state.local_variables.contains(&base);
                            log::debug!(
                                "Semantic check for attribute base '{}': shadowed={}, \
                                 at_module_level={}, local_vars={:?}",
                                base,
                                is_shadowed,
                                transformer.state.at_module_level,
                                transformer.state.local_variables
                            );

                            if is_shadowed {
                                log::debug!(
                                    "Skipping stdlib rewrite for '{base}' - shadowed by local \
                                     variable"
                                );
                                return; // Don't transform shadowed variables
                            }

                            if !matches!(attr_expr.ctx, ExprContext::Load) {
                                return;
                            }

                            // This is accessing an attribute on a stdlib module alias
                            // Transform j.dumps to _cribo.json.dumps
                            if attr_path.len() == 1 {
                                let attr_name = &attr_path[0];
                                log::debug!(
                                    "Transforming {base}.{attr_name} to {stdlib_path}.{attr_name} \
                                     (stdlib import alias)"
                                );

                                // Create dotted name expression like _cribo.json.dumps
                                let full_path = format!("{stdlib_path}.{attr_name}");
                                let parts: Vec<&str> = full_path.split('.').collect();
                                let mut new_expr = expressions::dotted_name(&parts, attr_expr.ctx);
                                // Preserve the original range
                                if let Expr::Attribute(attr) = &mut new_expr {
                                    attr.range = attr_expr.range;
                                }
                                *expr = new_expr;
                                return;
                            }
                            // For deeper paths like j.decoder.JSONDecoder, build the full path
                            let mut full_path = stdlib_path.clone();
                            for part in &attr_path {
                                full_path.push('.');
                                full_path.push_str(part);
                            }
                            log::debug!(
                                "Transforming {base}.{} to {full_path} (stdlib import alias, deep \
                                 path)",
                                attr_path.join(".")
                            );

                            let parts: Vec<&str> = full_path.split('.').collect();
                            let mut new_expr = expressions::dotted_name(&parts, attr_expr.ctx);
                            // Preserve the original range
                            if let Expr::Attribute(attr) = &mut new_expr {
                                attr.range = attr_expr.range;
                            }
                            *expr = new_expr;
                            return;
                        }
                        // Check if the base refers to an inlined module
                        else if let Some(actual_module) =
                            Self::find_module_for_alias(&base, &transformer.state)
                            && transformer
                                .state
                                .bundler
                                .get_module_id(&actual_module)
                                .is_some_and(|id| {
                                    transformer.state.bundler.inlined_modules.contains(&id)
                                })
                        {
                            log::debug!(
                                "Found module alias: {base} -> {actual_module} (is_entry_module: \
                                 {})",
                                transformer.state.is_entry_module()
                            );

                            // For a single attribute access (e.g., greetings.message or
                            // config.DEFAULT_NAME)
                            if attr_path.len() == 1 {
                                let attr_name = &attr_path[0];
                                if let Some(new_expr) = transformer
                                    .try_rewrite_single_attr_for_inlined_module_alias(
                                        &base,
                                        &actual_module,
                                        attr_name,
                                        attr_expr.ctx,
                                        attr_expr.range,
                                    )
                                {
                                    *expr = new_expr;
                                    return;
                                }
                            }
                            // For nested attribute access (e.g., greetings.greeting.message)
                            // We need to handle the case where greetings.greeting is a submodule
                            else if attr_path.len() > 1
                                && let Some(new_name) = transformer
                                    .maybe_rewrite_attr_for_inlined_submodule(
                                        &base,
                                        &actual_module,
                                        &attr_path,
                                        attr_expr.ctx,
                                        attr_expr.range,
                                    )
                            {
                                *expr = new_name;
                                return;
                            }
                        }
                    }

                    // If we didn't handle it above, recursively transform the value
                    Self::transform_expr(transformer, &mut attr_expr.value);
                } // Close the if let Some((base_name, attr_path)) = attribute_info
            }
            Expr::Call(call_expr) => {
                // Check if this is importlib.import_module() with a static string literal
                if DynamicHandler::is_importlib_import_module_call(
                    call_expr,
                    &transformer.state.import_aliases,
                ) {
                    // Extract the state values we need to avoid borrow checker conflicts
                    let mut created_namespace_objects =
                        std::mem::take(&mut transformer.state.created_namespace_objects);

                    if let Some(transformed) = DynamicHandler::transform_importlib_import_module(
                        call_expr,
                        transformer.state.bundler,
                        &mut created_namespace_objects,
                        |resolved_name| transformer.create_module_access_expr(resolved_name),
                    ) {
                        // Update the original fields
                        transformer.state.created_namespace_objects = created_namespace_objects;
                        *expr = transformed;
                        return;
                    }

                    // Update the original fields even if no transformation occurred
                    transformer.state.created_namespace_objects = created_namespace_objects;
                }

                Self::transform_expr(transformer, &mut call_expr.func);
                for arg in &mut call_expr.arguments.args {
                    Self::transform_expr(transformer, arg);
                }
                for keyword in &mut call_expr.arguments.keywords {
                    Self::transform_expr(transformer, &mut keyword.value);
                }
            }
            Expr::BinOp(binop_expr) => {
                Self::transform_expr(transformer, &mut binop_expr.left);
                Self::transform_expr(transformer, &mut binop_expr.right);
            }
            Expr::UnaryOp(unaryop_expr) => {
                Self::transform_expr(transformer, &mut unaryop_expr.operand);
            }
            Expr::BoolOp(boolop_expr) => {
                for value in &mut boolop_expr.values {
                    Self::transform_expr(transformer, value);
                }
            }
            Expr::Compare(compare_expr) => {
                Self::transform_expr(transformer, &mut compare_expr.left);
                for comparator in &mut compare_expr.comparators {
                    Self::transform_expr(transformer, comparator);
                }
            }
            Expr::If(if_expr) => {
                Self::transform_expr(transformer, &mut if_expr.test);
                Self::transform_expr(transformer, &mut if_expr.body);
                Self::transform_expr(transformer, &mut if_expr.orelse);
            }
            Expr::List(list_expr) => {
                for elem in &mut list_expr.elts {
                    Self::transform_expr(transformer, elem);
                }
            }
            Expr::Tuple(tuple_expr) => {
                for elem in &mut tuple_expr.elts {
                    Self::transform_expr(transformer, elem);
                }
            }
            Expr::Dict(dict_expr) => {
                for item in &mut dict_expr.items {
                    if let Some(key) = &mut item.key {
                        Self::transform_expr(transformer, key);
                    }
                    Self::transform_expr(transformer, &mut item.value);
                }
            }
            Expr::Set(set_expr) => {
                for elem in &mut set_expr.elts {
                    Self::transform_expr(transformer, elem);
                }
            }
            Expr::ListComp(listcomp_expr) => {
                Self::transform_expr(transformer, &mut listcomp_expr.elt);
                for generator in &mut listcomp_expr.generators {
                    Self::transform_expr(transformer, &mut generator.iter);
                    for if_clause in &mut generator.ifs {
                        Self::transform_expr(transformer, if_clause);
                    }
                }
            }
            Expr::DictComp(dictcomp_expr) => {
                Self::transform_expr(transformer, &mut dictcomp_expr.key);
                Self::transform_expr(transformer, &mut dictcomp_expr.value);
                for generator in &mut dictcomp_expr.generators {
                    Self::transform_expr(transformer, &mut generator.iter);
                    for if_clause in &mut generator.ifs {
                        Self::transform_expr(transformer, if_clause);
                    }
                }
            }
            Expr::SetComp(setcomp_expr) => {
                Self::transform_expr(transformer, &mut setcomp_expr.elt);
                for generator in &mut setcomp_expr.generators {
                    Self::transform_expr(transformer, &mut generator.iter);
                    for if_clause in &mut generator.ifs {
                        Self::transform_expr(transformer, if_clause);
                    }
                }
            }
            Expr::Generator(genexp_expr) => {
                Self::transform_expr(transformer, &mut genexp_expr.elt);
                for generator in &mut genexp_expr.generators {
                    Self::transform_expr(transformer, &mut generator.iter);
                    for if_clause in &mut generator.ifs {
                        Self::transform_expr(transformer, if_clause);
                    }
                }
            }
            Expr::Subscript(subscript_expr) => {
                Self::transform_expr(transformer, &mut subscript_expr.value);
                Self::transform_expr(transformer, &mut subscript_expr.slice);
            }
            Expr::Slice(slice_expr) => {
                if let Some(lower) = &mut slice_expr.lower {
                    Self::transform_expr(transformer, lower);
                }
                if let Some(upper) = &mut slice_expr.upper {
                    Self::transform_expr(transformer, upper);
                }
                if let Some(step) = &mut slice_expr.step {
                    Self::transform_expr(transformer, step);
                }
            }
            Expr::Lambda(lambda_expr) => {
                Self::transform_expr(transformer, &mut lambda_expr.body);
            }
            Expr::Yield(yield_expr) => {
                if let Some(value) = &mut yield_expr.value {
                    Self::transform_expr(transformer, value);
                }
            }
            Expr::YieldFrom(yieldfrom_expr) => {
                Self::transform_expr(transformer, &mut yieldfrom_expr.value);
            }
            Expr::Await(await_expr) => {
                Self::transform_expr(transformer, &mut await_expr.value);
            }
            Expr::Starred(starred_expr) => {
                Self::transform_expr(transformer, &mut starred_expr.value);
            }
            Expr::FString(fstring_expr) => {
                // Transform expressions within the f-string
                let fstring_range = fstring_expr.range;
                // Preserve the original flags from the f-string
                let original_flags = expressions::get_fstring_flags(&fstring_expr.value);
                let mut transformed_elements = Vec::new();
                let mut any_transformed = false;

                for element in fstring_expr.value.elements() {
                    match element {
                        InterpolatedStringElement::Literal(lit_elem) => {
                            transformed_elements
                                .push(InterpolatedStringElement::Literal(lit_elem.clone()));
                        }
                        InterpolatedStringElement::Interpolation(expr_elem) => {
                            let mut new_expr = expr_elem.expression.clone();
                            Self::transform_expr(transformer, &mut new_expr);

                            if !matches!(&new_expr, other if other == &expr_elem.expression) {
                                any_transformed = true;
                            }

                            let new_element = InterpolatedElement {
                                node_index: AtomicNodeIndex::NONE,
                                expression: new_expr,
                                debug_text: expr_elem.debug_text.clone(),
                                conversion: expr_elem.conversion,
                                format_spec: expr_elem.format_spec.clone(),
                                range: expr_elem.range,
                            };
                            transformed_elements
                                .push(InterpolatedStringElement::Interpolation(new_element));
                        }
                    }
                }

                if any_transformed {
                    let new_fstring = FString {
                        node_index: AtomicNodeIndex::NONE,
                        elements: InterpolatedStringElements::from(transformed_elements),
                        range: fstring_range,
                        flags: original_flags, // Preserve the original flags including quote style
                    };

                    let new_value = FStringValue::single(new_fstring);

                    *expr = Expr::FString(ExprFString {
                        node_index: AtomicNodeIndex::NONE,
                        value: new_value,
                        range: fstring_range,
                    });
                }
            }
            // Check if Name expressions need to be rewritten for wrapper module imports or stdlib
            // imports
            Expr::Name(name_expr) => {
                let name = name_expr.id.as_str();
                // Only rewrite reads; never rewrite assignment/delete targets.
                if !matches!(name_expr.ctx, ExprContext::Load) {
                    return;
                }

                // Check if this name is a stdlib import alias that needs rewriting
                // Only rewrite if it's not shadowed by a local variable
                if let Some(rewritten_path) = transformer.state.import_aliases.get(name) {
                    log::debug!("Found import alias for '{name}' -> '{rewritten_path}'");
                    // Check if this is a stdlib module reference (starts with _cribo.)
                    if rewritten_path.starts_with(CRIBO_PREFIX)
                        && rewritten_path.chars().nth(CRIBO_PREFIX.len()) == Some('.')
                    {
                        // Check if this name is shadowed by a local variable
                        // The transformer state tracks local variables to avoid treating them as
                        // module aliases
                        let is_shadowed = transformer.state.local_variables.contains(name);
                        log::debug!(
                            "Semantic check for '{}': shadowed={}, at_module_level={}, \
                             local_vars={:?}",
                            name,
                            is_shadowed,
                            transformer.state.at_module_level,
                            transformer.state.local_variables
                        );

                        if !is_shadowed {
                            log::debug!(
                                "Rewriting stdlib reference '{name}' to '{rewritten_path}'"
                            );

                            // Parse the rewritten path to create attribute access
                            // e.g., "_cribo.json" becomes _cribo.json
                            let parts: Vec<&str> = rewritten_path.split('.').collect();
                            if parts.len() >= 2 {
                                *expr = expressions::dotted_name(&parts, name_expr.ctx);
                                return;
                            }
                        }
                    }
                }

                // Check if this name was imported from a wrapper module and needs rewriting
                if let Some((wrapper_module, imported_name)) =
                    transformer.state.wrapper_module_imports.get(name)
                {
                    log::debug!("Rewriting name '{name}' to '{wrapper_module}.{imported_name}'");

                    // Create wrapper_module.imported_name attribute access
                    // Create wrapper_module.imported_name attribute access
                    let mut new_expr =
                        expressions::name_attribute(wrapper_module, imported_name, name_expr.ctx);
                    // Preserve the original range
                    if let Expr::Attribute(attr) = &mut new_expr {
                        attr.range = name_expr.range;
                    }
                    *expr = new_expr;
                }
            }
            // Constants, etc. don't need transformation
            _ => {}
        }
    }

    /// Collect the full dotted attribute path from a potentially nested attribute expression
    /// Returns (`base_name`, [attr1, attr2, ...])
    /// For example: `greetings.greeting.message` returns `(Some("greetings"), ["greeting",
    /// "message"])`
    fn collect_attribute_path(expr: &Expr) -> (Option<String>, Vec<String>) {
        let mut attrs = Vec::new();
        let mut current = expr;

        loop {
            match current {
                Expr::Attribute(attr) => {
                    attrs.push(attr.attr.as_str().to_owned());
                    current = &attr.value;
                }
                Expr::Name(name) => {
                    attrs.reverse();
                    return (Some(name.id.as_str().to_owned()), attrs);
                }
                _ => {
                    attrs.reverse();
                    return (None, attrs);
                }
            }
        }
    }

    /// Find the actual module name for a given alias
    fn find_module_for_alias(alias: &str, state: &TransformerState<'_>) -> Option<String> {
        log::debug!(
            "find_module_for_alias: alias={}, is_entry_module={}, local_vars={:?}",
            alias,
            state.is_entry_module(),
            state.local_variables.contains(alias)
        );

        // Don't treat local variables as module aliases
        if state.local_variables.contains(alias) {
            return None;
        }

        // First check our tracked import aliases
        if let Some(module_name) = state.import_aliases.get(alias) {
            return Some(module_name.clone());
        }

        // Then check if the alias directly matches a module name
        // But not in the entry module - in the entry module, direct module names
        // are namespace objects, not aliases
        if !state.is_entry_module()
            && state
                .bundler
                .get_module_id(alias)
                .is_some_and(|id| state.bundler.inlined_modules.contains(&id))
        {
            Some(alias.to_owned())
        } else {
            None
        }
    }
}

use ruff_python_ast::{Expr, ModModule, Stmt};

use crate::types::{FxIndexMap, FxIndexSet};

/// Handles symbol-level circular dependency analysis and resolution
#[derive(Debug, Default, Clone)]
pub(crate) struct SymbolDependencyGraph {
    /// Track which symbols are defined in which modules
    pub symbol_definitions: FxIndexSet<(String, String)>,
    /// Module-level dependencies (used at definition time, not inside function bodies)
    pub module_level_dependencies: FxIndexMap<(String, String), Vec<(String, String)>>,
}

impl SymbolDependencyGraph {
    /// Populate the dependency graph from a module's AST.
    ///
    /// Extracts top-level symbol definitions and their module-level dependencies
    /// (references evaluated at definition time, NOT inside function bodies).
    pub(crate) fn populate_from_ast(&mut self, module_name: &str, ast: &ModModule) {
        // First pass: collect all top-level symbol definitions
        let mut defined_symbols = FxIndexSet::default();
        for stmt in &ast.body {
            if let Some(name) = top_level_symbol_name(stmt) {
                defined_symbols.insert(name.clone());
                self.symbol_definitions
                    .insert((module_name.to_owned(), name));
            }
        }

        // Second pass: for each symbol, collect module-level name references
        // and intersect with defined_symbols to get intra-module dependencies
        for stmt in &ast.body {
            let Some(symbol_name) = top_level_symbol_name(stmt) else {
                continue;
            };

            let mut refs = FxIndexSet::default();
            collect_module_level_refs(stmt, &mut refs);

            let deps: Vec<(String, String)> = refs
                .into_iter()
                .filter(|r| r != &symbol_name && defined_symbols.contains(r))
                .map(|r| (module_name.to_owned(), r))
                .collect();

            if !deps.is_empty() {
                let key = (module_name.to_owned(), symbol_name);
                self.module_level_dependencies
                    .entry(key)
                    .or_default()
                    .extend(deps);
            }
        }
    }

    /// Find all symbols in the strongly connected component containing the given node
    /// Uses petgraph SCC detection for robust cycle detection
    fn find_cycle_symbols_with_scc(
        graph: &petgraph::Graph<String, ()>,
        cycle_node: petgraph::graph::NodeIndex,
    ) -> Vec<String> {
        Self::find_cycle_symbols_generic(graph, cycle_node)
    }

    /// Locate the SCC for a node using petgraph's SCC utilities
    /// Works with any graph node type that implements Clone
    fn find_cycle_symbols_generic<T>(
        graph: &petgraph::Graph<T, ()>,
        cycle_node: petgraph::graph::NodeIndex,
    ) -> Vec<T>
    where
        T: Clone,
    {
        use petgraph::algo::tarjan_scc;

        let components = tarjan_scc(graph);

        // Include self-loops (single-node SCC with self-edge) as cycles
        if let Some(component) = components.into_iter().find(|c| {
            c.contains(&cycle_node) && (c.len() > 1 || graph.contains_edge(cycle_node, cycle_node))
        }) {
            return component
                .into_iter()
                .map(|idx| graph[idx].clone())
                .collect();
        }

        // If no SCC found containing the node (unexpected), return just that symbol
        vec![graph[cycle_node].clone()]
    }

    /// Get symbols for a specific module in dependency order
    pub(crate) fn get_module_symbols_ordered(&self, module_name: &str) -> Vec<String> {
        use petgraph::{
            algo::toposort,
            graph::{DiGraph, NodeIndex},
        };
        // Build a directed graph of symbol dependencies ONLY for this module
        let mut graph = DiGraph::new();
        let mut node_map: FxIndexMap<String, NodeIndex> = FxIndexMap::default();

        // Add nodes for all symbols in this specific module
        for (module, symbol) in &self.symbol_definitions {
            if module == module_name {
                let node = graph.add_node(symbol.clone());
                node_map.insert(symbol.clone(), node);
            }
        }

        // Add edges for dependencies within this module (flattened with early continues)
        for ((module, symbol), deps) in &self.module_level_dependencies {
            if module != module_name {
                continue;
            }
            let Some(&from_node) = node_map.get(symbol) else {
                continue;
            };
            for (dep_module, dep_symbol) in deps {
                if dep_module != module_name {
                    continue;
                }
                let Some(&to_node) = node_map.get(dep_symbol) else {
                    continue;
                };
                // Edge from dependency to dependent
                graph.add_edge(to_node, from_node, ());
            }
        }

        // Perform topological sort
        match toposort(&graph, None) {
            Ok(sorted_nodes) => {
                // Return symbols in topological order (dependencies first)
                sorted_nodes
                    .into_iter()
                    .map(|node_idx| graph[node_idx].clone())
                    .collect()
            }
            Err(cycle) => {
                // If topological sort fails, there's a symbol-level circular dependency
                // This is a fatal error - we cannot generate correct code
                let cycle_info = cycle.node_id();
                let symbol = &graph[cycle_info];
                log::error!(
                    "Fatal: Circular dependency detected in module '{module_name}' involving \
                     symbol '{symbol}'"
                );

                // Find all symbols involved in the cycle using SCC detection
                let cycle_symbols = Self::find_cycle_symbols_with_scc(&graph, cycle_info);

                panic!(
                    "Cannot bundle due to circular symbol dependency in module '{module_name}': \
                     {cycle_symbols:?}"
                );
            }
        }
    }
}

/// Extract the defined symbol name from a top-level statement, if any.
fn top_level_symbol_name(stmt: &Stmt) -> Option<String> {
    match stmt {
        Stmt::FunctionDef(f) => Some(f.name.to_string()),
        Stmt::ClassDef(c) => Some(c.name.to_string()),
        Stmt::Assign(a) => {
            // Only simple `name = expr` assignments define a single symbol
            if a.targets.len() == 1 {
                if let Expr::Name(name) = &a.targets[0] {
                    return Some(name.id.to_string());
                }
            }
            None
        }
        _ => None,
    }
}

/// Collect name references from module-level expressions of a statement.
///
/// Only collects references evaluated at definition time:
/// - Function: decorators, default parameter values (NOT the body)
/// - Class: base classes, decorators, keyword arguments, and body (class body executes at
///   definition time, but nested function bodies do not)
/// - Assignment: the right-hand side expression
fn collect_module_level_refs(stmt: &Stmt, refs: &mut FxIndexSet<String>) {
    match stmt {
        Stmt::FunctionDef(f) => {
            // Decorators are evaluated at definition time
            for dec in &f.decorator_list {
                collect_names_from_expr(&dec.expression, refs);
            }
            // Default parameter values are evaluated at definition time
            for param in f
                .parameters
                .args
                .iter()
                .chain(&f.parameters.posonlyargs)
                .chain(&f.parameters.kwonlyargs)
            {
                if let Some(default) = &param.default {
                    collect_names_from_expr(default, refs);
                }
            }
            if let Some(default) = &f.parameters.vararg {
                if let Some(ann) = &default.annotation {
                    collect_names_from_expr(ann, refs);
                }
            }
            if let Some(default) = &f.parameters.kwarg {
                if let Some(ann) = &default.annotation {
                    collect_names_from_expr(ann, refs);
                }
            }
            // Return annotation
            if let Some(ann) = &f.returns {
                collect_names_from_expr(ann, refs);
            }
            // NOTE: function body is NOT module-level — references there are
            // resolved at call time, not definition time.
        }
        Stmt::ClassDef(c) => {
            // Decorators
            for dec in &c.decorator_list {
                collect_names_from_expr(&dec.expression, refs);
            }
            // Base classes are evaluated at definition time
            if let Some(args) = &c.arguments {
                for base in &args.args {
                    collect_names_from_expr(base, refs);
                }
                // Keyword arguments (e.g., metaclass=ABCMeta)
                for kw in &args.keywords {
                    collect_names_from_expr(&kw.value, refs);
                }
            }
            // Class body executes at definition time, but skip nested function bodies
            for body_stmt in &c.body {
                match body_stmt {
                    Stmt::FunctionDef(_) => {
                        // Method bodies are deferred — skip
                    }
                    Stmt::Assign(a) => {
                        collect_names_from_expr(&a.value, refs);
                    }
                    Stmt::Expr(e) => {
                        collect_names_from_expr(&e.value, refs);
                    }
                    _ => {}
                }
            }
        }
        Stmt::Assign(a) => {
            collect_names_from_expr(&a.value, refs);
        }
        Stmt::AugAssign(a) => {
            collect_names_from_expr(&a.value, refs);
        }
        _ => {}
    }
}

/// Recursively collect all top-level `Expr::Name` references from an expression.
fn collect_names_from_expr(expr: &Expr, refs: &mut FxIndexSet<String>) {
    match expr {
        Expr::Name(name) => {
            refs.insert(name.id.to_string());
        }
        Expr::Attribute(attr) => {
            // Only collect the root name: `foo.bar` → "foo"
            collect_names_from_expr(&attr.value, refs);
        }
        Expr::Call(call) => {
            collect_names_from_expr(&call.func, refs);
            for arg in &call.arguments.args {
                collect_names_from_expr(arg, refs);
            }
            for kw in &call.arguments.keywords {
                collect_names_from_expr(&kw.value, refs);
            }
        }
        Expr::Subscript(sub) => {
            collect_names_from_expr(&sub.value, refs);
            collect_names_from_expr(&sub.slice, refs);
        }
        Expr::BinOp(op) => {
            collect_names_from_expr(&op.left, refs);
            collect_names_from_expr(&op.right, refs);
        }
        Expr::UnaryOp(op) => {
            collect_names_from_expr(&op.operand, refs);
        }
        Expr::BoolOp(op) => {
            for val in &op.values {
                collect_names_from_expr(val, refs);
            }
        }
        Expr::Compare(cmp) => {
            collect_names_from_expr(&cmp.left, refs);
            for val in &cmp.comparators {
                collect_names_from_expr(val, refs);
            }
        }
        Expr::If(if_expr) => {
            collect_names_from_expr(&if_expr.test, refs);
            collect_names_from_expr(&if_expr.body, refs);
            collect_names_from_expr(&if_expr.orelse, refs);
        }
        Expr::List(list) => {
            for elt in &list.elts {
                collect_names_from_expr(elt, refs);
            }
        }
        Expr::Tuple(tuple) => {
            for elt in &tuple.elts {
                collect_names_from_expr(elt, refs);
            }
        }
        Expr::Set(set) => {
            for elt in &set.elts {
                collect_names_from_expr(elt, refs);
            }
        }
        Expr::Dict(dict) => {
            for item in &dict.items {
                if let Some(key) = &item.key {
                    collect_names_from_expr(key, refs);
                }
                collect_names_from_expr(&item.value, refs);
            }
        }
        Expr::Starred(starred) => {
            collect_names_from_expr(&starred.value, refs);
        }
        Expr::Await(await_expr) => {
            collect_names_from_expr(&await_expr.value, refs);
        }
        Expr::Lambda(lambda) => {
            // Lambda body is evaluated lazily, but defaults are not
            for param in lambda
                .parameters
                .as_ref()
                .map_or([].as_slice(), |p| p.args.as_slice())
            {
                if let Some(default) = &param.default {
                    collect_names_from_expr(default, refs);
                }
            }
        }
        _ => {
            // Literals, f-strings, etc. — no name references
        }
    }
}

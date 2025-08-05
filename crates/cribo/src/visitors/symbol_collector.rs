//! Symbol collection visitor for AST traversal
//!
//! This visitor collects all symbol definitions in a module including:
//! - Functions, classes, and variables
//! - Import statements and aliases
//! - Global declarations
//! - Export information (__all__)

use ruff_python_ast::{
    Expr, ModModule, Stmt, StmtAnnAssign, StmtAssign, StmtClassDef, StmtFunctionDef, StmtGlobal,
    StmtImport, StmtImportFrom,
    visitor::{Visitor, walk_stmt},
};
use ruff_text_size::{Ranged, TextRange};

use super::utils::extract_string_list_from_expr;
use crate::{
    analyzers::types::{CollectedSymbols, ScopePath, SymbolInfo, SymbolKind},
    types::FxIndexSet,
};

/// Visitor that collects symbol definitions and their metadata
pub struct SymbolCollector {
    /// Current scope stack
    scope_stack: ScopePath,
    /// Collected symbol information
    collected_symbols: CollectedSymbols,
    /// Track if we're at module level
    at_module_level: bool,
    /// Track global declarations in current scope
    current_globals: FxIndexSet<String>,
    /// Module-level __all__ exports
    module_exports: Option<Vec<String>>,
}

impl Default for SymbolCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolCollector {
    /// Create a new symbol collector
    pub fn new() -> Self {
        Self {
            scope_stack: Vec::new(),
            collected_symbols: CollectedSymbols::default(),
            at_module_level: true,
            current_globals: FxIndexSet::default(),
            module_exports: None,
        }
    }

    /// Run the collector on a module and return collected symbols
    pub fn analyze(module: &ModModule) -> CollectedSymbols {
        let mut collector = Self::new();
        collector.visit_body(&module.body);
        collector.collected_symbols
    }

    /// Enter a new scope
    fn enter_scope(&mut self, name: &str) {
        self.scope_stack.push(name.to_string());
        self.at_module_level = false;
    }

    /// Exit the current scope
    fn exit_scope(&mut self) {
        self.scope_stack.pop();
        self.at_module_level = self.scope_stack.is_empty();
    }

    /// Get the current scope path
    fn current_scope(&self) -> &ScopePath {
        &self.scope_stack
    }

    /// Check if a name is uppercase (likely a constant)
    fn is_constant_name(name: &str) -> bool {
        name.chars()
            .all(|c| c.is_uppercase() || c == '_' || c.is_numeric())
            && name.chars().any(char::is_alphabetic)
    }

    /// Add a symbol to the appropriate collection
    fn add_symbol(&mut self, symbol: SymbolInfo) {
        if self.at_module_level {
            self.collected_symbols
                .global_symbols
                .insert(symbol.name.clone(), symbol.clone());
        }

        self.collected_symbols
            .scoped_symbols
            .entry(self.current_scope().clone())
            .or_default()
            .push(symbol);
    }

    /// Process a function definition
    fn process_function(&mut self, func: &StmtFunctionDef) {
        let decorators: Vec<String> = func
            .decorator_list
            .iter()
            .filter_map(|decorator| match &decorator.expression {
                Expr::Name(name) => Some(name.id.to_string()),
                Expr::Attribute(attr) => Some(format!(
                    "{}.{}",
                    Self::expr_to_string(&attr.value),
                    attr.attr
                )),
                _ => None,
            })
            .collect();

        let symbol = SymbolInfo {
            name: func.name.to_string(),
            kind: SymbolKind::Function { decorators },
            scope: self.current_scope().clone(),
            is_exported: self.is_exported(func.name.as_ref()),
            is_global: self.at_module_level || self.current_globals.contains(func.name.as_str()),
            definition_range: func.range,
        };

        self.add_symbol(symbol);
    }

    /// Process a class definition
    fn process_class(&mut self, class: &StmtClassDef) {
        let bases: Vec<String> = if let Some(arguments) = &class.arguments {
            arguments.args.iter().map(Self::expr_to_string).collect()
        } else {
            Vec::new()
        };

        let symbol = SymbolInfo {
            name: class.name.to_string(),
            kind: SymbolKind::Class { bases },
            scope: self.current_scope().clone(),
            is_exported: self.is_exported(class.name.as_ref()),
            is_global: self.at_module_level || self.current_globals.contains(class.name.as_str()),
            definition_range: class.range,
        };

        self.add_symbol(symbol);
    }

    /// Process variable assignments
    fn process_assignment(&mut self, targets: &[Expr], range: TextRange) {
        for target in targets {
            if let Expr::Name(name) = target
                && name.ctx.is_store()
            {
                let symbol = SymbolInfo {
                    name: name.id.to_string(),
                    kind: SymbolKind::Variable {
                        is_constant: Self::is_constant_name(&name.id),
                    },
                    scope: self.current_scope().clone(),
                    is_exported: self.is_exported(name.id.as_ref()),
                    is_global: self.at_module_level
                        || self.current_globals.contains(name.id.as_str()),
                    definition_range: range,
                };

                self.add_symbol(symbol);
            }
        }
    }

    /// Process import statements
    fn process_import(&mut self, import: &StmtImport, range: TextRange) {
        for alias in &import.names {
            let import_name = alias.asname.as_ref().unwrap_or(&alias.name);
            let actual_name = &alias.name;

            // Record import alias
            if alias.asname.is_some() {
                self.collected_symbols
                    .module_renames
                    .insert(import_name.to_string(), actual_name.to_string());
            }

            let symbol = SymbolInfo {
                name: import_name.to_string(),
                kind: SymbolKind::Import {
                    module: actual_name.to_string(),
                },
                scope: self.current_scope().clone(),
                is_exported: self.is_exported(import_name.as_ref()),
                is_global: self.at_module_level,
                definition_range: range,
            };

            self.add_symbol(symbol);
        }
    }

    /// Process from imports
    fn process_from_import(&mut self, import: &StmtImportFrom, range: TextRange) {
        // Handle relative imports properly
        let from_module = import.module.as_ref().map_or_else(
            || ".".repeat(import.level as usize),
            std::string::ToString::to_string,
        );

        for alias in &import.names {
            let import_name = alias.asname.as_ref().unwrap_or(&alias.name);
            let actual_name = &alias.name;

            // Record import alias
            if alias.asname.is_some() {
                self.collected_symbols
                    .module_renames
                    .insert(import_name.to_string(), actual_name.to_string());
            }

            let symbol = SymbolInfo {
                name: import_name.to_string(),
                kind: SymbolKind::Import {
                    module: from_module.clone(),
                },
                scope: self.current_scope().clone(),
                is_exported: self.is_exported(import_name.as_ref()),
                is_global: self.at_module_level,
                definition_range: range,
            };

            self.add_symbol(symbol);
        }
    }

    /// Check if a symbol is exported
    fn is_exported(&self, name: &str) -> bool {
        if let Some(ref exports) = self.module_exports {
            exports.contains(&name.to_string())
        } else {
            // If no __all__, public symbols (not starting with _) are exported
            !name.starts_with('_')
        }
    }

    /// Convert an expression to a string representation
    fn expr_to_string(expr: &Expr) -> String {
        match expr {
            Expr::Name(name) => name.id.to_string(),
            Expr::Attribute(attr) => {
                format!("{}.{}", Self::expr_to_string(&attr.value), attr.attr)
            }
            Expr::Call(call) => Self::expr_to_string(&call.func),
            _ => "<complex>".to_string(),
        }
    }

    /// Extract __all__ exports from assignment
    fn extract_all_exports(&mut self, value: &Expr) {
        let result = extract_string_list_from_expr(value);
        if let Some(exports) = result.names {
            self.module_exports = Some(exports);
        }
    }
}

impl<'a> Visitor<'a> for SymbolCollector {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(func) => {
                self.process_function(func);
                self.enter_scope(&func.name);
                walk_stmt(self, stmt);
                self.exit_scope();
            }
            Stmt::ClassDef(class) => {
                self.process_class(class);
                self.enter_scope(&class.name);
                walk_stmt(self, stmt);
                self.exit_scope();
            }
            Stmt::Assign(StmtAssign { targets, value, .. }) => {
                // Check for __all__ assignment
                if self.at_module_level
                    && targets.len() == 1
                    && matches!(&targets[0], Expr::Name(name) if name.id == "__all__")
                {
                    self.extract_all_exports(value);
                }
                self.process_assignment(targets, stmt.range());
                walk_stmt(self, stmt);
            }
            Stmt::AnnAssign(StmtAnnAssign { target, .. }) => {
                self.process_assignment(std::slice::from_ref(target), stmt.range());
                walk_stmt(self, stmt);
            }
            Stmt::Import(import) => {
                self.process_import(import, stmt.range());
                walk_stmt(self, stmt);
            }
            Stmt::ImportFrom(import) => {
                self.process_from_import(import, stmt.range());
                walk_stmt(self, stmt);
            }
            Stmt::Global(StmtGlobal { names, .. }) => {
                // Track global declarations for the current scope
                for name in names {
                    self.current_globals.insert(name.to_string());
                }
                walk_stmt(self, stmt);
            }
            _ => walk_stmt(self, stmt),
        }
    }
}

#[cfg(test)]
mod tests {
    use ruff_python_parser::parse_module;

    use super::*;

    #[test]
    fn test_symbol_collection_basic() {
        let code = r#"
def foo():
    pass

class Bar:
    pass

x = 42
CONSTANT = "value"
"#;
        let parsed = parse_module(code).expect("Failed to parse test module");
        let module = parsed.into_syntax();
        let symbols = SymbolCollector::analyze(&module);

        assert_eq!(symbols.global_symbols.len(), 4);
        assert!(symbols.global_symbols.contains_key("foo"));
        assert!(symbols.global_symbols.contains_key("Bar"));
        assert!(symbols.global_symbols.contains_key("x"));
        assert!(symbols.global_symbols.contains_key("CONSTANT"));

        // Check constant detection
        let constant = &symbols.global_symbols["CONSTANT"];
        assert!(matches!(
            constant.kind,
            SymbolKind::Variable { is_constant: true }
        ));
    }

    #[test]
    fn test_import_aliases() {
        let code = r"
import numpy as np
from typing import List as L
";
        let parsed = parse_module(code).expect("Failed to parse test module");
        let module = parsed.into_syntax();
        let symbols = SymbolCollector::analyze(&module);

        assert_eq!(symbols.module_renames.len(), 2);
        assert_eq!(symbols.module_renames.get("np"), Some(&"numpy".to_string()));
        assert_eq!(symbols.module_renames.get("L"), Some(&"List".to_string()));
    }

    #[test]
    fn test_exports() {
        let code = r#"
__all__ = ["public_func", "PublicClass"]

def public_func():
    pass

def _private_func():
    pass

class PublicClass:
    pass
"#;
        let parsed = parse_module(code).expect("Failed to parse test module");
        let module = parsed.into_syntax();
        let symbols = SymbolCollector::analyze(&module);

        let public_func = &symbols.global_symbols["public_func"];
        assert!(public_func.is_exported);

        let private_func = &symbols.global_symbols["_private_func"];
        assert!(!private_func.is_exported);
    }
}

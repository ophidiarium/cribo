//! Export collection visitor for Python modules
//!
//! This visitor identifies module exports including __all__ declarations,
//! re-exports from imports, and implicit exports.

use ruff_python_ast::{
    Expr, ModModule, Stmt, StmtImportFrom,
    visitor::{Visitor, walk_stmt},
};

use crate::analyzers::types::{ExportInfo, ReExport};

/// Visitor that collects export information from a module
pub struct ExportCollector {
    /// Collected export information
    export_info: ExportInfo,
    /// Track if we've seen dynamic __all__ modifications
    has_dynamic_all: bool,
    /// Current __all__ contents if known
    current_all: Option<Vec<String>>,
}

impl Default for ExportCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl ExportCollector {
    /// Create a new export collector
    pub fn new() -> Self {
        Self {
            export_info: ExportInfo {
                exported_names: None,
                is_dynamic: false,
                re_exports: Vec::new(),
            },
            has_dynamic_all: false,
            current_all: None,
        }
    }

    /// Analyze a module and return export information
    pub fn analyze(module: &ModModule) -> ExportInfo {
        let mut collector = Self::new();
        collector.visit_body(&module.body);

        // Set the final exported names
        if let Some(all_names) = collector.current_all {
            collector.export_info.exported_names = Some(all_names);
        }

        collector.export_info.is_dynamic = collector.has_dynamic_all;
        collector.export_info
    }

    /// Extract string list from __all__ assignment
    fn extract_all_exports(&mut self, expr: &Expr) -> Option<Vec<String>> {
        match expr {
            Expr::List(list) => {
                let mut names = Vec::new();
                for item in &list.elts {
                    if let Some(name) = Self::extract_string_from_expr(item) {
                        names.push(name);
                    } else {
                        // Non-literal in __all__, mark as dynamic
                        self.has_dynamic_all = true;
                        return None;
                    }
                }
                Some(names)
            }
            Expr::Tuple(tuple) => {
                let mut names = Vec::new();
                for item in &tuple.elts {
                    if let Some(name) = Self::extract_string_from_expr(item) {
                        names.push(name);
                    } else {
                        // Non-literal in __all__, mark as dynamic
                        self.has_dynamic_all = true;
                        return None;
                    }
                }
                Some(names)
            }
            _ => {
                // Dynamic __all__ assignment
                self.has_dynamic_all = true;
                None
            }
        }
    }

    /// Extract a string value from an expression if it's a string literal
    fn extract_string_from_expr(expr: &Expr) -> Option<String> {
        if let Expr::StringLiteral(string_lit) = expr {
            Some(string_lit.value.to_str().to_string())
        } else {
            None
        }
    }

    /// Check if this is a re-export pattern in __init__.py
    fn check_for_reexport(&mut self, import: &StmtImportFrom) {
        // Only consider relative imports in __init__.py context
        if import.level > 0 {
            let dots = ".".repeat(import.level as usize);
            let from_module = if let Some(ref module) = import.module {
                format!("{}{}", dots, module.as_str())
            } else {
                dots
            };

            // Check if this is a star import
            if import.names.len() == 1 && import.names[0].name.as_str() == "*" {
                self.export_info.re_exports.push(ReExport {
                    from_module,
                    names: vec![],
                    is_star: true,
                });
            } else {
                // Collect specific imports
                let names: Vec<(String, Option<String>)> = import
                    .names
                    .iter()
                    .map(|alias| {
                        (
                            alias.name.to_string(),
                            alias.asname.as_ref().map(|s| s.to_string()),
                        )
                    })
                    .collect();

                self.export_info.re_exports.push(ReExport {
                    from_module,
                    names,
                    is_star: false,
                });
            }
        }
    }
}

impl<'a> Visitor<'a> for ExportCollector {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::Assign(assign) => {
                // Check for __all__ assignment
                if let Some(Expr::Name(name)) = assign.targets.first()
                    && name.id.as_str() == "__all__"
                    && let Some(exports) = self.extract_all_exports(&assign.value)
                {
                    self.current_all = Some(exports);
                }
            }
            Stmt::AugAssign(aug_assign) => {
                // Check for __all__ += [...] or similar
                if let Expr::Name(name) = &*aug_assign.target
                    && name.id.as_str() == "__all__"
                {
                    self.has_dynamic_all = true;
                }
            }
            Stmt::ImportFrom(import) => {
                // Check for re-export patterns
                self.check_for_reexport(import);
            }
            _ => {}
        }

        walk_stmt(self, stmt);
    }
}

#[cfg(test)]
mod tests {
    use ruff_python_parser::parse_module;

    use super::*;

    #[test]
    fn test_simple_all_export() {
        let code = r#"
__all__ = ["foo", "bar", "baz"]

def foo():
    pass

def bar():
    pass

def baz():
    pass

def _private():
    pass
"#;
        let parsed = parse_module(code).unwrap();
        let module = parsed.into_syntax();
        let export_info = ExportCollector::analyze(&module);

        assert!(!export_info.is_dynamic);
        assert_eq!(
            export_info.exported_names,
            Some(vec![
                "foo".to_string(),
                "bar".to_string(),
                "baz".to_string()
            ])
        );
    }

    #[test]
    fn test_dynamic_all() {
        let code = r#"
__all__ = []
__all__.append("foo")
__all__ += ["bar"]
"#;
        let parsed = parse_module(code).unwrap();
        let module = parsed.into_syntax();
        let export_info = ExportCollector::analyze(&module);

        assert!(export_info.is_dynamic);
    }

    #[test]
    fn test_reexports() {
        let code = r#"
from .submodule import foo, bar as baz
from . import module
from ..parent import *
"#;
        let parsed = parse_module(code).unwrap();
        let module = parsed.into_syntax();
        let export_info = ExportCollector::analyze(&module);

        assert_eq!(export_info.re_exports.len(), 3);

        // Check first re-export
        assert_eq!(export_info.re_exports[0].from_module, ".submodule");
        assert_eq!(export_info.re_exports[0].names.len(), 2);
        assert_eq!(
            export_info.re_exports[0].names[0],
            ("foo".to_string(), None)
        );
        assert_eq!(
            export_info.re_exports[0].names[1],
            ("bar".to_string(), Some("baz".to_string()))
        );

        // Check star import
        assert_eq!(export_info.re_exports[2].from_module, "..parent");
        assert!(export_info.re_exports[2].is_star);
    }

    #[test]
    fn test_tuple_all() {
        let code = r#"
__all__ = ("foo", "bar")
"#;
        let parsed = parse_module(code).unwrap();
        let module = parsed.into_syntax();
        let export_info = ExportCollector::analyze(&module);

        assert!(!export_info.is_dynamic);
        assert_eq!(
            export_info.exported_names,
            Some(vec!["foo".to_string(), "bar".to_string()])
        );
    }
}

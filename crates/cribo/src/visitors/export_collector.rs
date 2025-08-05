//! Export collection visitor for Python modules
//!
//! This visitor identifies module exports including __all__ declarations,
//! re-exports from imports, and implicit exports.

use ruff_python_ast::{
    Expr, ModModule, Stmt,
    visitor::{Visitor, walk_stmt},
};

use super::utils::extract_string_list_from_expr;
use crate::analyzers::types::ExportInfo;

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
        let result = extract_string_list_from_expr(expr);
        if result.is_dynamic {
            self.has_dynamic_all = true;
        }
        result.names
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
            // Import from statements and other statements - no processing needed
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
        let parsed = parse_module(code).expect("Test code should parse successfully");
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
        let parsed = parse_module(code).expect("Test code should parse successfully");
        let module = parsed.into_syntax();
        let export_info = ExportCollector::analyze(&module);

        assert!(export_info.is_dynamic);
    }

    #[test]
    fn test_tuple_all() {
        let code = r#"
__all__ = ("foo", "bar")
"#;
        let parsed = parse_module(code).expect("Test code should parse successfully");
        let module = parsed.into_syntax();
        let export_info = ExportCollector::analyze(&module);

        assert!(!export_info.is_dynamic);
        assert_eq!(
            export_info.exported_names,
            Some(vec!["foo".to_string(), "bar".to_string()])
        );
    }
}

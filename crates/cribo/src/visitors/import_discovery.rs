//! Import discovery visitor that finds all imports in a Python module,
//! including those nested within functions, classes, and other scopes.

use ruff_python_ast::visitor::{Visitor, walk_stmt};
use ruff_python_ast::{ModModule, Stmt, StmtImport, StmtImportFrom};
use ruff_text_size::TextRange;

/// An import discovered during AST traversal
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredImport {
    /// The module being imported
    pub module_name: Option<String>,
    /// Names being imported (for from imports)
    pub names: Vec<(String, Option<String>)>, // (name, alias)
    /// Location where the import was found
    pub location: ImportLocation,
    /// Source range of the import statement
    pub range: TextRange,
    /// Import level for relative imports
    pub level: u32,
}

/// Where an import was discovered in the AST
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportLocation {
    /// Import at module level
    Module,
    /// Import inside a function
    Function(String),
    /// Import inside a class definition
    Class(String),
    /// Import inside a method
    Method { class: String, method: String },
    /// Import inside a conditional block
    Conditional { depth: usize },
    /// Import inside other nested scope
    Nested(Vec<ScopeElement>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeElement {
    Function(String),
    Class(String),
    If,
    While,
    For,
    With,
    Try,
}

/// Visitor that discovers all imports in a Python module
#[derive(Default)]
pub struct ImportDiscoveryVisitor {
    /// All discovered imports
    imports: Vec<DiscoveredImport>,
    /// Current scope stack
    scope_stack: Vec<ScopeElement>,
}

impl ImportDiscoveryVisitor {
    /// Create a new import discovery visitor
    pub fn new() -> Self {
        Self::default()
    }

    /// Get all discovered imports
    pub fn into_imports(self) -> Vec<DiscoveredImport> {
        self.imports
    }

    /// Get the current location based on scope stack
    fn current_location(&self) -> ImportLocation {
        if self.scope_stack.is_empty() {
            return ImportLocation::Module;
        }

        // Analyze the scope stack to determine location
        match &self.scope_stack[..] {
            [ScopeElement::Function(name)] => ImportLocation::Function(name.clone()),
            [ScopeElement::Class(name)] => ImportLocation::Class(name.clone()),
            [
                ScopeElement::Class(class),
                ..,
                ScopeElement::Function(method),
            ] => ImportLocation::Method {
                class: class.clone(),
                method: method.clone(),
            },
            _ => {
                // Check if we're in any conditional
                let conditional_depth = self
                    .scope_stack
                    .iter()
                    .filter(|s| {
                        matches!(
                            s,
                            ScopeElement::If | ScopeElement::While | ScopeElement::For
                        )
                    })
                    .count();

                if conditional_depth > 0 {
                    ImportLocation::Conditional {
                        depth: conditional_depth,
                    }
                } else {
                    ImportLocation::Nested(self.scope_stack.clone())
                }
            }
        }
    }

    /// Record an import statement
    fn record_import(&mut self, stmt: &StmtImport) {
        for alias in &stmt.names {
            let import = DiscoveredImport {
                module_name: Some(alias.name.to_string()),
                names: vec![(
                    alias.name.to_string(),
                    alias.asname.as_ref().map(|n| n.to_string()),
                )],
                location: self.current_location(),
                range: stmt.range,
                level: 0,
            };
            self.imports.push(import);
        }
    }

    /// Record a from import statement
    fn record_import_from(&mut self, stmt: &StmtImportFrom) {
        let names: Vec<(String, Option<String>)> = stmt
            .names
            .iter()
            .map(|alias| {
                (
                    alias.name.to_string(),
                    alias.asname.as_ref().map(|n| n.to_string()),
                )
            })
            .collect();

        let import = DiscoveredImport {
            module_name: stmt.module.as_ref().map(|m| m.to_string()),
            names,
            location: self.current_location(),
            range: stmt.range,
            level: stmt.level,
        };
        self.imports.push(import);
    }

    /// Visit a module and discover all imports
    pub fn visit_module(&mut self, module: &ModModule) {
        for stmt in &module.body {
            self.visit_stmt(stmt);
        }
    }
}

impl<'a> Visitor<'a> for ImportDiscoveryVisitor {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::Import(import_stmt) => {
                self.record_import(import_stmt);
            }
            Stmt::ImportFrom(import_from) => {
                self.record_import_from(import_from);
            }
            Stmt::FunctionDef(func) => {
                self.scope_stack
                    .push(ScopeElement::Function(func.name.to_string()));
                // Visit the function body
                walk_stmt(self, stmt);
                self.scope_stack.pop();
                return; // Don't call walk_stmt again
            }
            Stmt::ClassDef(class) => {
                self.scope_stack
                    .push(ScopeElement::Class(class.name.to_string()));
                // Visit the class body
                walk_stmt(self, stmt);
                self.scope_stack.pop();
                return;
            }
            Stmt::If(_) => {
                self.scope_stack.push(ScopeElement::If);
                walk_stmt(self, stmt);
                self.scope_stack.pop();
                return;
            }
            Stmt::While(_) => {
                self.scope_stack.push(ScopeElement::While);
                walk_stmt(self, stmt);
                self.scope_stack.pop();
                return;
            }
            Stmt::For(_) => {
                self.scope_stack.push(ScopeElement::For);
                walk_stmt(self, stmt);
                self.scope_stack.pop();
                return;
            }
            Stmt::With(_) => {
                self.scope_stack.push(ScopeElement::With);
                walk_stmt(self, stmt);
                self.scope_stack.pop();
                return;
            }
            Stmt::Try(_) => {
                self.scope_stack.push(ScopeElement::Try);
                walk_stmt(self, stmt);
                self.scope_stack.pop();
                return;
            }
            _ => {}
        }

        // For other statement types, use default traversal
        walk_stmt(self, stmt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruff_python_parser::parse_module;

    #[test]
    fn test_module_level_import() {
        let source = r#"
import os
from sys import path
"#;
        let parsed = parse_module(source).expect("Failed to parse test module");
        let mut visitor = ImportDiscoveryVisitor::new();
        visitor.visit_module(parsed.syntax());
        let imports = visitor.into_imports();

        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].module_name, Some("os".to_string()));
        assert!(matches!(imports[0].location, ImportLocation::Module));
        assert_eq!(imports[1].module_name, Some("sys".to_string()));
        assert_eq!(imports[1].names, vec![("path".to_string(), None)]);
    }

    #[test]
    fn test_function_scoped_import() {
        let source = r#"
def my_function():
    import json
    from datetime import datetime
    return json.dumps({})
"#;
        let parsed = parse_module(source).expect("Failed to parse test module");
        let mut visitor = ImportDiscoveryVisitor::new();
        visitor.visit_module(parsed.syntax());
        let imports = visitor.into_imports();

        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].module_name, Some("json".to_string()));
        assert!(matches!(
            imports[0].location,
            ImportLocation::Function(ref name) if name == "my_function"
        ));
        assert_eq!(imports[1].module_name, Some("datetime".to_string()));
        assert_eq!(imports[1].names, vec![("datetime".to_string(), None)]);
    }

    #[test]
    fn test_class_method_import() {
        let source = r#"
class MyClass:
    def method(self):
        from collections import defaultdict
        return defaultdict(list)
"#;
        let parsed = parse_module(source).expect("Failed to parse test module");
        let mut visitor = ImportDiscoveryVisitor::new();
        visitor.visit_module(parsed.syntax());
        let imports = visitor.into_imports();

        assert_eq!(imports.len(), 1);
        assert!(matches!(
            imports[0].location,
            ImportLocation::Method { ref class, ref method } if class == "MyClass" && method == "method"
        ));
    }

    #[test]
    fn test_conditional_import() {
        let source = r#"
if True:
    import platform
    if platform.system() == "Windows":
        import winreg
"#;
        let parsed = parse_module(source).expect("Failed to parse test module");
        let mut visitor = ImportDiscoveryVisitor::new();
        visitor.visit_module(parsed.syntax());
        let imports = visitor.into_imports();

        assert_eq!(imports.len(), 2);
        assert!(matches!(
            imports[0].location,
            ImportLocation::Conditional { depth: 1 }
        ));
        assert!(matches!(
            imports[1].location,
            ImportLocation::Conditional { depth: 2 }
        ));
    }
}

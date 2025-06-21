//! Import discovery visitor that finds all imports in a Python module,
//! including those nested within functions, classes, and other scopes.
//! Also performs semantic analysis to determine import usage patterns.

use ruff_python_ast::{
    Expr, ExprName, Stmt, StmtImport, StmtImportFrom,
    visitor::{Visitor, walk_expr, walk_stmt},
};
use ruff_text_size::TextRange;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use crate::{cribo_graph::ModuleId, semantic_bundler::SemanticBundler};

/// Execution context for code - determines when code runs relative to module import
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecutionContext {
    /// Code at module level - executes when module is imported
    ModuleLevel,
    /// Inside a function body - executes when function is called
    FunctionBody,
    /// Inside a class body - executes when class is defined (at module import time)
    ClassBody,
    /// Inside a class method - executes when method is called
    ClassMethod { is_init: bool },
    /// Inside a decorator - executes at decoration time (usually module level)
    Decorator,
    /// Default parameter value - evaluated at function definition time
    DefaultParameter,
    /// Type annotation context - may not execute at runtime
    TypeAnnotation,
}

/// Usage information for an imported name
#[derive(Debug, Clone)]
pub struct ImportUsage {
    /// Where the name was used
    pub _location: TextRange,
    /// In what execution context
    pub _context: ExecutionContext,
    /// The actual name used (might be aliased)
    pub _name_used: String,
}

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
    /// Execution contexts where this import is used
    pub execution_contexts: HashSet<ExecutionContext>,
    /// Whether this import is used in a class __init__ method
    pub is_used_in_init: bool,
    /// Whether this import can be moved to function scope
    pub is_movable: bool,
    /// Whether this import is only used within TYPE_CHECKING blocks
    pub is_type_checking_only: bool,
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

/// Visitor that discovers all imports in a Python module and analyzes their usage
pub struct ImportDiscoveryVisitor<'a> {
    /// All discovered imports
    imports: Vec<DiscoveredImport>,
    /// Current scope stack
    scope_stack: Vec<ScopeElement>,
    /// Map from imported names to their module sources
    imported_names: HashMap<String, String>,
    /// Track usage of each imported name
    name_usage: HashMap<String, Vec<ImportUsage>>,
    /// Optional reference to semantic bundler for enhanced analysis
    _semantic_bundler: Option<&'a SemanticBundler>,
    /// Current module ID if available
    _module_id: Option<ModuleId>,
    /// Current execution context
    current_context: ExecutionContext,
    /// Whether we're in a type checking block
    in_type_checking: bool,
}

impl<'a> Default for ImportDiscoveryVisitor<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> ImportDiscoveryVisitor<'a> {
    /// Create a new import discovery visitor
    pub fn new() -> Self {
        Self {
            imports: Vec::new(),
            scope_stack: Vec::new(),
            imported_names: HashMap::default(),
            name_usage: HashMap::default(),
            _semantic_bundler: None,
            _module_id: None,
            current_context: ExecutionContext::ModuleLevel,
            in_type_checking: false,
        }
    }

    /// Create a new visitor with semantic bundler for enhanced analysis
    pub fn with_semantic_bundler(
        semantic_bundler: &'a SemanticBundler,
        module_id: ModuleId,
    ) -> Self {
        Self {
            imports: Vec::new(),
            scope_stack: Vec::new(),
            imported_names: HashMap::default(),
            name_usage: HashMap::default(),
            _semantic_bundler: Some(semantic_bundler),
            _module_id: Some(module_id),
            current_context: ExecutionContext::ModuleLevel,
            in_type_checking: false,
        }
    }

    /// Get all discovered imports
    pub fn into_imports(mut self) -> Vec<DiscoveredImport> {
        // Post-process imports to determine movability based on usage
        for i in 0..self.imports.len() {
            let import = &self.imports[i];
            let is_movable = self.is_import_movable(import);
            self.imports[i].is_movable = is_movable;

            // An import is type-checking-only if it was imported in a TYPE_CHECKING block
            // AND is not used anywhere outside of TYPE_CHECKING blocks
            // We already set is_type_checking_only when the import was discovered
            // No need to update it here since we track usage contexts separately
        }
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
            [ScopeElement::Class(class), ScopeElement::Function(method)] => {
                ImportLocation::Method {
                    class: class.clone(),
                    method: method.clone(),
                }
            }
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

    /// Get current execution context based on scope stack
    fn get_current_execution_context(&self) -> ExecutionContext {
        if self.in_type_checking {
            return ExecutionContext::TypeAnnotation;
        }

        // Analyze scope stack to determine context
        for (i, scope) in self.scope_stack.iter().enumerate() {
            match scope {
                ScopeElement::Class(_) => {
                    // Check if we're in a method within this class
                    if i + 1 < self.scope_stack.len()
                        && let ScopeElement::Function(method_name) = &self.scope_stack[i + 1]
                    {
                        return ExecutionContext::ClassMethod {
                            is_init: method_name == "__init__",
                        };
                    }
                    return ExecutionContext::ClassBody;
                }
                ScopeElement::Function(_) => {
                    // If we're in a function at module level, it's a function body
                    if i == 0 {
                        return ExecutionContext::FunctionBody;
                    }
                }
                _ => {}
            }
        }

        self.current_context
    }

    /// Analyze whether an import can be moved to function scope
    fn is_import_movable(&self, import: &DiscoveredImport) -> bool {
        // Check if import has side effects
        if let Some(module_name) = &import.module_name
            && self.is_side_effect_import(module_name)
        {
            return false;
        }

        // Check execution contexts where import is used
        let requires_module_level = import.execution_contexts.iter().any(|ctx| match ctx {
            ExecutionContext::ModuleLevel
            | ExecutionContext::ClassBody
            | ExecutionContext::Decorator
            | ExecutionContext::DefaultParameter => true,
            ExecutionContext::ClassMethod { is_init } => *is_init,
            ExecutionContext::FunctionBody | ExecutionContext::TypeAnnotation => false,
        });

        !requires_module_level && !import.is_used_in_init
    }

    /// Check if a condition is a TYPE_CHECKING check
    fn is_type_checking_condition(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Name(name) => name.id.as_str() == "TYPE_CHECKING",
            Expr::Attribute(attr) => {
                if attr.attr.as_str() == "TYPE_CHECKING"
                    && let Expr::Name(name) = &*attr.value
                {
                    return name.id.as_str() == "typing";
                }
                false
            }
            _ => false,
        }
    }

    /// Check if a module import has side effects
    fn is_side_effect_import(&self, module_name: &str) -> bool {
        matches!(
            module_name,
            "antigravity"
                | "this"
                | "__hello__"
                | "__phello__"
                | "site"
                | "sitecustomize"
                | "usercustomize"
                | "readline"
                | "rlcompleter"
                | "turtle"
                | "tkinter"
                | "webbrowser"
                | "platform"
                | "locale"
                | "os"
                | "sys"
                | "logging"
                | "warnings"
                | "encodings"
                | "pygame"
                | "matplotlib"
        ) || module_name.starts_with('_')
    }

    /// Record an import statement
    fn record_import(&mut self, stmt: &StmtImport) {
        for alias in &stmt.names {
            let module_name = alias.name.to_string();
            let imported_as = alias
                .asname
                .as_ref()
                .map(|n| n.to_string())
                .unwrap_or_else(|| module_name.clone());

            // Track the import mapping
            self.imported_names
                .insert(imported_as.clone(), module_name.clone());

            let import = DiscoveredImport {
                module_name: Some(module_name),
                names: vec![(
                    alias.name.to_string(),
                    alias.asname.as_ref().map(|n| n.to_string()),
                )],
                location: self.current_location(),
                range: stmt.range,
                level: 0,
                execution_contexts: HashSet::default(),
                is_used_in_init: false,
                is_movable: false,
                is_type_checking_only: self.in_type_checking,
            };
            self.imports.push(import);
        }
    }

    /// Record a from import statement
    fn record_import_from(&mut self, stmt: &StmtImportFrom) {
        let module_name = stmt.module.as_ref().map(|m| m.to_string());

        let names: Vec<(String, Option<String>)> = stmt
            .names
            .iter()
            .map(|alias| {
                let name = alias.name.to_string();
                let asname = alias.asname.as_ref().map(|n| n.to_string());

                // Track import mappings
                let imported_as = asname.as_ref().unwrap_or(&name).clone();
                if let Some(mod_name) = &module_name {
                    self.imported_names
                        .insert(imported_as, format!("{mod_name}.{name}"));
                }

                (name, asname)
            })
            .collect();

        let import = DiscoveredImport {
            module_name,
            names,
            location: self.current_location(),
            range: stmt.range,
            level: stmt.level,
            execution_contexts: HashSet::default(),
            is_used_in_init: false,
            is_movable: false,
            is_type_checking_only: self.in_type_checking,
        };
        self.imports.push(import);
    }
}

impl<'a> Visitor<'a> for ImportDiscoveryVisitor<'a> {
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
            Stmt::If(if_stmt) => {
                // Check if this is a TYPE_CHECKING block
                let was_type_checking = self.in_type_checking;
                if self.is_type_checking_condition(&if_stmt.test) {
                    self.in_type_checking = true;
                }

                self.scope_stack.push(ScopeElement::If);
                walk_stmt(self, stmt);
                self.scope_stack.pop();

                // Restore the previous type checking state
                self.in_type_checking = was_type_checking;
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

    fn visit_expr(&mut self, expr: &'a Expr) {
        if let Expr::Name(ExprName { id, range, .. }) = expr {
            let name = id.to_string();

            // Check if this is an imported name
            if self.imported_names.contains_key(&name) {
                let context = self.get_current_execution_context();

                // Record usage
                self.name_usage
                    .entry(name.clone())
                    .or_default()
                    .push(ImportUsage {
                        _location: *range,
                        _context: context,
                        _name_used: name.clone(),
                    });

                // Update the import's execution contexts
                if let Some(module_source) = self.imported_names.get(&name) {
                    // Find the corresponding import and update its contexts
                    for import in &mut self.imports {
                        if import.module_name.as_ref() == Some(module_source)
                            || import
                                .names
                                .iter()
                                .any(|(n, alias)| alias.as_ref().unwrap_or(n) == &name)
                        {
                            import.execution_contexts.insert(context);
                            if matches!(context, ExecutionContext::ClassMethod { is_init: true }) {
                                import.is_used_in_init = true;
                            }
                        }
                    }
                }
            }
        }

        // Continue traversal
        walk_expr(self, expr);
    }
}

#[cfg(test)]
mod tests {
    use ruff_python_ast::visitor::Visitor;
    use ruff_python_parser::parse_module;

    use super::*;

    #[test]
    fn test_module_level_import() {
        let source = r#"
import os
from sys import path
"#;
        let parsed = parse_module(source).expect("Failed to parse test module");
        let mut visitor = ImportDiscoveryVisitor::new();
        for stmt in &parsed.syntax().body {
            visitor.visit_stmt(stmt);
        }
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
        for stmt in &parsed.syntax().body {
            visitor.visit_stmt(stmt);
        }
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
        for stmt in &parsed.syntax().body {
            visitor.visit_stmt(stmt);
        }
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
        for stmt in &parsed.syntax().body {
            visitor.visit_stmt(stmt);
        }
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

    #[test]
    fn test_nested_function_in_method_not_misclassified() {
        let source = r#"
class MyClass:
    def method(self):
        def nested_function():
            import os
            return os.path.join('a', 'b')
        return nested_function()
"#;
        let parsed = parse_module(source).expect("Failed to parse test module");
        let mut visitor = ImportDiscoveryVisitor::new();
        for stmt in &parsed.syntax().body {
            visitor.visit_stmt(stmt);
        }
        let imports = visitor.into_imports();

        assert_eq!(imports.len(), 1);
        // The import should be classified as Nested, not Method
        assert!(matches!(
            imports[0].location,
            ImportLocation::Nested(ref scopes) if scopes.len() == 3 &&
                matches!(&scopes[0], ScopeElement::Class(c) if c == "MyClass") &&
                matches!(&scopes[1], ScopeElement::Function(m) if m == "method") &&
                matches!(&scopes[2], ScopeElement::Function(f) if f == "nested_function")
        ));
    }
}

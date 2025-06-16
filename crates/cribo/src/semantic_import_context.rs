//! Semantic analysis for import usage and execution contexts
//!
//! This module provides deep semantic analysis of import usage patterns to determine
//! when imports can be safely moved to function scope without causing runtime errors.

use anyhow::Result;
use log::{debug, trace};
use ruff_python_ast::{
    Expr, ExprName, ModModule, Parameters, Stmt, StmtClassDef, StmtFunctionDef,
    visitor::{Visitor, walk_expr, walk_stmt},
};
use ruff_text_size::TextRange;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use crate::cribo_graph::ModuleId;
use crate::semantic_bundler::SemanticBundler;

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
    /// Inside an if TYPE_CHECKING block
    TypeCheckingBlock,
}

impl ExecutionContext {
    /// Check if this context requires the import to be available at module initialization
    pub fn requires_module_level_import(&self) -> bool {
        match self {
            ExecutionContext::ModuleLevel
            | ExecutionContext::ClassBody
            | ExecutionContext::Decorator
            | ExecutionContext::DefaultParameter => true,
            ExecutionContext::FunctionBody => false,
            ExecutionContext::ClassMethod { is_init } => {
                // Class __init__ methods need imports available at module level
                // because the class needs to be instantiable when the module loads
                *is_init
            }
            ExecutionContext::TypeAnnotation | ExecutionContext::TypeCheckingBlock => false,
        }
    }

    /// Check if this is a deferred execution context
    pub fn is_deferred(&self) -> bool {
        matches!(
            self,
            ExecutionContext::FunctionBody | ExecutionContext::ClassMethod { is_init: false }
        )
    }
}

/// Represents how an import is used in the code
#[derive(Debug, Clone)]
pub struct ImportUsage {
    /// The name being used from the import
    pub name: String,
    /// Where in the code it's used
    pub location: TextRange,
    /// The execution context of the usage
    pub context: ExecutionContext,
    /// The scope path to this usage (e.g., ["MyClass", "__init__"])
    pub scope_path: Vec<String>,
}

/// Enhanced import information with semantic usage analysis
#[derive(Debug, Clone)]
pub struct SemanticImportInfo {
    /// The module being imported
    pub module_name: Option<String>,
    /// Names imported from the module
    pub imported_names: Vec<(String, Option<String>)>, // (name, alias)
    /// All usages of this import
    pub usages: Vec<ImportUsage>,
    /// Whether this import has side effects
    pub has_side_effects: bool,
    /// Import level for relative imports
    pub level: u32,
    /// Location of the import statement
    pub import_location: TextRange,
}

impl SemanticImportInfo {
    /// Check if this import requires module-level availability
    pub fn requires_module_level(&self) -> bool {
        // If any usage requires module-level, the import must stay at module level
        self.usages
            .iter()
            .any(|usage| usage.context.requires_module_level_import())
    }

    /// Check if this import is only used in deferred contexts
    pub fn is_deferred_only(&self) -> bool {
        !self.usages.is_empty() && self.usages.iter().all(|usage| usage.context.is_deferred())
    }

    /// Get all functions that use this import
    pub fn get_using_functions(&self) -> HashSet<String> {
        self.usages
            .iter()
            .filter_map(|usage| {
                if matches!(
                    usage.context,
                    ExecutionContext::FunctionBody | ExecutionContext::ClassMethod { .. }
                ) {
                    usage.scope_path.first().cloned()
                } else {
                    None
                }
            })
            .collect()
    }
}

/// Semantic analyzer for import usage patterns
pub struct SemanticImportAnalyzer<'a> {
    /// Reference to the semantic bundler
    semantic_bundler: &'a SemanticBundler,
    /// Current module being analyzed
    module_id: ModuleId,
    /// Import information being collected
    imports: HashMap<String, SemanticImportInfo>,
    /// Current execution context stack
    context_stack: Vec<ExecutionContext>,
    /// Current scope name stack
    scope_stack: Vec<String>,
    /// Names imported in the current module
    imported_names: HashMap<String, String>, // name -> module
}

impl<'a> SemanticImportAnalyzer<'a> {
    /// Create a new semantic import analyzer
    pub fn new(semantic_bundler: &'a SemanticBundler, module_id: ModuleId) -> Self {
        Self {
            semantic_bundler,
            module_id,
            imports: HashMap::default(),
            context_stack: vec![ExecutionContext::ModuleLevel],
            scope_stack: vec![],
            imported_names: HashMap::default(),
        }
    }

    /// Analyze a module and return semantic import information
    pub fn analyze_module(&mut self, module: &ModModule) -> Result<Vec<SemanticImportInfo>> {
        debug!(
            "Starting semantic import analysis for module {:?}",
            self.module_id
        );

        // First pass: collect all imports
        for stmt in &module.body {
            if let Stmt::Import(_) | Stmt::ImportFrom(_) = stmt {
                self.visit_stmt(stmt);
            }
        }

        // Second pass: analyze usage
        for stmt in &module.body {
            self.visit_stmt(stmt);
        }

        Ok(self.imports.drain().map(|(_, info)| info).collect())
    }

    /// Get current execution context
    fn current_context(&self) -> ExecutionContext {
        self.context_stack
            .last()
            .copied()
            .unwrap_or(ExecutionContext::ModuleLevel)
    }

    /// Push a new execution context
    fn push_context(&mut self, context: ExecutionContext) {
        self.context_stack.push(context);
    }

    /// Pop the current execution context
    fn pop_context(&mut self) {
        self.context_stack.pop();
    }

    /// Record usage of an imported name
    fn record_usage(&mut self, name: &str, location: TextRange) {
        if let Some(module) = self.imported_names.get(name).cloned() {
            let context = self.current_context();
            let scope_path = self.scope_stack.clone();

            if let Some(import_info) = self.imports.get_mut(&module) {
                import_info.usages.push(ImportUsage {
                    name: name.to_string(),
                    location,
                    context,
                    scope_path,
                });

                trace!(
                    "Recorded usage of '{}' from '{}' in context {:?}",
                    name, module, context
                );
            }
        }
    }

    /// Check if a module has side effects
    fn has_side_effects(module_name: &str) -> bool {
        // Known modules with side effects
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
}

impl<'a> Visitor<'_> for SemanticImportAnalyzer<'a> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Import(import_stmt) => {
                for alias in &import_stmt.names {
                    let module_name = alias.name.to_string();
                    let imported_as = alias
                        .asname
                        .as_ref()
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| module_name.clone());

                    self.imported_names
                        .insert(imported_as.clone(), module_name.clone());

                    self.imports
                        .entry(module_name.clone())
                        .or_insert_with(|| SemanticImportInfo {
                            module_name: Some(module_name.clone()),
                            imported_names: vec![(
                                module_name.clone(),
                                alias.asname.as_ref().map(|n| n.to_string()),
                            )],
                            usages: vec![],
                            has_side_effects: Self::has_side_effects(&module_name),
                            level: 0,
                            import_location: import_stmt.range,
                        });
                }
            }

            Stmt::ImportFrom(import_from) => {
                let module_name = import_from
                    .module
                    .as_ref()
                    .map(|m| m.to_string())
                    .unwrap_or_else(|| format!(".<relative:{}>", import_from.level));

                let mut names = vec![];
                for alias in &import_from.names {
                    let name = alias.name.to_string();
                    let imported_as = alias
                        .asname
                        .as_ref()
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| name.clone());

                    self.imported_names.insert(imported_as, module_name.clone());
                    names.push((name, alias.asname.as_ref().map(|n| n.to_string())));
                }

                self.imports
                    .entry(module_name.clone())
                    .or_insert_with(|| SemanticImportInfo {
                        module_name: import_from.module.as_ref().map(|m| m.to_string()),
                        imported_names: names,
                        usages: vec![],
                        has_side_effects: import_from
                            .module
                            .as_ref()
                            .map(|m| Self::has_side_effects(m.as_str()))
                            .unwrap_or(false),
                        level: import_from.level,
                        import_location: import_from.range,
                    });
            }

            Stmt::FunctionDef(func_def) => {
                self.visit_function_def(func_def);
                return; // Don't call walk_stmt
            }

            Stmt::ClassDef(class_def) => {
                self.visit_class_def(class_def);
                return; // Don't call walk_stmt
            }

            _ => {}
        }

        walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Name(ExprName { id, range, .. }) => {
                self.record_usage(id.as_str(), *range);
            }

            Expr::Attribute(attr_expr) => {
                // For now, just visit the value part
                // Could be enhanced to track module.attribute usage
                self.visit_expr(&attr_expr.value);
                return;
            }

            _ => {}
        }

        walk_expr(self, expr);
    }
}

impl<'a> SemanticImportAnalyzer<'a> {
    /// Visit a function definition with proper context
    fn visit_function_def(&mut self, func_def: &StmtFunctionDef) {
        let func_name = func_def.name.to_string();

        // Visit decorators in decorator context
        self.push_context(ExecutionContext::Decorator);
        for decorator in &func_def.decorator_list {
            self.visit_expr(&decorator.expression);
        }
        self.pop_context();

        // Visit parameter defaults in module context (evaluated at definition time)
        self.push_context(ExecutionContext::DefaultParameter);
        self.visit_parameters(&func_def.parameters);
        self.pop_context();

        // Visit function body in function context
        self.scope_stack.push(func_name);
        self.push_context(ExecutionContext::FunctionBody);
        for stmt in &func_def.body {
            self.visit_stmt(stmt);
        }
        self.pop_context();
        self.scope_stack.pop();
    }

    /// Visit a class definition with proper context
    fn visit_class_def(&mut self, class_def: &StmtClassDef) {
        let class_name = class_def.name.to_string();

        // Visit decorators in decorator context
        self.push_context(ExecutionContext::Decorator);
        for decorator in &class_def.decorator_list {
            self.visit_expr(&decorator.expression);
        }
        self.pop_context();

        // Visit base classes in module context
        if let Some(arguments) = &class_def.arguments {
            for base in &arguments.args {
                self.visit_expr(base);
            }
        }

        // Visit class body
        self.scope_stack.push(class_name);
        self.push_context(ExecutionContext::ClassBody);

        for stmt in &class_def.body {
            match stmt {
                Stmt::FunctionDef(method_def) => {
                    let method_name = method_def.name.to_string();
                    let is_init = method_name == "__init__";

                    // Visit method decorators
                    self.push_context(ExecutionContext::Decorator);
                    for decorator in &method_def.decorator_list {
                        self.visit_expr(&decorator.expression);
                    }
                    self.pop_context();

                    // Visit method parameters
                    self.push_context(ExecutionContext::DefaultParameter);
                    self.visit_parameters(&method_def.parameters);
                    self.pop_context();

                    // Visit method body
                    self.scope_stack.push(method_name);
                    self.push_context(ExecutionContext::ClassMethod { is_init });
                    for method_stmt in &method_def.body {
                        self.visit_stmt(method_stmt);
                    }
                    self.pop_context();
                    self.scope_stack.pop();
                }
                _ => {
                    self.visit_stmt(stmt);
                }
            }
        }

        self.pop_context();
        self.scope_stack.pop();
    }

    /// Visit function parameters for default value analysis
    fn visit_parameters(&mut self, parameters: &Parameters) {
        // Visit default values for positional-only parameters
        for param in &parameters.posonlyargs {
            if let Some(default) = &param.default {
                self.visit_expr(default);
            }
        }

        // Visit default values for regular parameters
        for param in &parameters.args {
            if let Some(default) = &param.default {
                self.visit_expr(default);
            }
        }

        // Visit default values for keyword-only parameters
        for param in &parameters.kwonlyargs {
            if let Some(default) = &param.default {
                self.visit_expr(default);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruff_python_parser::parse_module;

    #[test]
    fn test_class_init_usage_detection() {
        let source = r#"
from logger import get_logger

class Config:
    def __init__(self):
        self.logger = get_logger()
"#;

        let parsed = parse_module(source).expect("Failed to parse");
        let semantic_bundler = SemanticBundler::new();
        let module_id = ModuleId::new(0);

        let mut analyzer = SemanticImportAnalyzer::new(&semantic_bundler, module_id);
        let imports = analyzer.analyze_module(parsed.syntax()).unwrap();

        assert_eq!(imports.len(), 1);
        let logger_import = &imports[0];
        assert_eq!(logger_import.module_name, Some("logger".to_string()));
        assert!(logger_import.requires_module_level());
        assert!(!logger_import.is_deferred_only());
    }

    #[test]
    fn test_function_only_usage() {
        let source = r#"
from utils import helper

def process():
    return helper()
"#;

        let parsed = parse_module(source).expect("Failed to parse");
        let semantic_bundler = SemanticBundler::new();
        let module_id = ModuleId::new(0);

        let mut analyzer = SemanticImportAnalyzer::new(&semantic_bundler, module_id);
        let imports = analyzer.analyze_module(parsed.syntax()).unwrap();

        assert_eq!(imports.len(), 1);
        let utils_import = &imports[0];
        assert!(!utils_import.requires_module_level());
        assert!(utils_import.is_deferred_only());
    }
}

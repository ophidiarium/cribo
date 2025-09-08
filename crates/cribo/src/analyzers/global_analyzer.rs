//! Global variable analyzer for wrapper modules
//!
//! This analyzer traverses module ASTs to identify:
//! - Module-level variable definitions
//! - Global declarations within functions (including async functions)
//! - Functions that use global statements
//!
//! This information is used to determine which variables need to be lifted
//! to true globals in the bundled output to preserve Python's global semantics.

use ruff_python_ast::{
    Expr, ModModule, Stmt, StmtFunctionDef,
    visitor::source_order::{self, SourceOrderVisitor},
};
use ruff_text_size::TextRange;

use crate::{
    semantic_bundler::ModuleGlobalInfo,
    types::{FxIndexMap, FxIndexSet},
};

/// Visitor that analyzes a module for global variable usage patterns
pub struct GlobalAnalyzer {
    /// Module-level variables collected during first pass
    module_level_vars: FxIndexSet<String>,

    /// Names eligible for lifting via `global` (module assigns/classes + imports + functions)
    liftable_vars: FxIndexSet<String>,

    /// Global declarations found in functions
    global_declarations: FxIndexMap<String, Vec<TextRange>>,

    /// Functions that contain global statements
    functions_using_globals: FxIndexSet<String>,

    /// Module name being analyzed
    module_name: String,

    /// Current function name stack (for nested functions)
    function_stack: Vec<String>,

    /// Whether we're currently at module level
    at_module_level: bool,
}

impl GlobalAnalyzer {
    /// Create a new global analyzer for a module
    pub fn new(module_name: impl Into<String>) -> Self {
        Self {
            module_level_vars: FxIndexSet::default(),
            liftable_vars: FxIndexSet::default(),
            global_declarations: FxIndexMap::default(),
            functions_using_globals: FxIndexSet::default(),
            module_name: module_name.into(),
            function_stack: Vec::new(),
            at_module_level: true,
        }
    }

    /// Analyze a module and return global usage information
    pub fn analyze(module_name: impl Into<String>, ast: &ModModule) -> Option<ModuleGlobalInfo> {
        let mut analyzer = Self::new(module_name);
        source_order::walk_body(&mut analyzer, &ast.body);
        analyzer.into_global_info()
    }

    /// Convert the analyzer state into `ModuleGlobalInfo` if any globals were found
    fn into_global_info(self) -> Option<ModuleGlobalInfo> {
        if self.global_declarations.is_empty() {
            None
        } else {
            Some(ModuleGlobalInfo {
                module_level_vars: self.module_level_vars,
                liftable_vars: self.liftable_vars,
                global_declarations: self.global_declarations,
                functions_using_globals: self.functions_using_globals,
                module_name: self.module_name,
            })
        }
    }

    /// Helper to collect variable names from assignment targets
    fn collect_from_target(&mut self, target: &Expr) {
        match target {
            Expr::Name(name) => {
                if self.at_module_level {
                    self.module_level_vars.insert(name.id.to_string());
                    self.liftable_vars.insert(name.id.to_string());
                }
            }
            Expr::Tuple(tuple) => {
                for elt in &tuple.elts {
                    self.collect_from_target(elt);
                }
            }
            Expr::List(list) => {
                for elt in &list.elts {
                    self.collect_from_target(elt);
                }
            }
            Expr::Starred(starred) => {
                self.collect_from_target(&starred.value);
            }
            _ => {}
        }
    }

    /// Process a function definition (handles both sync and async functions)
    /// Note: Async functions are represented as `StmtFunctionDef` with `is_async` flag
    fn process_function(&mut self, func_def: &StmtFunctionDef) {
        // Push function name onto stack
        self.function_stack.push(func_def.name.id.to_string());
        let was_module_level = self.at_module_level;
        self.at_module_level = false;

        let mut has_globals = false;

        // Visit the function body
        for stmt in &func_def.body {
            if let Stmt::Global(global_stmt) = stmt {
                has_globals = true;
                for identifier in &global_stmt.names {
                    let var_name = identifier.id.to_string();
                    self.global_declarations
                        .entry(var_name)
                        .or_default()
                        .push(identifier.range);
                }
            }
            // Continue visiting other statements
            self.visit_stmt(stmt);
        }

        // Track this function if it uses globals
        if has_globals {
            // Use the full function path for nested functions
            let function_name = self.function_stack.join(".");
            self.functions_using_globals.insert(function_name);
        }

        // Restore state
        self.at_module_level = was_module_level;
        self.function_stack.pop();
    }
}

impl<'a> SourceOrderVisitor<'a> for GlobalAnalyzer {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            // Collect module-level variable definitions
            Stmt::Assign(assign) if self.at_module_level => {
                for target in &assign.targets {
                    self.collect_from_target(target);
                }
                // Continue default traversal
                source_order::walk_stmt(self, stmt);
            }
            Stmt::AnnAssign(ann_assign) if self.at_module_level => {
                self.collect_from_target(&ann_assign.target);
                // Continue default traversal
                source_order::walk_stmt(self, stmt);
            }
            Stmt::AugAssign(aug_assign) if self.at_module_level => {
                self.collect_from_target(&aug_assign.target);
                // Continue default traversal
                source_order::walk_stmt(self, stmt);
            }

            // Track module-level imports as defining names in the module namespace
            Stmt::Import(import_stmt) if self.at_module_level => {
                for alias in &import_stmt.names {
                    // Local binding is `asname` if present, otherwise top-level package segment
                    let name = if let Some(asname) = &alias.asname {
                        asname.to_string()
                    } else {
                        let full = alias.name.to_string();
                        full.split('.').next().unwrap_or(&full).to_string()
                    };
                    // Imports are liftable for globals handling but shouldn't affect
                    // module-level var rewriting behavior
                    self.liftable_vars.insert(name);
                }
                // Continue default traversal
                source_order::walk_stmt(self, stmt);
            }
            Stmt::ImportFrom(from_stmt) if self.at_module_level => {
                for alias in &from_stmt.names {
                    // Skip wildcard imports (from m import *)
                    if alias.name.as_str() == "*" {
                        continue;
                    }
                    let binding = alias
                        .asname
                        .as_ref()
                        .map_or_else(|| alias.name.to_string(), ToString::to_string);
                    // From-import bindings are liftable but not regular module vars
                    self.liftable_vars.insert(binding);
                }
                // Continue default traversal
                source_order::walk_stmt(self, stmt);
            }

            // Process function definitions (includes async functions)
            // Note: In ruff's AST, async functions are represented as FunctionDef with is_async
            // flag
            Stmt::FunctionDef(func_def) => {
                // Functions are liftable for globals handling but don't participate in
                // module-level var rewriting which can cause premature self-references
                if self.at_module_level {
                    self.liftable_vars.insert(func_def.name.id.to_string());
                }
                self.process_function(func_def);
                // Don't use walk_stmt here as we already visited the body
            }

            // Process class definitions (they create a new scope)
            Stmt::ClassDef(class_def) => {
                let was_module_level = self.at_module_level;
                self.at_module_level = false;

                // Visit class body
                source_order::walk_stmt(self, stmt);

                self.at_module_level = was_module_level;

                // The class name itself is a module-level variable
                if self.at_module_level {
                    self.module_level_vars.insert(class_def.name.id.to_string());
                    self.liftable_vars.insert(class_def.name.id.to_string());
                }
            }

            _ => {
                // Continue default traversal for other statements
                source_order::walk_stmt(self, stmt);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ruff_python_parser::parse_module;

    use super::*;

    #[test]
    fn test_module_level_vars() {
        let source = r"
x = 1
y = 2
z = x + y

def foo():
    global x
    x = 10
        ";

        let parsed = parse_module(source).expect("Test code should parse successfully");
        let info = GlobalAnalyzer::analyze("test_module", parsed.syntax())
            .expect("Should return Some when there are global declarations");

        assert!(info.module_level_vars.contains("x"));
        assert!(info.module_level_vars.contains("y"));
        assert!(info.module_level_vars.contains("z"));
        assert!(info.global_declarations.contains_key("x"));
        assert!(info.functions_using_globals.contains("foo"));
    }

    #[test]
    fn test_global_declarations() {
        let source = r"
x = 1

def foo():
    global x
    x = 2
    
def bar():
    y = 3  # local
        ";

        let parsed = parse_module(source).expect("Test code should parse successfully");
        let info = GlobalAnalyzer::analyze("test_module", parsed.syntax());

        assert!(info.is_some());
        let info = info.expect("Should return Some when there are global declarations");

        assert!(info.global_declarations.contains_key("x"));
        assert!(info.functions_using_globals.contains("foo"));
        assert!(!info.functions_using_globals.contains("bar"));
    }

    #[test]
    fn test_nested_functions() {
        let source = r"
x = 1

def outer():
    def inner():
        global x
        x = 2
        ";

        let parsed = parse_module(source).expect("Test code should parse successfully");
        let info = GlobalAnalyzer::analyze("test_module", parsed.syntax());

        assert!(info.is_some());
        let info = info.expect("Should return Some when there are global declarations");

        assert!(info.global_declarations.contains_key("x"));
        assert!(info.functions_using_globals.contains("outer.inner"));
    }

    #[test]
    fn test_no_globals() {
        let source = r"
def foo():
    x = 1
    return x
        ";

        let parsed = parse_module(source).expect("Test code should parse successfully");
        let info = GlobalAnalyzer::analyze("test_module", parsed.syntax());

        assert!(info.is_none());
    }

    #[test]
    fn test_async_function_globals() {
        let source = r"
x = 10
y = 20

async def async_func():
    global x
    x = 100
    return x

async def nested_async():
    async def inner():
        global y
        y = 200
    await inner()
        ";

        let parsed = parse_module(source).expect("Failed to parse module with async functions");
        let info = GlobalAnalyzer::analyze("test_module", parsed.syntax());

        assert!(info.is_some());
        let info = info.expect("Expected global info for async functions");

        // Check that async functions are recognized
        assert!(info.functions_using_globals.contains("async_func"));
        assert!(info.functions_using_globals.contains("nested_async.inner"));

        // Check that global declarations in async functions are tracked
        assert!(info.global_declarations.contains_key("x"));
        assert!(info.global_declarations.contains_key("y"));

        // Check module-level vars
        assert!(info.module_level_vars.contains("x"));
        assert!(info.module_level_vars.contains("y"));
    }

    #[test]
    fn test_module_level_function_and_imports_tracked() {
        let source = r"
import logging

def reconfigure():
    global logging
    logging = custom
";

        let parsed = parse_module(source).expect("Failed to parse module");
        let info = GlobalAnalyzer::analyze("test_module", parsed.syntax())
            .expect("Expected global info due to global in function");

        // Liftable vars include imported name and function name
        assert!(info.liftable_vars.contains("logging"));
        assert!(info.liftable_vars.contains("reconfigure"));

        // Global declarations should include the imported name
        assert!(info.global_declarations.contains_key("logging"));
        assert!(info.functions_using_globals.contains("reconfigure"));
    }

    #[test]
    fn test_from_import_and_dotted_import_binding_names() {
        let source = r"
import a.b.c
from pkg import mod as m, other

def f():
    global m
    m = 1
";

        let parsed = parse_module(source).expect("Failed to parse module");
        let info = GlobalAnalyzer::analyze("test_module", parsed.syntax())
            .expect("Expected global info due to global in function");

        // 'import a.b.c' makes top-level name available; treat as liftable
        assert!(info.liftable_vars.contains("a"));
        // From-import binds alias or name as liftable vars
        assert!(info.liftable_vars.contains("m"));
        assert!(info.liftable_vars.contains("other"));

        // Globals tracked
        assert!(info.global_declarations.contains_key("m"));
        assert!(info.functions_using_globals.contains("f"));
    }
}

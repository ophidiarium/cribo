//! Analyzer for detecting cross-module inheritance forward references
//!
//! This analyzer checks if there are forward reference issues that can occur
//! when class inheritance involves symbols or namespaces that are defined later
//! in the bundled output.

use ruff_python_ast::{Expr, Stmt};

use crate::{
    ast_builder::expressions::expr_to_dotted_name,
    code_generator::module_registry::is_init_function, types::FxIndexMap,
};

/// Analyzer for detecting forward reference issues in bundled code
pub struct ForwardReferenceAnalyzer;

impl ForwardReferenceAnalyzer {
    /// Check if there are cross-module inheritance forward references in the given statements
    ///
    /// This detects cases where:
    /// - A class inherits from another class that is defined later (via direct definition or assignment)
    /// - A class inherits from a namespace attribute where the namespace is initialized later
    ///
    /// Returns `true` if forward references are detected that require reordering
    pub fn has_cross_module_inheritance_forward_refs(statements: &[Stmt]) -> bool {
        // Look for classes that inherit from base classes that are defined later
        // This can happen when symbol renaming creates forward references

        // First, collect all class positions and assignment positions
        let mut class_positions = FxIndexMap::default();
        let mut assignment_positions = FxIndexMap::default();
        let mut namespace_init_positions = FxIndexMap::default();

        for (idx, stmt) in statements.iter().enumerate() {
            match stmt {
                Stmt::ClassDef(class_def) => {
                    class_positions.insert(class_def.name.to_string(), idx);
                }
                Stmt::Assign(assign) => {
                    // Check if this is a simple assignment like HTTPBasicAuth = HTTPBasicAuth_2
                    if assign.targets.len() == 1
                        && let Expr::Name(target) = &assign.targets[0]
                    {
                        assignment_positions.insert(target.id.to_string(), idx);
                    }
                    // Also check for namespace init assignments like:
                    // mypkg.compat = __cribo_init_...()
                    if assign.targets.len() == 1
                        && let Expr::Attribute(attr) = &assign.targets[0]
                        && let Expr::Call(call) = assign.value.as_ref()
                        && let Expr::Name(func_name) = call.func.as_ref()
                        && is_init_function(func_name.id.as_str())
                    {
                        // Extract the namespace path (e.g., "mypkg.compat")
                        let namespace_path = expr_to_dotted_name(&Expr::Attribute(attr.clone()));
                        namespace_init_positions.insert(namespace_path, idx);
                    }
                }
                _ => {}
            }
        }

        // Now check for forward references
        for (idx, stmt) in statements.iter().enumerate() {
            if let Stmt::ClassDef(class_def) = stmt
                && let Some(arguments) = &class_def.arguments
            {
                let class_name = class_def.name.as_str();
                let class_pos = idx;

                for base in &arguments.args {
                    // Check simple name references
                    if let Expr::Name(name_expr) = base {
                        let base_name = name_expr.id.as_str();

                        // Check if the base class is defined via assignment later
                        if let Some(&assign_pos) = assignment_positions.get(base_name)
                            && assign_pos > class_pos
                        {
                            return true;
                        }

                        // Check if the base class itself is defined later as a class
                        // This covers both regular and renamed classes
                        if let Some(&base_pos) = class_positions.get(base_name)
                            && base_pos > class_pos
                        {
                            return true;
                        }
                    }
                    // Check attribute references (e.g., mypkg.compat.JSONDecodeError)
                    else if let Expr::Attribute(attr_expr) = base {
                        // Extract the base module path (e.g., "mypkg.compat" from
                        // "mypkg.compat.JSONDecodeError")
                        let base_path = expr_to_dotted_name(&attr_expr.value);
                        // Check if this namespace is initialized later
                        if let Some(&init_pos) = namespace_init_positions.get(&base_path)
                            && init_pos > class_pos
                        {
                            log::debug!(
                                "Class '{}' inherits from {}.{} but namespace '{}' is initialized \
                                 later at position {} (class at {})",
                                class_name,
                                base_path,
                                attr_expr.attr,
                                base_path,
                                init_pos,
                                class_pos
                            );
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

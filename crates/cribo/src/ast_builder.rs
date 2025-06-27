//! AST builder module for creating synthetic AST nodes
//!
//! This module provides factory functions for creating AST nodes that don't
//! originate from source files. All synthetic nodes use default ranges to
//! clearly indicate they are generated.

use ruff_python_ast::{
    Alias, Arguments, AtomicNodeIndex, Expr, ExprAttribute, ExprCall, ExprContext, ExprName,
    Identifier, ModModule, Stmt, StmtAssign, StmtImport, StmtImportFrom, name::Name,
};
use ruff_text_size::TextRange;

/// Create a synthetic range for generated nodes
fn synthetic_range() -> TextRange {
    TextRange::default()
}

/// Create an import statement: `import module_name`
pub fn import(module_name: &str) -> Stmt {
    Stmt::Import(StmtImport {
        names: vec![Alias {
            name: Identifier::new(module_name, synthetic_range()),
            asname: None,
            range: synthetic_range(),
            node_index: AtomicNodeIndex::dummy(),
        }],
        range: synthetic_range(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Create an import statement with alias: `import module_name as alias`
pub fn import_as(module_name: &str, alias: &str) -> Stmt {
    Stmt::Import(StmtImport {
        names: vec![Alias {
            name: Identifier::new(module_name, synthetic_range()),
            asname: Some(Identifier::new(alias, synthetic_range())),
            range: synthetic_range(),
            node_index: AtomicNodeIndex::dummy(),
        }],
        range: synthetic_range(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Create a from import statement: `from module import name1, name2, ...`
pub fn from_import(module: &str, names: &[&str]) -> Stmt {
    let aliases = names
        .iter()
        .map(|name| Alias {
            name: Identifier::new(*name, synthetic_range()),
            asname: None,
            range: synthetic_range(),
            node_index: AtomicNodeIndex::dummy(),
        })
        .collect();

    Stmt::ImportFrom(StmtImportFrom {
        module: Some(Identifier::new(module, synthetic_range())),
        names: aliases,
        level: 0,
        range: synthetic_range(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Create a from import with aliases: `from module import (name, alias)...`
pub fn from_import_with_aliases(module: &str, imports: &[(&str, Option<&str>)]) -> Stmt {
    let aliases = imports
        .iter()
        .map(|(name, alias)| Alias {
            name: Identifier::new(*name, synthetic_range()),
            asname: alias.map(|a| Identifier::new(a, synthetic_range())),
            range: synthetic_range(),
            node_index: AtomicNodeIndex::dummy(),
        })
        .collect();

    Stmt::ImportFrom(StmtImportFrom {
        module: Some(Identifier::new(module, synthetic_range())),
        names: aliases,
        level: 0,
        range: synthetic_range(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Create a relative from import: `from ..module import name`
pub fn relative_from_import(module: Option<&str>, level: u32, names: &[&str]) -> Stmt {
    let aliases = names
        .iter()
        .map(|name| Alias {
            name: Identifier::new(*name, synthetic_range()),
            asname: None,
            range: synthetic_range(),
            node_index: AtomicNodeIndex::dummy(),
        })
        .collect();

    Stmt::ImportFrom(StmtImportFrom {
        module: module.map(|m| Identifier::new(m, synthetic_range())),
        names: aliases,
        level,
        range: synthetic_range(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Create a simple assignment: `target = value`
pub fn assign(target: &str, value: Expr) -> Stmt {
    Stmt::Assign(StmtAssign {
        targets: vec![Expr::Name(ExprName {
            id: Name::new(target),
            ctx: ExprContext::Store,
            range: synthetic_range(),
            node_index: AtomicNodeIndex::dummy(),
        })],
        value: Box::new(value),
        range: synthetic_range(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Create an attribute assignment: `obj.attr = value`
pub fn assign_attribute(obj: &str, attr: &str, value: Expr) -> Stmt {
    let target = Expr::Attribute(ExprAttribute {
        value: Box::new(Expr::Name(ExprName {
            id: Name::new(obj),
            ctx: ExprContext::Load,
            range: synthetic_range(),
            node_index: AtomicNodeIndex::dummy(),
        })),
        attr: Identifier::new(attr, synthetic_range()),
        ctx: ExprContext::Store,
        range: synthetic_range(),
        node_index: AtomicNodeIndex::dummy(),
    });

    Stmt::Assign(StmtAssign {
        targets: vec![target],
        value: Box::new(value),
        range: synthetic_range(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Create a name expression: `name`
pub fn name(name: &str) -> Expr {
    Expr::Name(ExprName {
        id: Name::new(name),
        ctx: ExprContext::Load,
        range: synthetic_range(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Create an attribute expression: `obj.attr`
pub fn attribute(obj: &str, attr: &str) -> Expr {
    Expr::Attribute(ExprAttribute {
        value: Box::new(name(obj)),
        attr: Identifier::new(attr, synthetic_range()),
        ctx: ExprContext::Load,
        range: synthetic_range(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Create a function call: `func()`
pub fn call(func: Expr) -> Expr {
    Expr::Call(ExprCall {
        func: Box::new(func),
        arguments: Arguments {
            args: Box::new([]),
            keywords: Box::new([]),
            range: synthetic_range(),
            node_index: AtomicNodeIndex::dummy(),
        },
        range: synthetic_range(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Create a function call with arguments: `func(arg1, arg2, ...)`
pub fn call_with_args(func: Expr, args: Vec<Expr>) -> Expr {
    Expr::Call(ExprCall {
        func: Box::new(func),
        arguments: Arguments {
            args: args.into_boxed_slice(),
            keywords: Box::new([]),
            range: synthetic_range(),
            node_index: AtomicNodeIndex::dummy(),
        },
        range: synthetic_range(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Create a from import statement with specific symbols: `from module import symbol1, symbol2, ...`
/// This is useful for partial import removal where we only keep certain symbols
pub fn from_import_specific(module: &str, symbols: &[(String, Option<String>)]) -> Stmt {
    let aliases = symbols
        .iter()
        .map(|(name, alias)| Alias {
            name: Identifier::new(name.clone(), synthetic_range()),
            asname: alias
                .as_ref()
                .map(|a| Identifier::new(a.clone(), synthetic_range())),
            range: synthetic_range(),
            node_index: AtomicNodeIndex::dummy(),
        })
        .collect();

    Stmt::ImportFrom(StmtImportFrom {
        module: Some(Identifier::new(module, synthetic_range())),
        names: aliases,
        level: 0,
        range: synthetic_range(),
        node_index: AtomicNodeIndex::dummy(),
    })
}

/// Create an empty module
pub fn empty_module() -> ModModule {
    ModModule {
        body: vec![],
        range: synthetic_range(),
        node_index: AtomicNodeIndex::dummy(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_import() {
        let stmt = import("os");
        match stmt {
            Stmt::Import(import) => {
                assert_eq!(import.names[0].name.as_str(), "os");
                assert!(import.names[0].asname.is_none());
            }
            _ => panic!("Expected Import statement"),
        }
    }

    #[test]
    fn test_import_as() {
        let stmt = import_as("numpy", "np");
        match stmt {
            Stmt::Import(import) => {
                assert_eq!(import.names[0].name.as_str(), "numpy");
                assert_eq!(import.names[0].asname.as_ref().unwrap().as_str(), "np");
            }
            _ => panic!("Expected Import statement"),
        }
    }

    #[test]
    fn test_from_import() {
        let stmt = from_import("os", &["path", "environ"]);
        match stmt {
            Stmt::ImportFrom(import) => {
                assert_eq!(import.module.as_ref().unwrap().as_str(), "os");
                assert_eq!(import.names.len(), 2);
                assert_eq!(import.names[0].name.as_str(), "path");
                assert_eq!(import.names[1].name.as_str(), "environ");
            }
            _ => panic!("Expected ImportFrom statement"),
        }
    }

    #[test]
    fn test_assign() {
        let value = name("value");
        let stmt = assign("x", value);
        match stmt {
            Stmt::Assign(assign) => {
                assert_eq!(assign.targets.len(), 1);
                match &assign.targets[0] {
                    Expr::Name(name) => assert_eq!(name.id.as_str(), "x"),
                    _ => panic!("Expected Name target"),
                }
            }
            _ => panic!("Expected Assign statement"),
        }
    }

    #[test]
    fn test_assign_attribute() {
        let value = name("value");
        let stmt = assign_attribute("obj", "attr", value);
        match stmt {
            Stmt::Assign(assign) => {
                assert_eq!(assign.targets.len(), 1);
                match &assign.targets[0] {
                    Expr::Attribute(attr) => {
                        assert_eq!(attr.attr.as_str(), "attr");
                        match &*attr.value {
                            Expr::Name(name) => assert_eq!(name.id.as_str(), "obj"),
                            _ => panic!("Expected Name value"),
                        }
                    }
                    _ => panic!("Expected Attribute target"),
                }
            }
            _ => panic!("Expected Assign statement"),
        }
    }
}

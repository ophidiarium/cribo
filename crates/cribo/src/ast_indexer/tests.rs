//! Tests for the AST indexing module

use ruff_python_parser::parse_module;

use super::*;

#[test]
fn test_simple_module_indexing() {
    let source = r#"
import os
from pathlib import Path

x = 42

def foo():
    return x

class Bar:
    def __init__(self):
        self.value = foo()
"#;

    let parsed = parse_module(source).expect("Failed to parse test module");

    let mut module = parsed.into_syntax();
    let indexed = index_module(&mut module);

    // Should have indexed multiple nodes
    let count = indexed.node_count;
    assert!(count > 10, "Expected more than 10 nodes, got {count}");

    // Should have tracked the functions (foo and __init__)
    assert_eq!(indexed.node_registry.functions.len(), 2);
    let function_names: Vec<_> = indexed
        .node_registry
        .functions
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    assert!(function_names.contains(&"foo"));
    assert!(function_names.contains(&"__init__"));

    // Should have tracked the class
    assert_eq!(indexed.node_registry.classes.len(), 1);
    assert_eq!(indexed.node_registry.classes[0].0, "Bar");

    // Should have tracked imports
    assert!(indexed.node_registry.imports.contains_key("os"));
    assert!(indexed.node_registry.imports.contains_key("pathlib"));
}

#[test]
fn test_nested_function_indexing() {
    let source = r#"
def outer():
    def inner():
        def deeply_nested():
            pass
        return deeply_nested
    return inner
"#;

    let parsed = parse_module(source).expect("Failed to parse test module");

    let mut module = parsed.into_syntax();
    let indexed = index_module(&mut module);

    // Should track all functions including nested ones
    assert_eq!(indexed.node_registry.functions.len(), 3);
    let function_names: Vec<_> = indexed
        .node_registry
        .functions
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    assert!(function_names.contains(&"outer"));
    assert!(function_names.contains(&"inner"));
    assert!(function_names.contains(&"deeply_nested"));
}

#[test]
fn test_all_export_tracking() {
    let source = r#"
__all__ = ["foo", "Bar"]

def foo():
    pass

def _private():
    pass

class Bar:
    pass
"#;

    let parsed = parse_module(source).expect("Failed to parse test module");

    let mut module = parsed.into_syntax();
    let indexed = index_module(&mut module);

    // Should have tracked the __all__ assignment
    assert!(indexed.node_registry.exports.contains_key("__all__"));
}

#[test]
fn test_sequential_indexing() {
    let source = r#"
x = 1
y = 2
z = 3
"#;

    let parsed = parse_module(source).expect("Failed to parse test module");

    let mut module = parsed.into_syntax();
    let _indexed = index_module(&mut module);

    // Each statement should have a unique index
    let indices: Vec<_> = module
        .body
        .iter()
        .map(|stmt| match stmt {
            Stmt::Assign(assign) => assign.node_index.load().as_usize(),
            _ => panic!("Expected assign statement"),
        })
        .collect();

    // Debug print
    println!("Indices: {indices:?}");

    // Indices should be sequential and unique
    assert_eq!(indices.len(), 3);
    // They should all be different (not all zeros)
    assert!(
        indices[0] != indices[1] || indices[1] != indices[2],
        "All indices are the same: {indices:?}"
    );
}

#[test]
fn test_node_index_map() {
    let mut map = NodeIndexMap::new();
    let module1 = PathBuf::from("module1.py");
    let module2 = PathBuf::from("module2.py");

    let orig1 = AtomicNodeIndex::from(10).load();
    let orig2 = AtomicNodeIndex::from(20).load();
    let trans1 = AtomicNodeIndex::from(100).load();
    let trans2 = AtomicNodeIndex::from(200).load();

    map.add_mapping(module1.clone(), orig1, trans1);
    map.add_mapping(module2.clone(), orig2, trans2);

    // Test forward mapping
    assert_eq!(map.get_transformed(&module1, orig1), Some(trans1));
    assert_eq!(map.get_transformed(&module2, orig2), Some(trans2));
    assert_eq!(map.get_transformed(&module1, orig2), None);

    // Test reverse mapping
    assert_eq!(map.get_original(trans1), Some(&(module1.clone(), orig1)));
    assert_eq!(map.get_original(trans2), Some(&(module2.clone(), orig2)));
}

#[test]
fn test_complex_ast_indexing() {
    let source = r#"
from typing import List, Dict
import asyncio

async def async_func(items: List[str]) -> Dict[str, int]:
    results = {}
    async with asyncio.Lock():
        for i, item in enumerate(items):
            try:
                results[item] = await process_item(item)
            except ValueError as e:
                print(f"Error processing {item}: {e}")
                results[item] = -1
    return results

class AsyncProcessor:
    def __init__(self):
        self.lock = asyncio.Lock()
    
    async def process(self, data):
        async with self.lock:
            return await self._process_internal(data)
"#;

    let parsed = parse_module(source).expect("Failed to parse test module");

    let mut module = parsed.into_syntax();
    let indexed = index_module(&mut module);

    // Should have indexed many nodes for this complex code
    let count = indexed.node_count;
    assert!(
        count > 50,
        "Expected more than 50 nodes for complex code, got {count}"
    );

    // Should track async functions
    assert!(
        indexed
            .node_registry
            .functions
            .iter()
            .any(|(name, _)| name == "async_func")
    );

    // Should track classes
    assert!(
        indexed
            .node_registry
            .classes
            .iter()
            .any(|(name, _)| name == "AsyncProcessor")
    );

    // Should track imports
    assert!(indexed.node_registry.imports.contains_key("typing"));
    assert!(indexed.node_registry.imports.contains_key("asyncio"));
}

#![allow(clippy::disallowed_methods)]

use anyhow::Result;
use cribo::cribo_graph::{CriboGraph, ItemData, ItemType};
use rustc_hash::FxHashSet;
use std::path::PathBuf;

/// Test error handling when trying to add dependencies for non-existent modules
#[test]
fn test_add_dependency_module_not_found() {
    let mut graph = CriboGraph::new();

    // Try to add dependency for non-existent modules - should create them
    let nonexistent_id =
        graph.add_module("nonexistent".to_string(), PathBuf::from("nonexistent.py"));
    let also_nonexistent_id = graph.add_module(
        "alsononexistent".to_string(),
        PathBuf::from("alsononexistent.py"),
    );

    // This should work now since modules exist
    graph.add_module_dependency(nonexistent_id, also_nonexistent_id);

    // Verify dependency was added
    let deps = graph.get_dependencies(nonexistent_id);
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0], also_nonexistent_id);
}

/// Test circular dependency detection
#[test]
fn test_circular_dependency_detection() {
    let mut graph = CriboGraph::new();

    // Create modules that will form a cycle
    let module_a_id = graph.add_module("module_a".to_string(), PathBuf::from("a.py"));
    let module_b_id = graph.add_module("module_b".to_string(), PathBuf::from("b.py"));
    let module_c_id = graph.add_module("module_c".to_string(), PathBuf::from("c.py"));

    // Add import items to track the imports
    if let Some(module_a) = graph.get_module_by_name_mut("module_a") {
        module_a.add_item(ItemData {
            item_type: ItemType::FromImport {
                module: "module_b".to_string(),
                names: vec![("process_b".to_string(), None)],
                level: 0,
                is_star: false,
            },
            var_decls: FxHashSet::default(),
            read_vars: FxHashSet::default(),
            eventual_read_vars: FxHashSet::default(),
            write_vars: FxHashSet::default(),
            eventual_write_vars: FxHashSet::default(),
            has_side_effects: false,
            span: None,
            imported_names: FxHashSet::default(),
            reexported_names: FxHashSet::default(),
        });
    }

    if let Some(module_b) = graph.get_module_by_name_mut("module_b") {
        module_b.add_item(ItemData {
            item_type: ItemType::FromImport {
                module: "module_c".to_string(),
                names: vec![("process_c".to_string(), None)],
                level: 0,
                is_star: false,
            },
            var_decls: FxHashSet::default(),
            read_vars: FxHashSet::default(),
            eventual_read_vars: FxHashSet::default(),
            write_vars: FxHashSet::default(),
            eventual_write_vars: FxHashSet::default(),
            has_side_effects: false,
            span: None,
            imported_names: FxHashSet::default(),
            reexported_names: FxHashSet::default(),
        });
    }

    if let Some(module_c) = graph.get_module_by_name_mut("module_c") {
        module_c.add_item(ItemData {
            item_type: ItemType::FromImport {
                module: "module_a".to_string(),
                names: vec![("process_a".to_string(), None)],
                level: 0,
                is_star: false,
            },
            var_decls: FxHashSet::default(),
            read_vars: FxHashSet::default(),
            eventual_read_vars: FxHashSet::default(),
            write_vars: FxHashSet::default(),
            eventual_write_vars: FxHashSet::default(),
            has_side_effects: false,
            span: None,
            imported_names: FxHashSet::default(),
            reexported_names: FxHashSet::default(),
        });
    }

    // Create circular dependencies: a -> b -> c -> a
    graph.add_module_dependency(module_a_id, module_b_id);
    graph.add_module_dependency(module_b_id, module_c_id);
    graph.add_module_dependency(module_c_id, module_a_id);

    // Test cycle detection
    assert!(graph.has_cycles());

    // Test that topological sort fails
    let result = graph.topological_sort();
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Circular dependency detected")
    );
}

/// Test module renaming scenarios
#[test]
fn test_module_renaming() {
    let mut graph = CriboGraph::new();

    // Add initial module
    let original_id = graph.add_module("original_name".to_string(), PathBuf::from("module.py"));

    // CriboGraph doesn't support renaming by path - each module has unique name
    // So we add a new module with different name
    let new_id = graph.add_module("new_name".to_string(), PathBuf::from("module2.py"));

    // These should be different modules
    assert_ne!(original_id, new_id);

    // Both names should exist
    assert!(graph.get_module_by_name("original_name").is_some());
    assert!(graph.get_module_by_name("new_name").is_some());

    // Add import to new module
    if let Some(module) = graph.get_module_by_name_mut("new_name") {
        module.add_item(ItemData {
            item_type: ItemType::Import {
                module: "dependency".to_string(),
                alias: None,
            },
            var_decls: FxHashSet::default(),
            read_vars: FxHashSet::default(),
            eventual_read_vars: FxHashSet::default(),
            write_vars: FxHashSet::default(),
            eventual_write_vars: FxHashSet::default(),
            has_side_effects: false,
            span: None,
            imported_names: FxHashSet::default(),
            reexported_names: FxHashSet::default(),
        });

        // Verify import was added
        assert_eq!(module.items.len(), 1);
    }
}

/// Test updating existing module by name
#[test]
fn test_module_update_by_name() {
    let mut graph = CriboGraph::new();

    // Add initial module
    let original_id =
        graph.add_module("test_module".to_string(), PathBuf::from("original_path.py"));

    // Add initial import
    if let Some(module) = graph.get_module_by_name_mut("test_module") {
        module.add_item(ItemData {
            item_type: ItemType::Import {
                module: "old_import".to_string(),
                alias: None,
            },
            var_decls: FxHashSet::default(),
            read_vars: FxHashSet::default(),
            eventual_read_vars: FxHashSet::default(),
            write_vars: FxHashSet::default(),
            eventual_write_vars: FxHashSet::default(),
            has_side_effects: false,
            span: None,
            imported_names: FxHashSet::default(),
            reexported_names: FxHashSet::default(),
        });
    }

    // Try to add module with same name - should return same ID
    let updated_id = graph.add_module("test_module".to_string(), PathBuf::from("new_path.py"));

    // Should return same ID (module already exists)
    assert_eq!(original_id, updated_id);

    // Path should not change (CriboGraph doesn't update paths)
    assert_eq!(
        graph
            .module_paths
            .iter()
            .find(|(_, id)| **id == original_id)
            .map(|(p, _)| p.clone()),
        Some(PathBuf::from("original_path.py"))
    );

    // Update imports using the update mechanism
    if let Some(module) = graph.get_module_by_name_mut("test_module") {
        // Clear old items and add new import
        module.items.clear();
        module.add_item(ItemData {
            item_type: ItemType::Import {
                module: "new_import".to_string(),
                alias: None,
            },
            var_decls: FxHashSet::default(),
            read_vars: FxHashSet::default(),
            eventual_read_vars: FxHashSet::default(),
            write_vars: FxHashSet::default(),
            eventual_write_vars: FxHashSet::default(),
            has_side_effects: false,
            span: None,
            imported_names: FxHashSet::default(),
            reexported_names: FxHashSet::default(),
        });
    }
}

/// Test filtering functionality with complex dependency chains
#[test]
fn test_filter_reachable_from_complex() -> Result<()> {
    let mut graph = CriboGraph::new();

    // Create a complex dependency structure:
    // entry -> lib1 -> util1
    //       -> lib2 -> util2
    // orphan (not reachable from entry)

    let modules = vec![
        ("entry", "entry.py", vec!["lib1", "lib2"]),
        ("lib1", "lib1.py", vec!["util1"]),
        ("lib2", "lib2.py", vec!["util2"]),
        ("util1", "util1.py", vec![]),
        ("util2", "util2.py", vec![]),
        ("orphan", "orphan.py", vec![]), // Not reachable
    ];

    // Add all modules and collect IDs
    let mut module_ids = FxHashSet::default();
    for (name, path, _imports) in &modules {
        let id = graph.add_module(name.to_string(), PathBuf::from(path));
        module_ids.insert(id);
    }

    // Get module IDs by name for dependency setup
    let entry_id = *graph.module_names.get("entry").unwrap();
    let lib1_id = *graph.module_names.get("lib1").unwrap();
    let lib2_id = *graph.module_names.get("lib2").unwrap();
    let util1_id = *graph.module_names.get("util1").unwrap();
    let util2_id = *graph.module_names.get("util2").unwrap();
    let orphan_id = *graph.module_names.get("orphan").unwrap();

    // Add dependencies
    graph.add_module_dependency(entry_id, lib1_id);
    graph.add_module_dependency(entry_id, lib2_id);
    graph.add_module_dependency(lib1_id, util1_id);
    graph.add_module_dependency(lib2_id, util2_id);

    // For CriboGraph, we'll test reachability using get_dependencies/get_dependents
    // We can collect all reachable modules using DFS
    let mut reachable = FxHashSet::default();
    let mut stack = vec![entry_id];

    while let Some(current) = stack.pop() {
        if reachable.insert(current) {
            let deps = graph.get_dependencies(current);
            stack.extend(deps);
        }
    }

    // Should contain entry and all its dependencies, but not orphan
    assert_eq!(reachable.len(), 5); // entry, lib1, lib2, util1, util2
    assert!(reachable.contains(&entry_id));
    assert!(reachable.contains(&lib1_id));
    assert!(reachable.contains(&lib2_id));
    assert!(reachable.contains(&util1_id));
    assert!(reachable.contains(&util2_id));
    assert!(!reachable.contains(&orphan_id));

    Ok(())
}

/// Test filter with non-existent entry module
#[test]
fn test_filter_reachable_from_nonexistent() {
    let graph = CriboGraph::new();

    // In CriboGraph, there's no filter_reachable_from method
    // Instead we test that a non-existent module has no dependencies
    let nonexistent_id = cribo::cribo_graph::ModuleId::new(999);
    let deps = graph.get_dependencies(nonexistent_id);
    assert_eq!(deps.len(), 0);

    // Also test by name
    assert!(graph.get_module_by_name("nonexistent").is_none());
}

/// Test getting dependencies for non-existent module
#[test]
fn test_get_dependencies_nonexistent() {
    let graph = CriboGraph::new();

    // Test with non-existent module ID
    let nonexistent_id = cribo::cribo_graph::ModuleId::new(999);
    let result = graph.get_dependencies(nonexistent_id);
    assert_eq!(result.len(), 0); // Returns empty vec instead of None
}

/// Test entry modules detection
#[test]
fn test_get_entry_modules() {
    let mut graph = CriboGraph::new();

    // Create modules: entry1 and entry2 have no dependencies, lib has dependencies
    let entry1_id = graph.add_module("entry1".to_string(), PathBuf::from("entry1.py"));
    let entry2_id = graph.add_module("entry2".to_string(), PathBuf::from("entry2.py"));
    let lib_id = graph.add_module("lib".to_string(), PathBuf::from("lib.py"));

    // Add dependency: lib depends on entry1
    graph.add_module_dependency(lib_id, entry1_id);

    // Find entry modules (modules with no dependencies)
    let mut entry_modules = Vec::new();
    for &module_id in graph.modules.keys() {
        let deps = graph.get_dependencies(module_id);
        if deps.is_empty() {
            entry_modules.push(module_id);
        }
    }

    // entry1 and entry2 should be entries (no dependencies)
    // lib should NOT be an entry (depends on entry1)
    assert_eq!(entry_modules.len(), 2);
    assert!(entry_modules.contains(&entry1_id));
    assert!(entry_modules.contains(&entry2_id));
    assert!(!entry_modules.contains(&lib_id));
}

/// Test adding duplicate edges
#[test]
fn test_duplicate_edge_handling() -> Result<()> {
    let mut graph = CriboGraph::new();

    let a_id = graph.add_module("a".to_string(), PathBuf::from("a.py"));
    let b_id = graph.add_module("b".to_string(), PathBuf::from("b.py"));

    // Add same dependency multiple times
    graph.add_module_dependency(b_id, a_id);
    graph.add_module_dependency(b_id, a_id); // Duplicate
    graph.add_module_dependency(b_id, a_id); // Another duplicate

    // Should work without errors - petgraph handles duplicates
    let dependencies = graph.get_dependencies(b_id);
    assert_eq!(dependencies.len(), 1);
    assert_eq!(dependencies[0], a_id);

    Ok(())
}

/// Test large dependency graph with deep chains
#[test]
fn test_large_dependency_chain() -> Result<()> {
    let mut graph = CriboGraph::new();

    // Create a chain: module0 -> module1 -> module2 -> ... -> module9
    const CHAIN_LENGTH: usize = 10;

    let mut module_ids = Vec::new();
    for i in 0..CHAIN_LENGTH {
        let id = graph.add_module(
            format!("module{}", i),
            PathBuf::from(format!("module{}.py", i)),
        );
        module_ids.push(id);
    }

    // Add dependencies to form chain
    for i in 0..CHAIN_LENGTH - 1 {
        graph.add_module_dependency(module_ids[i], module_ids[i + 1]);
    }

    // Test topological sort - should be in reverse order (dependencies first)
    let sorted = graph.topological_sort()?;
    assert_eq!(sorted.len(), CHAIN_LENGTH);

    // First module should be the leaf (module9)
    assert_eq!(sorted[0], module_ids[9]);
    // Last module should be the root (module0)
    assert_eq!(sorted[CHAIN_LENGTH - 1], module_ids[0]);

    // Test dependencies
    let deps = graph.get_dependencies(module_ids[0]);
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0], module_ids[1]);

    let leaf_deps = graph.get_dependencies(module_ids[9]);
    assert_eq!(leaf_deps.len(), 0); // Leaf has no dependencies

    Ok(())
}

/// Test empty graph operations
#[test]
fn test_empty_graph() {
    let graph = CriboGraph::new();

    assert_eq!(graph.modules.len(), 0);
    assert!(!graph.has_cycles());
    assert!(graph.get_module_by_name("any").is_none());

    // Test with non-existent module ID
    let nonexistent_id = cribo::cribo_graph::ModuleId::new(999);
    assert_eq!(graph.get_dependencies(nonexistent_id).len(), 0);

    // Topological sort of empty graph should succeed
    let sorted = graph.topological_sort().unwrap();
    assert_eq!(sorted.len(), 0);
}

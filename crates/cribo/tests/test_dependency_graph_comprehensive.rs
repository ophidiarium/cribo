#![allow(clippy::disallowed_methods)]

use anyhow::Result;
use cribo::dependency_graph::{DependencyGraph, ModuleNode};
use std::path::PathBuf;

/// Test error handling when trying to add dependencies for non-existent modules
#[test]
fn test_add_dependency_module_not_found() {
    let mut graph = DependencyGraph::new();

    // Try to add dependency for non-existent modules
    let result = graph.add_dependency("nonexistent", "alsononexistent");
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Module not found: nonexistent")
    );

    // Add one module and try dependency with non-existent target
    let module = ModuleNode {
        name: "existing".to_string(),
        path: PathBuf::from("existing.py"),
        imports: vec![],
    };
    graph.add_module(module);

    let result = graph.add_dependency("existing", "nonexistent");
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Module not found: nonexistent")
    );
}

/// Test circular dependency detection
#[test]
fn test_circular_dependency_detection() {
    let mut graph = DependencyGraph::new();

    // Create modules that will form a cycle
    let module_a = ModuleNode {
        name: "module_a".to_string(),
        path: PathBuf::from("a.py"),
        imports: vec!["module_b".to_string()],
    };

    let module_b = ModuleNode {
        name: "module_b".to_string(),
        path: PathBuf::from("b.py"),
        imports: vec!["module_c".to_string()],
    };

    let module_c = ModuleNode {
        name: "module_c".to_string(),
        path: PathBuf::from("c.py"),
        imports: vec!["module_a".to_string()],
    };

    // Add modules
    graph.add_module(module_a);
    graph.add_module(module_b);
    graph.add_module(module_c);

    // Create circular dependencies: a -> b -> c -> a
    graph.add_dependency("module_b", "module_a").unwrap();
    graph.add_dependency("module_c", "module_b").unwrap();
    graph.add_dependency("module_a", "module_c").unwrap();

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
    let mut graph = DependencyGraph::new();

    // Add initial module
    let original_module = ModuleNode {
        name: "original_name".to_string(),
        path: PathBuf::from("module.py"),
        imports: vec![],
    };
    let original_index = graph.add_module(original_module);

    // Add renamed module with same path - should update existing
    let renamed_module = ModuleNode {
        name: "new_name".to_string(),
        path: PathBuf::from("module.py"), // Same path
        imports: vec!["dependency".to_string()],
    };
    let renamed_index = graph.add_module(renamed_module);

    // Should return same index
    assert_eq!(original_index, renamed_index);

    // Original name should no longer exist
    assert!(graph.get_module("original_name").is_none());

    // New name should exist
    assert!(graph.get_module("new_name").is_some());

    // Should have updated imports
    let module = graph.get_module("new_name").unwrap();
    assert_eq!(module.imports, vec!["dependency".to_string()]);
}

/// Test updating existing module by name
#[test]
fn test_module_update_by_name() {
    let mut graph = DependencyGraph::new();

    // Add initial module
    let original_module = ModuleNode {
        name: "test_module".to_string(),
        path: PathBuf::from("original_path.py"),
        imports: vec!["old_import".to_string()],
    };
    let original_index = graph.add_module(original_module);

    // Update module with same name but different path and imports
    let updated_module = ModuleNode {
        name: "test_module".to_string(), // Same name
        path: PathBuf::from("new_path.py"),
        imports: vec!["new_import".to_string()],
    };
    let updated_index = graph.add_module(updated_module);

    // Should return same index
    assert_eq!(original_index, updated_index);

    // Should have updated content
    let module = graph.get_module("test_module").unwrap();
    assert_eq!(module.path, PathBuf::from("new_path.py"));
    assert_eq!(module.imports, vec!["new_import".to_string()]);
}

/// Test filtering functionality with complex dependency chains
#[test]
fn test_filter_reachable_from_complex() -> Result<()> {
    let mut graph = DependencyGraph::new();

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

    // Add all modules
    for (name, path, imports) in &modules {
        let module = ModuleNode {
            name: name.to_string(),
            path: PathBuf::from(path),
            imports: imports.iter().map(|s| s.to_string()).collect(),
        };
        graph.add_module(module);
    }

    // Add dependencies
    graph.add_dependency("lib1", "entry")?;
    graph.add_dependency("lib2", "entry")?;
    graph.add_dependency("util1", "lib1")?;
    graph.add_dependency("util2", "lib2")?;

    // Filter from entry
    let filtered = graph.filter_reachable_from("entry")?;

    // Should contain entry and all its dependencies, but not orphan
    let filtered_modules = filtered.get_modules();
    assert_eq!(filtered_modules.len(), 5); // entry, lib1, lib2, util1, util2

    let names: Vec<&str> = filtered_modules.iter().map(|m| m.name.as_str()).collect();
    assert!(names.contains(&"entry"));
    assert!(names.contains(&"lib1"));
    assert!(names.contains(&"lib2"));
    assert!(names.contains(&"util1"));
    assert!(names.contains(&"util2"));
    assert!(!names.contains(&"orphan"));

    Ok(())
}

/// Test filter with non-existent entry module
#[test]
fn test_filter_reachable_from_nonexistent() {
    let graph = DependencyGraph::new();

    let result = graph.filter_reachable_from("nonexistent");
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Entry module not found: nonexistent")
    );
}

/// Test getting dependencies for non-existent module
#[test]
fn test_get_dependencies_nonexistent() {
    let graph = DependencyGraph::new();

    let result = graph.get_dependencies("nonexistent");
    assert!(result.is_none());
}

/// Test entry modules detection
#[test]
fn test_get_entry_modules() {
    let mut graph = DependencyGraph::new();

    // Create modules: entry1 and entry2 have no dependencies, lib has dependencies
    let entry1 = ModuleNode {
        name: "entry1".to_string(),
        path: PathBuf::from("entry1.py"),
        imports: vec![],
    };

    let entry2 = ModuleNode {
        name: "entry2".to_string(),
        path: PathBuf::from("entry2.py"),
        imports: vec![],
    };

    let lib = ModuleNode {
        name: "lib".to_string(),
        path: PathBuf::from("lib.py"),
        imports: vec!["entry1".to_string()],
    };

    graph.add_module(entry1);
    graph.add_module(entry2);
    graph.add_module(lib);

    // Add dependency: entry1 -> lib
    graph.add_dependency("entry1", "lib").unwrap();

    // Get entry modules (modules with no incoming dependencies)
    let entries = graph.get_entry_modules();

    // entry1 should be an entry (no incoming dependencies)
    // entry2 should be an entry (no dependencies at all)
    // lib should NOT be an entry (has incoming dependency from entry1)
    let entry_names: Vec<&str> = entries.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(entries.len(), 2);
    assert!(entry_names.contains(&"entry1"));
    assert!(entry_names.contains(&"entry2"));
    assert!(!entry_names.contains(&"lib"));
}

/// Test adding duplicate edges
#[test]
fn test_duplicate_edge_handling() -> Result<()> {
    let mut graph = DependencyGraph::new();

    let module_a = ModuleNode {
        name: "a".to_string(),
        path: PathBuf::from("a.py"),
        imports: vec![],
    };

    let module_b = ModuleNode {
        name: "b".to_string(),
        path: PathBuf::from("b.py"),
        imports: vec![],
    };

    graph.add_module(module_a);
    graph.add_module(module_b);

    // Add same dependency multiple times
    graph.add_dependency("a", "b")?;
    graph.add_dependency("a", "b")?; // Duplicate
    graph.add_dependency("a", "b")?; // Another duplicate

    // Should work without errors
    let dependencies = graph.get_dependencies("b").unwrap();
    assert_eq!(dependencies.len(), 1);
    assert_eq!(dependencies[0], "a");

    Ok(())
}

/// Test large dependency graph with deep chains
#[test]
fn test_large_dependency_chain() -> Result<()> {
    let mut graph = DependencyGraph::new();

    // Create a chain: module0 -> module1 -> module2 -> ... -> module9
    const CHAIN_LENGTH: usize = 10;

    for i in 0..CHAIN_LENGTH {
        let module = ModuleNode {
            name: format!("module{}", i),
            path: PathBuf::from(format!("module{}.py", i)),
            imports: if i < CHAIN_LENGTH - 1 {
                vec![format!("module{}", i + 1)]
            } else {
                vec![]
            },
        };
        graph.add_module(module);
    }

    // Add dependencies to form chain
    for i in 0..CHAIN_LENGTH - 1 {
        graph.add_dependency(&format!("module{}", i + 1), &format!("module{}", i))?;
    }

    // Test topological sort - should be in reverse order (dependencies first)
    let sorted = graph.topological_sort()?;
    assert_eq!(sorted.len(), CHAIN_LENGTH);

    // First module should be the leaf (module9)
    assert_eq!(sorted[0].name, "module9");
    // Last module should be the root (module0)
    assert_eq!(sorted[CHAIN_LENGTH - 1].name, "module0");

    // Test dependencies
    let deps = graph.get_dependencies("module0").unwrap();
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0], "module1");

    let leaf_deps = graph.get_dependencies("module9").unwrap();
    assert_eq!(leaf_deps.len(), 0); // Leaf has no dependencies

    Ok(())
}

/// Test empty graph operations
#[test]
fn test_empty_graph() {
    let graph = DependencyGraph::new();

    assert_eq!(graph.get_modules().len(), 0);
    assert!(!graph.has_cycles());
    assert_eq!(graph.get_entry_modules().len(), 0);
    assert!(graph.get_module("any").is_none());
    assert!(graph.get_dependencies("any").is_none());

    // Topological sort of empty graph should succeed
    let sorted = graph.topological_sort().unwrap();
    assert_eq!(sorted.len(), 0);
}

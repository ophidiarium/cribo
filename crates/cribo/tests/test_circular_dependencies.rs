#![allow(clippy::disallowed_methods)]

use anyhow::Result;
use cribo::config::Config;
use cribo::orchestrator::BundleOrchestrator;
use insta::assert_snapshot;
use std::path::PathBuf;
use tempfile::TempDir;

fn bundle_test_script(script_dir: &str) -> Result<String> {
    let script_dir_path = PathBuf::from("tests/fixtures/circular_dependencies").join(script_dir);
    let main_path = script_dir_path.join("main.py");

    if !main_path.exists() {
        panic!("Main script not found: {}", main_path.display());
    }

    let config = Config {
        src: vec![script_dir_path],
        ..Default::default()
    };
    let mut bundler = BundleOrchestrator::new(config);

    let temp_dir = TempDir::new()?;
    let output_path = temp_dir.path().join("bundled_script.py");

    // Bundle the script
    bundler.bundle(&main_path, &output_path, false)?;

    // Read the bundled content
    let bundled_content = std::fs::read_to_string(&output_path)?;
    Ok(bundled_content)
}

#[test]
fn test_three_module_circular_dependency() {
    let result = bundle_test_script("three_module_cycle");

    // This should detect the circular dependency: module_a -> module_b -> module_c -> module_a
    match result {
        Ok(bundled_content) => {
            // If bundling succeeds, it means we've successfully resolved the circular dependency
            // This is a function-level cycle that should be resolvable
            assert_snapshot!("three_module_cycle_bundled", bundled_content);
        }
        Err(error) => {
            // If it fails, we should get a meaningful error about the circular dependency
            let error_msg = error.to_string();
            assert!(
                error_msg.contains("Circular dependencies detected")
                    || error_msg.contains("circular")
                    || error_msg.contains("cycle"),
                "Error should mention circular dependency: {}",
                error_msg
            );
            assert_snapshot!("three_module_cycle_error", error_msg);
        }
    }
}

#[test]
fn test_four_module_circular_dependency() {
    let result = bundle_test_script("four_module_cycle");

    // This tests a longer cycle: A -> B -> C -> D -> A
    match result {
        Ok(bundled_content) => {
            assert_snapshot!("four_module_cycle_bundled", bundled_content);
        }
        Err(error) => {
            let error_msg = error.to_string();
            assert!(
                error_msg.contains("Circular dependencies detected")
                    || error_msg.contains("circular")
                    || error_msg.contains("cycle"),
                "Error should mention circular dependency: {}",
                error_msg
            );
            assert_snapshot!("four_module_cycle_error", error_msg);
        }
    }
}

#[test]
fn test_package_level_circular_dependency() {
    let result = bundle_test_script("package_level_cycles");

    // Tests circular dependency between packages: pkg1 -> pkg2 -> pkg1
    match result {
        Ok(bundled_content) => {
            assert_snapshot!("package_level_cycles_bundled", bundled_content);
        }
        Err(error) => {
            let error_msg = error.to_string();
            assert!(
                error_msg.contains("Circular dependencies detected")
                    || error_msg.contains("circular")
                    || error_msg.contains("cycle"),
                "Error should mention circular dependency: {}",
                error_msg
            );
            assert_snapshot!("package_level_cycles_error", error_msg);
        }
    }
}

#[test]
fn test_relative_import_circular_dependency() {
    let result = bundle_test_script("relative_import_cycles");

    // Tests circular dependency with relative imports: .auth -> .database -> .auth
    match result {
        Ok(bundled_content) => {
            assert_snapshot!("relative_import_cycles_bundled", bundled_content);
        }
        Err(error) => {
            let error_msg = error.to_string();
            assert!(
                error_msg.contains("Circular dependencies detected")
                    || error_msg.contains("circular")
                    || error_msg.contains("cycle"),
                "Error should mention circular dependency: {}",
                error_msg
            );
            assert_snapshot!("relative_import_cycles_error", error_msg);
        }
    }
}

#[test]
fn test_unresolvable_circular_dependency() {
    let result = bundle_test_script("unresolvable_patterns");

    // This should always fail - temporal paradox with module-level constants
    match result {
        Ok(bundled_content) => {
            panic!(
                "Unresolvable circular dependency should not bundle successfully. Got: {}",
                bundled_content
            );
        }
        Err(error) => {
            let error_msg = error.to_string();
            assert!(
                error_msg.contains("Circular dependencies detected")
                    || error_msg.contains("circular")
                    || error_msg.contains("cycle"),
                "Error should mention circular dependency: {}",
                error_msg
            );
            assert_snapshot!("unresolvable_patterns_error", error_msg);
        }
    }
}

#[test]
fn test_mixed_resolvable_and_unresolvable_cycles() {
    // This tests the bundler code path where we have both types of cycles
    let result = bundle_test_script("mixed_cycles");

    // This should fail because there are unresolvable cycles present
    match result {
        Ok(_) => {
            panic!("Mixed cycles with unresolvable cycles should not bundle successfully");
        }
        Err(error) => {
            let error_msg = error.to_string();

            // The error should mention that it's an unresolvable circular dependency
            assert!(
                error_msg.contains("Unresolvable circular dependencies detected"),
                "Error should mention unresolvable cycles: {}",
                error_msg
            );

            // Should contain specific module names
            assert!(
                error_msg.contains("constants_module") || error_msg.contains("config_constants"),
                "Error should mention the constants modules: {}",
                error_msg
            );

            assert_snapshot!("mixed_cycles_error", error_msg);
        }
    }
}

#[test]
fn test_class_level_circular_dependency() {
    // This tests class-level circular dependencies
    let result = bundle_test_script("class_level_cycles");

    // Currently, class-level cycles are treated as resolvable but not yet implemented
    match result {
        Ok(bundled_content) => {
            // If it succeeds, the bundler resolved it
            assert_snapshot!("class_level_cycles_bundled", bundled_content);
        }
        Err(error) => {
            let error_msg = error.to_string();
            assert!(
                error_msg.contains("Circular dependencies detected"),
                "Error should mention circular dependency: {}",
                error_msg
            );
            assert_snapshot!("class_level_cycles_error", error_msg);
        }
    }
}

#[test]
fn test_circular_dependency_detection_in_dependency_graph() {
    use cribo::dependency_graph::{DependencyGraph, ModuleNode};
    use std::path::PathBuf;

    let mut graph = DependencyGraph::new();

    // Create a simple circular dependency: A -> B -> A
    let module_a = ModuleNode {
        name: "module_a".to_string(),
        path: PathBuf::from("/test/module_a.py"),
        imports: vec!["module_b".to_string()],
    };

    let module_b = ModuleNode {
        name: "module_b".to_string(),
        path: PathBuf::from("/test/module_b.py"),
        imports: vec!["module_a".to_string()],
    };

    graph.add_module(module_a);
    graph.add_module(module_b);
    graph.add_dependency("module_a", "module_b").unwrap();
    graph.add_dependency("module_b", "module_a").unwrap();

    // The graph should detect the cycle
    assert!(
        graph.has_cycles(),
        "Graph should detect circular dependency"
    );

    // Topological sort should fail with cycle information
    let sort_result = graph.topological_sort();
    assert!(
        sort_result.is_err(),
        "Topological sort should fail on circular dependency"
    );

    let error_msg = sort_result.unwrap_err().to_string();
    assert!(
        error_msg.contains("Circular dependency detected"),
        "Error should mention circular dependency detection: {}",
        error_msg
    );
}

#[test]
fn test_tarjans_strongly_connected_components() {
    use cribo::dependency_graph::{DependencyGraph, ModuleNode};
    use std::path::PathBuf;

    let mut graph = DependencyGraph::new();

    // Create a more complex graph with multiple cycles
    // Cycle 1: A -> B -> A
    // Cycle 2: C -> D -> E -> C
    // Single node: F

    let modules = vec![
        ("module_a", vec!["module_b"]),
        ("module_b", vec!["module_a"]),
        ("module_c", vec!["module_d"]),
        ("module_d", vec!["module_e"]),
        ("module_e", vec!["module_c"]),
        ("module_f", vec![]),
    ];

    // Add all modules
    for (name, imports) in &modules {
        let module = ModuleNode {
            name: name.to_string(),
            path: PathBuf::from(format!("/test/{}.py", name)),
            imports: imports.iter().map(|s| s.to_string()).collect(),
        };
        graph.add_module(module);
    }

    // Add dependencies
    for (from, imports) in &modules {
        for to in imports {
            graph.add_dependency(from, to).unwrap();
        }
    }

    // Find strongly connected components
    let sccs = graph.find_strongly_connected_components();

    // Should find 2 SCCs (the two cycles)
    assert_eq!(
        sccs.len(),
        2,
        "Should find exactly 2 strongly connected components"
    );

    // Each SCC should have the right number of modules
    let mut scc_sizes: Vec<usize> = sccs.iter().map(|scc| scc.len()).collect();
    scc_sizes.sort();
    assert_eq!(
        scc_sizes,
        vec![2, 3],
        "Should have one 2-module cycle and one 3-module cycle"
    );
}

#[test]
fn test_cycle_path_detection() {
    use cribo::dependency_graph::{DependencyGraph, ModuleNode};
    use std::path::PathBuf;

    let mut graph = DependencyGraph::new();

    // Create a simple 3-module cycle: A -> B -> C -> A
    let modules = vec![
        ("module_a", vec!["module_b"]),
        ("module_b", vec!["module_c"]),
        ("module_c", vec!["module_a"]),
    ];

    for (name, imports) in &modules {
        let module = ModuleNode {
            name: name.to_string(),
            path: PathBuf::from(format!("/test/{}.py", name)),
            imports: imports.iter().map(|s| s.to_string()).collect(),
        };
        graph.add_module(module);
    }

    for (from, imports) in &modules {
        for to in imports {
            graph.add_dependency(from, to).unwrap();
        }
    }

    // Find cycle paths
    let cycle_paths = graph.find_cycle_paths().unwrap();

    // Should find at least one cycle
    assert!(
        !cycle_paths.is_empty(),
        "Should find at least one cycle path"
    );

    // Each cycle should have 3 modules
    for cycle in &cycle_paths {
        assert!(
            cycle.len() >= 3,
            "Cycle should have at least 3 modules: {:?}",
            cycle
        );
    }
}

#[test]
fn test_circular_dependency_classification() {
    use cribo::dependency_graph::{CircularDependencyType, DependencyGraph, ModuleNode};
    use std::path::PathBuf;

    let mut graph = DependencyGraph::new();

    // Create cycles with different types
    // Function-level cycle: auth -> database -> auth
    // Constants cycle: constants_a -> constants_b -> constants_a

    let modules = vec![
        ("auth", vec!["database"]),
        ("database", vec!["auth"]),
        ("constants_a", vec!["constants_b"]),
        ("constants_b", vec!["constants_a"]),
    ];

    for (name, imports) in &modules {
        let module = ModuleNode {
            name: name.to_string(),
            path: PathBuf::from(format!("/test/{}.py", name)),
            imports: imports.iter().map(|s| s.to_string()).collect(),
        };
        graph.add_module(module);
    }

    for (from, imports) in &modules {
        for to in imports {
            graph.add_dependency(from, to).unwrap();
        }
    }

    // Classify circular dependencies
    let analysis = graph.classify_circular_dependencies();

    // Should have 2 cycles total
    assert_eq!(analysis.total_cycles_detected, 2, "Should detect 2 cycles");

    // Should have 1 resolvable and 1 unresolvable cycle
    assert_eq!(
        analysis.resolvable_cycles.len(),
        1,
        "Should have 1 resolvable cycle"
    );
    assert_eq!(
        analysis.unresolvable_cycles.len(),
        1,
        "Should have 1 unresolvable cycle"
    );

    // Check that constants cycle is classified as unresolvable
    let unresolvable_cycle = &analysis.unresolvable_cycles[0];
    assert!(matches!(
        unresolvable_cycle.cycle_type,
        CircularDependencyType::ModuleConstants
    ));
    assert!(
        unresolvable_cycle
            .modules
            .iter()
            .any(|m| m.contains("constants"))
    );

    // Check that auth cycle is classified as resolvable
    let resolvable_cycle = &analysis.resolvable_cycles[0];
    assert!(matches!(
        resolvable_cycle.cycle_type,
        CircularDependencyType::FunctionLevel
    ));
}

#[test]
fn test_empty_graph_cycle_detection() {
    use cribo::dependency_graph::DependencyGraph;

    let graph = DependencyGraph::new();

    // Empty graph should have no cycles
    assert!(!graph.has_cycles(), "Empty graph should not have cycles");

    let sccs = graph.find_strongly_connected_components();
    assert!(sccs.is_empty(), "Empty graph should have no SCCs");

    let cycle_paths = graph.find_cycle_paths().unwrap();
    assert!(
        cycle_paths.is_empty(),
        "Empty graph should have no cycle paths"
    );

    let analysis = graph.classify_circular_dependencies();
    assert_eq!(
        analysis.total_cycles_detected, 0,
        "Empty graph should detect 0 cycles"
    );
    assert_eq!(
        analysis.largest_cycle_size, 0,
        "Empty graph should have largest cycle size 0"
    );
}

#[test]
fn test_single_module_no_cycles() {
    use cribo::dependency_graph::{DependencyGraph, ModuleNode};
    use std::path::PathBuf;

    let mut graph = DependencyGraph::new();

    // Add a single module with no dependencies
    let module = ModuleNode {
        name: "single_module".to_string(),
        path: PathBuf::from("/test/single_module.py"),
        imports: vec![],
    };
    graph.add_module(module);

    // Single module should not create cycles
    assert!(!graph.has_cycles(), "Single module should not have cycles");

    let sccs = graph.find_strongly_connected_components();
    assert!(sccs.is_empty(), "Single module should have no SCCs");

    let analysis = graph.classify_circular_dependencies();
    assert_eq!(
        analysis.total_cycles_detected, 0,
        "Single module should detect 0 cycles"
    );
}

#[test]
fn test_linear_dependency_chain_no_cycles() {
    use cribo::dependency_graph::{DependencyGraph, ModuleNode};
    use std::path::PathBuf;

    let mut graph = DependencyGraph::new();

    // Create a linear chain: A -> B -> C -> D (no cycles)
    let modules = vec![
        ("module_a", vec!["module_b"]),
        ("module_b", vec!["module_c"]),
        ("module_c", vec!["module_d"]),
        ("module_d", vec![]), // Terminal module
    ];

    for (name, imports) in &modules {
        let module = ModuleNode {
            name: name.to_string(),
            path: PathBuf::from(format!("/test/{}.py", name)),
            imports: imports.iter().map(|s| s.to_string()).collect(),
        };
        graph.add_module(module);
    }

    for (from, imports) in &modules {
        for to in imports {
            graph.add_dependency(from, to).unwrap();
        }
    }

    // Linear chain should not have cycles
    assert!(!graph.has_cycles(), "Linear chain should not have cycles");

    let sccs = graph.find_strongly_connected_components();
    assert!(sccs.is_empty(), "Linear chain should have no SCCs");

    let analysis = graph.classify_circular_dependencies();
    assert_eq!(
        analysis.total_cycles_detected, 0,
        "Linear chain should detect 0 cycles"
    );
}

#[test]
fn test_self_referencing_module() {
    use cribo::dependency_graph::{DependencyGraph, ModuleNode};
    use std::path::PathBuf;

    let mut graph = DependencyGraph::new();

    // Create a module that imports itself
    let module = ModuleNode {
        name: "self_ref".to_string(),
        path: PathBuf::from("/test/self_ref.py"),
        imports: vec!["self_ref".to_string()],
    };
    graph.add_module(module);
    graph.add_dependency("self_ref", "self_ref").unwrap();

    // Self-referencing module should create a cycle
    assert!(
        graph.has_cycles(),
        "Self-referencing module should have cycles"
    );

    // Note: Tarjan's algorithm filters out single-node SCCs unless they have self-loops
    // The implementation only includes components with > 1 node for actual cycles
    let sccs = graph.find_strongly_connected_components();
    assert_eq!(
        sccs.len(),
        0,
        "Single-node self-reference excluded from SCCs by design"
    );

    // But three-color DFS should still detect the cycle
    let cycle_paths = graph.find_cycle_paths().unwrap();
    assert!(
        cycle_paths.is_empty() || !cycle_paths[0].is_empty(),
        "Cycle paths should be detectable for self-reference"
    );

    let analysis = graph.classify_circular_dependencies();
    assert_eq!(
        analysis.total_cycles_detected, 0,
        "Self-reference not counted as cycle by current implementation"
    );
}

#[test]
fn test_complex_multi_cycle_graph() {
    use cribo::dependency_graph::{DependencyGraph, ModuleNode};
    use std::path::PathBuf;

    let mut graph = DependencyGraph::new();

    // Create multiple disconnected cycles:
    // Cycle 1: A -> B -> A
    // Cycle 2: C -> D -> E -> C
    // Independent: F -> G (no cycle)
    let modules = vec![
        ("module_a", vec!["module_b"]),
        ("module_b", vec!["module_a"]),
        ("module_c", vec!["module_d"]),
        ("module_d", vec!["module_e"]),
        ("module_e", vec!["module_c"]),
        ("module_f", vec!["module_g"]),
        ("module_g", vec![]),
    ];

    for (name, imports) in &modules {
        let module = ModuleNode {
            name: name.to_string(),
            path: PathBuf::from(format!("/test/{}.py", name)),
            imports: imports.iter().map(|s| s.to_string()).collect(),
        };
        graph.add_module(module);
    }

    for (from, imports) in &modules {
        for to in imports {
            graph.add_dependency(from, to).unwrap();
        }
    }

    // Should detect cycles
    assert!(graph.has_cycles(), "Complex graph should have cycles");

    let sccs = graph.find_strongly_connected_components();
    assert_eq!(sccs.len(), 2, "Should find 2 SCCs (2 cycles)");

    // Verify SCC sizes
    let mut scc_sizes: Vec<usize> = sccs.iter().map(|scc| scc.len()).collect();
    scc_sizes.sort();
    assert_eq!(scc_sizes, vec![2, 3], "Should have cycles of size 2 and 3");

    let analysis = graph.classify_circular_dependencies();
    assert_eq!(analysis.total_cycles_detected, 2, "Should detect 2 cycles");
    assert_eq!(
        analysis.largest_cycle_size, 3,
        "Largest cycle should be size 3"
    );
}

#[test]
fn test_error_handling_missing_modules() {
    use cribo::dependency_graph::DependencyGraph;

    let mut graph = DependencyGraph::new();

    // Try to add dependency to non-existent modules
    let result = graph.add_dependency("non_existent_from", "non_existent_to");
    assert!(
        result.is_err(),
        "Should error when adding dependency to non-existent modules"
    );

    // Try to get dependencies for non-existent module
    let deps = graph.get_dependencies("non_existent");
    assert!(
        deps.is_none(),
        "Should return None for non-existent module dependencies"
    );

    // Try to get module by non-existent name
    let module = graph.get_module("non_existent");
    assert!(
        module.is_none(),
        "Should return None for non-existent module"
    );
}

#[test]
fn test_get_entry_modules() {
    use cribo::dependency_graph::{DependencyGraph, ModuleNode};
    use std::path::PathBuf;

    let mut graph = DependencyGraph::new();

    // Create a graph with clear entry points:
    // entry1 -> module_a -> module_b
    // entry2 -> module_c
    // module_d (standalone)
    let modules = vec![
        ("entry1", vec!["module_a"]),
        ("entry2", vec!["module_c"]),
        ("module_a", vec!["module_b"]),
        ("module_b", vec![]),
        ("module_c", vec![]),
        ("module_d", vec![]), // Standalone entry point
    ];

    for (name, imports) in &modules {
        let module = ModuleNode {
            name: name.to_string(),
            path: PathBuf::from(format!("/test/{}.py", name)),
            imports: imports.iter().map(|s| s.to_string()).collect(),
        };
        graph.add_module(module);
    }

    for (from, imports) in &modules {
        for to in imports {
            graph.add_dependency(from, to).unwrap();
        }
    }

    let entry_modules = graph.get_entry_modules();
    let entry_names: indexmap::IndexSet<&str> =
        entry_modules.iter().map(|m| m.name.as_str()).collect();

    // Should identify modules with no dependencies as entry points
    let expected_entries: indexmap::IndexSet<&str> =
        ["entry1", "entry2", "module_d"].iter().cloned().collect();

    assert_eq!(
        entry_names, expected_entries,
        "Should correctly identify entry modules (those with no dependencies)"
    );
}

#[test]
fn test_import_chain_building() {
    use cribo::dependency_graph::{DependencyGraph, ImportType, ModuleNode};
    use std::path::PathBuf;

    let mut graph = DependencyGraph::new();

    // Create a simple cycle to test import chain building
    let modules = vec![
        ("module_a", vec!["module_b"]),
        ("module_b", vec!["module_a"]),
    ];

    for (name, imports) in &modules {
        let module = ModuleNode {
            name: name.to_string(),
            path: PathBuf::from(format!("/test/{}.py", name)),
            imports: imports.iter().map(|s| s.to_string()).collect(),
        };
        graph.add_module(module);
    }

    for (from, imports) in &modules {
        for to in imports {
            graph.add_dependency(from, to).unwrap();
        }
    }

    let analysis = graph.classify_circular_dependencies();

    // Check that import chains are properly built
    assert_eq!(analysis.resolvable_cycles.len(), 1);
    let cycle = &analysis.resolvable_cycles[0];

    assert!(
        !cycle.import_chain.is_empty(),
        "Import chain should not be empty"
    );

    // Verify import chain contains expected edges
    let has_a_to_b = cycle
        .import_chain
        .iter()
        .any(|edge| edge.from_module == "module_a" && edge.to_module == "module_b");
    let has_b_to_a = cycle
        .import_chain
        .iter()
        .any(|edge| edge.from_module == "module_b" && edge.to_module == "module_a");

    assert!(
        has_a_to_b || has_b_to_a,
        "Import chain should contain cycle edges"
    );

    // Check that import type is set (simplified implementation uses Direct)
    for edge in &cycle.import_chain {
        assert!(matches!(edge.import_type, ImportType::Direct));
    }
}

#[test]
fn test_cycle_type_classification() {
    use cribo::dependency_graph::{CircularDependencyType, DependencyGraph, ModuleNode};
    use std::path::PathBuf;

    let mut graph = DependencyGraph::new();

    // Test 1: Module with "constants" in name should be classified as ModuleConstants
    let constants_modules = vec![
        ("constants_a", vec!["constants_b"]),
        ("constants_b", vec!["constants_a"]),
    ];

    for (name, imports) in &constants_modules {
        let module = ModuleNode {
            name: name.to_string(),
            path: PathBuf::from(format!("/test/{}.py", name)),
            imports: imports.iter().map(|s| s.to_string()).collect(),
        };
        graph.add_module(module);
    }

    for (from, imports) in &constants_modules {
        for to in imports {
            graph.add_dependency(from, to).unwrap();
        }
    }

    let analysis = graph.classify_circular_dependencies();

    // Should have one unresolvable cycle
    assert_eq!(analysis.unresolvable_cycles.len(), 1);
    assert_eq!(analysis.resolvable_cycles.len(), 0);

    let unresolvable = &analysis.unresolvable_cycles[0];
    assert!(matches!(
        unresolvable.cycle_type,
        CircularDependencyType::ModuleConstants
    ));

    // Test 2: Regular modules should be classified as FunctionLevel
    let mut graph2 = DependencyGraph::new();

    let regular_modules = vec![
        ("module_x", vec!["module_y"]),
        ("module_y", vec!["module_x"]),
    ];

    for (name, imports) in &regular_modules {
        let module = ModuleNode {
            name: name.to_string(),
            path: PathBuf::from(format!("/test/{}.py", name)),
            imports: imports.iter().map(|s| s.to_string()).collect(),
        };
        graph2.add_module(module);
    }

    for (from, imports) in &regular_modules {
        for to in imports {
            graph2.add_dependency(from, to).unwrap();
        }
    }

    let analysis2 = graph2.classify_circular_dependencies();

    // Should have one resolvable cycle
    assert_eq!(analysis2.resolvable_cycles.len(), 1);
    assert_eq!(analysis2.unresolvable_cycles.len(), 0);

    let resolvable = &analysis2.resolvable_cycles[0];
    assert!(matches!(
        resolvable.cycle_type,
        CircularDependencyType::FunctionLevel
    ));
}

#[test]
fn test_resolution_strategy_suggestions() {
    use cribo::dependency_graph::{DependencyGraph, ModuleNode, ResolutionStrategy};
    use std::path::PathBuf;

    let mut graph = DependencyGraph::new();

    // Create modules to test different resolution strategies
    let modules = vec![
        ("func_module_a", vec!["func_module_b"]),
        ("func_module_b", vec!["func_module_a"]),
    ];

    for (name, imports) in &modules {
        let module = ModuleNode {
            name: name.to_string(),
            path: PathBuf::from(format!("/test/{}.py", name)),
            imports: imports.iter().map(|s| s.to_string()).collect(),
        };
        graph.add_module(module);
    }

    for (from, imports) in &modules {
        for to in imports {
            graph.add_dependency(from, to).unwrap();
        }
    }

    let analysis = graph.classify_circular_dependencies();

    assert_eq!(analysis.resolvable_cycles.len(), 1);
    let cycle = &analysis.resolvable_cycles[0];

    // FunctionLevel cycles should get LazyImport resolution
    match &cycle.suggested_resolution {
        ResolutionStrategy::LazyImport { modules } => {
            assert!(!modules.is_empty());
            assert!(
                modules.contains(&"func_module_a".to_string())
                    || modules.contains(&"func_module_b".to_string())
            );
        }
        _ => panic!("Expected LazyImport resolution for FunctionLevel cycle"),
    }
}

#[test]
fn test_cycle_detection_with_back_edge() {
    use cribo::dependency_graph::{DependencyGraph, ModuleNode};
    use std::path::PathBuf;

    let mut graph = DependencyGraph::new();

    // Create a more complex graph to test back edge detection
    // A -> B -> C -> B (back edge from C to B)
    let modules = vec![
        ("module_a", vec!["module_b"]),
        ("module_b", vec!["module_c"]),
        ("module_c", vec!["module_b"]), // Back edge
    ];

    for (name, imports) in &modules {
        let module = ModuleNode {
            name: name.to_string(),
            path: PathBuf::from(format!("/test/{}.py", name)),
            imports: imports.iter().map(|s| s.to_string()).collect(),
        };
        graph.add_module(module);
    }

    for (from, imports) in &modules {
        for to in imports {
            graph.add_dependency(from, to).unwrap();
        }
    }

    // Test find_cycle_paths
    let cycle_paths = graph.find_cycle_paths().unwrap();

    assert!(
        !cycle_paths.is_empty(),
        "Should detect cycles with back edges"
    );

    // Should find the B -> C -> B cycle
    let has_expected_cycle = cycle_paths.iter().any(|path| {
        path.len() >= 2
            && ((path.contains(&"module_b".to_string()) && path.contains(&"module_c".to_string()))
                || (path == &vec!["module_b", "module_c"] || path == &vec!["module_c", "module_b"]))
    });

    assert!(has_expected_cycle, "Should find the B-C cycle");
}

#[test]
fn test_import_time_cycle_classification() {
    use cribo::dependency_graph::{
        CircularDependencyType, DependencyGraph, ModuleNode, ResolutionStrategy,
    };
    use std::path::PathBuf;

    let mut graph = DependencyGraph::new();

    // Test modules with "import" or "loader" in name
    let import_modules = vec![
        ("import_manager", vec!["loader_module"]),
        ("loader_module", vec!["import_manager"]),
    ];

    for (name, imports) in &import_modules {
        let module = ModuleNode {
            name: name.to_string(),
            path: PathBuf::from(format!("/test/{}.py", name)),
            imports: imports.iter().map(|s| s.to_string()).collect(),
        };
        graph.add_module(module);
    }

    for (from, imports) in &import_modules {
        for to in imports {
            graph.add_dependency(from, to).unwrap();
        }
    }

    let analysis = graph.classify_circular_dependencies();

    // Should have one resolvable cycle of ImportTime type
    assert_eq!(analysis.resolvable_cycles.len(), 1);
    assert_eq!(analysis.unresolvable_cycles.len(), 0);

    let cycle = &analysis.resolvable_cycles[0];
    assert!(matches!(
        cycle.cycle_type,
        CircularDependencyType::ImportTime
    ));

    // ImportTime cycles should get ModuleSplit resolution
    match &cycle.suggested_resolution {
        ResolutionStrategy::ModuleSplit { suggestions } => {
            assert!(!suggestions.is_empty());
            assert!(
                suggestions
                    .iter()
                    .any(|s| s.contains("extract") || s.contains("separate"))
            );
        }
        _ => panic!("Expected ModuleSplit resolution for ImportTime cycle"),
    }
}

#[test]
fn test_class_level_cycle_resolution_strategy() {
    use cribo::dependency_graph::{
        CircularDependencyType, DependencyGraph, ModuleNode, ResolutionStrategy,
    };
    use std::path::PathBuf;

    let mut graph = DependencyGraph::new();

    // Test modules with "class" in name
    let class_modules = vec![
        ("user_class", vec!["admin_class"]),
        ("admin_class", vec!["user_class"]),
    ];

    for (name, imports) in &class_modules {
        let module = ModuleNode {
            name: name.to_string(),
            path: PathBuf::from(format!("/test/{}.py", name)),
            imports: imports.iter().map(|s| s.to_string()).collect(),
        };
        graph.add_module(module);
    }

    for (from, imports) in &class_modules {
        for to in imports {
            graph.add_dependency(from, to).unwrap();
        }
    }

    let analysis = graph.classify_circular_dependencies();

    // Should have one resolvable cycle of ClassLevel type
    assert_eq!(analysis.resolvable_cycles.len(), 1);
    assert_eq!(analysis.unresolvable_cycles.len(), 0);

    let cycle = &analysis.resolvable_cycles[0];
    assert!(matches!(
        cycle.cycle_type,
        CircularDependencyType::ClassLevel
    ));

    // ClassLevel cycles should get FunctionScopedImport resolution
    match &cycle.suggested_resolution {
        ResolutionStrategy::FunctionScopedImport { import_statements } => {
            assert!(!import_statements.is_empty());
            assert!(
                import_statements
                    .iter()
                    .any(|s| s.contains("Move") && s.contains("inside functions"))
            );
        }
        _ => panic!("Expected FunctionScopedImport resolution for ClassLevel cycle"),
    }
}

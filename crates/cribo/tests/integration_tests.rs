#![allow(clippy::disallowed_methods)]

use insta::{assert_snapshot, with_settings};
use std::path::PathBuf;
use tempfile::TempDir;

use cribo::config::Config;
use cribo::orchestrator::BundleOrchestrator;

#[test]
fn test_simple_project_bundling() {
    // Initialize logger for debugging
    let _ = env_logger::try_init();

    let config = Config {
        src: vec![PathBuf::from("tests/fixtures/simple_project")],
        ..Default::default()
    };
    let mut bundler = BundleOrchestrator::new(config);

    // Create temporary output directory
    let temp_dir = TempDir::new().unwrap();
    let output_path = temp_dir.path().join("bundle.py");

    // Path to our test fixture
    let entry_path = PathBuf::from("tests/fixtures/simple_project/main.py");

    // Skip if test fixture doesn't exist (for CI environments)
    if !entry_path.exists() {
        return;
    }

    // Test bundling
    let result = bundler.bundle(&entry_path, &output_path, false);

    match result {
        Ok(()) => {
            // Verify output file exists
            assert!(output_path.exists());

            // Read and snapshot the bundled content
            let content = std::fs::read_to_string(&output_path).unwrap();

            with_settings!({
                description => "Bundle output for simple project with User model and helper functions",
            }, {
                assert_snapshot!(content);
            });
        }
        Err(e) => {
            eprintln!("Bundle test failed: {}", e);
            // Print more error details
            println!("Full error chain:");
            let mut source = e.source();
            while let Some(err) = source {
                println!("  Caused by: {}", err);
                source = err.source();
            }
            panic!("Bundling failed: {}", e);
        }
    }
}

#[test]
fn test_requirements_generation() {
    let config = Config::default();
    let mut bundler = BundleOrchestrator::new(config);

    // Create temporary output directory
    let temp_dir = TempDir::new().unwrap();
    let output_path = temp_dir.path().join("bundle.py");

    // Path to our test fixture with third-party dependencies
    let entry_path = PathBuf::from("tests/fixtures/pydantic_project/main.py");

    // Skip if test fixture doesn't exist
    if !entry_path.exists() {
        return;
    }

    // Test bundling with requirements generation
    let result = bundler.bundle(&entry_path, &output_path, true);

    match result {
        Ok(()) => {
            let mut output = String::new();

            // Check if bundle was created
            if output_path.exists() {
                let bundle_content = std::fs::read_to_string(&output_path).unwrap();
                output.push_str("=== BUNDLE CONTENT ===\n");
                output.push_str(&bundle_content);
                output.push_str("\n=== END BUNDLE CONTENT ===\n\n");
            }

            // Check if requirements.txt was created
            let requirements_path = temp_dir.path().join("requirements.txt");
            if requirements_path.exists() {
                let requirements = std::fs::read_to_string(&requirements_path).unwrap();
                output.push_str("=== REQUIREMENTS.TXT ===\n");
                output.push_str(&requirements);
                output.push_str("\n=== END REQUIREMENTS.TXT ===");
            } else {
                output.push_str("No requirements.txt file was generated.");
            }

            with_settings!({
                description => "Bundle and requirements generation for project with third-party dependencies",
            }, {
                assert_snapshot!(output);
            });
        }
        Err(e) => {
            eprintln!("Requirements test failed: {}", e);
            panic!("Requirements generation test failed: {}", e);
        }
    }
}

#[test]
fn test_module_resolution() {
    use cribo::config::Config;
    use cribo::resolver::{ImportType, ModuleResolver};

    let config = Config {
        src: vec![PathBuf::from("tests/fixtures/simple_project")],
        ..Default::default()
    };

    let resolver = ModuleResolver::new(config);

    match resolver {
        Ok(resolver) => {
            let first_party_modules_set = resolver.get_first_party_modules();
            let mut first_party_modules: Vec<_> = first_party_modules_set.iter().cloned().collect();
            first_party_modules.sort(); // Sort for deterministic output

            // Test import classifications
            let classifications = [
                (
                    ".relative_module",
                    resolver.classify_import(".relative_module"),
                ),
                (
                    "..parent_module",
                    resolver.classify_import("..parent_module"),
                ),
                (
                    "...grandparent.module",
                    resolver.classify_import("...grandparent.module"),
                ),
                ("os", resolver.classify_import("os")),
                (
                    "unknown_package",
                    resolver.classify_import("unknown_package"),
                ),
            ];

            // Verify the expected classifications
            assert_eq!(classifications[0].1, ImportType::FirstParty);
            assert_eq!(classifications[1].1, ImportType::FirstParty);
            assert_eq!(classifications[2].1, ImportType::FirstParty);
            assert_eq!(classifications[3].1, ImportType::StandardLibrary);
            assert_eq!(classifications[4].1, ImportType::ThirdParty);

            let output = format!(
                "First-party modules discovered: {:#?}\n\nImport classifications:\n{}",
                first_party_modules, // Use the sorted Vec
                classifications
                    .iter()
                    .map(|(module, import_type)| format!("  {} -> {:?}", module, import_type))
                    .collect::<Vec<_>>()
                    .join("\n")
            );

            with_settings!({
                description => "Module resolution results showing discovered modules and import classifications",
            }, {
                assert_snapshot!(output);
            });
        }
        Err(e) => {
            eprintln!("Module resolution test failed: {}", e);
            panic!("Module resolution failed: {}", e);
        }
    }
}

#[test]
fn test_dependency_graph() {
    use cribo::cribo_graph::CriboGraph;
    use std::path::PathBuf;

    let mut graph = CriboGraph::new();

    // Add modules to graph
    let main_id = graph.add_module("main".to_string(), PathBuf::from("main.py"));
    let utils_id = graph.add_module(
        "utils.helpers".to_string(),
        PathBuf::from("utils/helpers.py"),
    );
    let models_id = graph.add_module("models.user".to_string(), PathBuf::from("models/user.py"));

    // Add dependencies - main depends on utils and models
    graph.add_module_dependency(main_id, utils_id);
    graph.add_module_dependency(main_id, models_id);

    // Collect graph information
    let module_count = graph.modules.len();
    let mut dependencies_info = Vec::new();

    // Get module names in sorted order for deterministic output
    let mut module_names: Vec<_> = graph
        .modules
        .values()
        .map(|m| m.module_name.clone())
        .collect();
    module_names.sort();

    for module_name in &module_names {
        if let Some(&module_id) = graph.module_names.get(module_name) {
            let deps = graph.get_dependencies(module_id);
            let dep_names: Vec<String> = deps
                .iter()
                .filter_map(|&dep_id| graph.modules.get(&dep_id).map(|m| m.module_name.clone()))
                .collect();
            dependencies_info.push(format!(
                "Module {} depends on: {:?}",
                module_name, dep_names
            ));
        }
    }

    // Test topological sort
    let sorted = graph.topological_sort().unwrap();
    let sorted_names: Vec<String> = sorted
        .iter()
        .filter_map(|&id| graph.modules.get(&id).map(|m| m.module_name.clone()))
        .collect();

    let output = format!(
        "Graph has {} modules\n\nDependencies:\n{}\n\nTopological sort: {:?}",
        module_count,
        dependencies_info.join("\n"),
        sorted_names
    );

    with_settings!({
        description => "CriboGraph creation and topological sorting results",
    }, {
        assert_snapshot!(output);
    });

    // Verify the sort order still makes sense
    let main_index = sorted_names.iter().position(|name| name == "main").unwrap();
    let utils_index = sorted_names
        .iter()
        .position(|name| name == "utils.helpers")
        .unwrap();
    let models_index = sorted_names
        .iter()
        .position(|name| name == "models.user")
        .unwrap();

    assert!(utils_index < main_index);
    assert!(models_index < main_index);
}

#[test]
fn test_extract_edge_case_imports() {
    // Initialize logger for debugging
    let _ = env_logger::try_init();

    let bundler = BundleOrchestrator::new(Config::default());
    // Path to edge-case test file in the fixtures directory
    let file_path = PathBuf::from("tests/fixtures/test_edge_cases.py");

    // Ensure the test file exists
    assert!(
        file_path.exists(),
        "test_edge_cases.py not found at expected location"
    );

    let imports = bundler
        .extract_imports(&file_path, None)
        .expect("Failed to extract imports");

    let output = format!(
        "Extracted imports from edge cases file:\n{}",
        imports
            .iter()
            .enumerate()
            .map(|(i, import)| format!("  {}: {}", i + 1, import))
            .collect::<Vec<_>>()
            .join("\n")
    );

    with_settings!({
        description => "Import extraction from complex Python file with various import patterns",
    }, {
        assert_snapshot!(output);
    });
}

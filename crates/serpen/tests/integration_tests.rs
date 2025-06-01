use insta::{assert_snapshot, with_settings};
use std::path::PathBuf;
use tempfile::TempDir;

use serpen::bundler::Bundler;
use serpen::config::Config;

#[test]
fn test_simple_project_bundling() {
    // Initialize logger for debugging
    let _ = env_logger::try_init();

    let config = Config {
        src: vec![PathBuf::from("tests/fixtures/simple_project")],
        ..Default::default()
    };
    let mut bundler = Bundler::new(config);

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
    let mut bundler = Bundler::new(config);

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
    use serpen::config::Config;
    use serpen::resolver::{ImportType, ModuleResolver};

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
    use serpen::dependency_graph::{DependencyGraph, ModuleNode};
    use std::path::PathBuf;

    let mut graph = DependencyGraph::new();

    // Create test modules
    let main_module = ModuleNode {
        name: "main".to_string(),
        path: PathBuf::from("main.py"),
        imports: vec!["utils.helpers".to_string(), "models.user".to_string()],
    };

    let utils_module = ModuleNode {
        name: "utils.helpers".to_string(),
        path: PathBuf::from("utils/helpers.py"),
        imports: vec![],
    };

    let models_module = ModuleNode {
        name: "models.user".to_string(),
        path: PathBuf::from("models/user.py"),
        imports: vec![],
    };

    // Add modules to graph
    graph.add_module(main_module);
    graph.add_module(utils_module);
    graph.add_module(models_module);

    // Add dependencies - main depends on utils and models
    // This means utils.helpers -> main and models.user -> main (dependency -> dependent)
    graph.add_dependency("utils.helpers", "main").unwrap();
    graph.add_dependency("models.user", "main").unwrap();

    // Collect graph information
    let mut modules = graph.get_modules();
    modules.sort_by(|a, b| a.name.cmp(&b.name));
    let module_count = modules.len();
    let mut dependencies_info = Vec::new();

    for module in modules {
        if let Some(deps) = graph.get_dependencies(&module.name) {
            dependencies_info.push(format!("Module {} depends on: {:?}", module.name, deps));
        }
    }

    // Test topological sort
    let sorted = graph.topological_sort().unwrap();
    let sorted_names: Vec<&str> = sorted.iter().map(|m| m.name.as_str()).collect();

    let output = format!(
        "Graph has {} modules\n\nDependencies:\n{}\n\nTopological sort: {:?}",
        module_count,
        dependencies_info.join("\n"),
        sorted_names
    );

    with_settings!({
        description => "Dependency graph creation and topological sorting results",
    }, {
        assert_snapshot!(output);
    });

    // Verify the sort order still makes sense
    let main_index = sorted.iter().position(|m| m.name == "main").unwrap();
    let utils_index = sorted
        .iter()
        .position(|m| m.name == "utils.helpers")
        .unwrap();
    let models_index = sorted.iter().position(|m| m.name == "models.user").unwrap();

    assert!(utils_index < main_index);
    assert!(models_index < main_index);
}

#[test]
fn test_extract_edge_case_imports() {
    // Initialize logger for debugging
    let _ = env_logger::try_init();

    let bundler = Bundler::new(Config::default());
    // Path to edge-case test file in the fixtures directory
    let file_path = PathBuf::from("tests/fixtures/test_edge_cases.py");

    // Ensure the test file exists
    assert!(
        file_path.exists(),
        "test_edge_cases.py not found at expected location"
    );

    let imports = bundler
        .extract_imports(&file_path)
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

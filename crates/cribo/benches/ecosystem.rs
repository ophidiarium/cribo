use std::{path::PathBuf, process::Command, time::Duration};

use criterion::{Criterion, criterion_group, criterion_main};
use log::warn;

fn get_workspace_root() -> PathBuf {
    let output = Command::new("cargo")
        .args(["locate-project", "--workspace", "--message-format", "plain"])
        .output()
        .expect("Failed to get workspace root");

    assert!(
        output.status.success(),
        "cargo locate-project failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let cargo_toml_path = String::from_utf8(output.stdout)
        .expect("Invalid UTF-8")
        .trim()
        .to_owned();

    PathBuf::from(cargo_toml_path)
        .parent()
        .expect("Failed to get parent directory")
        .to_path_buf()
}

fn bundle_ecosystem_package(package_name: &str) -> std::process::Output {
    let workspace_root = get_workspace_root();

    // Different packages have different structures
    let package_base = workspace_root
        .join("ecosystem")
        .join("packages")
        .join(package_name);

    // Try different common locations for the package
    // Special handling for pyyaml which uses "yaml" as module name
    let module_name = if package_name == "pyyaml" {
        "yaml"
    } else {
        package_name
    };

    let possible_paths = if package_name == "pyyaml" {
        // PyYAML has a yaml/ directory with Cython files, skip it
        vec![
            package_base.join("lib3").join(module_name), // In lib3: packages/pyyaml/lib3/yaml
            package_base.join("lib").join(module_name),  // In lib: packages/pyyaml/lib/yaml
        ]
    } else {
        vec![
            package_base.join(module_name), // Direct: packages/idna/idna
            package_base.join("src").join(module_name), // In src: packages/requests/src/requests
        ]
    };

    let package_path = possible_paths
        .iter()
        .find(|p| {
            if p.exists() {
                // Check if it's a Python package directory
                let has_init = p.join("__init__.py").exists() || p.join("__main__.py").exists();
                if !has_init {
                    warn!(
                        "Directory {:?} exists but doesn't contain __init__.py or __main__.py",
                        p
                    );
                }
                has_init
            } else {
                false
            }
        })
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "Package {} not found in any of the expected locations with valid Python module: \
                 {:?}",
                package_name, possible_paths
            );
        });

    // Create per-package output directory
    let output_dir = workspace_root.join("target").join("tmp").join(package_name);
    std::fs::create_dir_all(&output_dir).ok();
    let output_path = output_dir.join("bundled_bench.py");

    // Prefer prebuilt binary if available
    let mut cmd = option_env!("CARGO_BIN_EXE_cribo").map_or_else(
        || {
            let mut c = Command::new("cargo");
            c.args(["run", "--release", "--"]);
            c
        },
        Command::new,
    );

    let output = cmd
        .arg("--entry")
        .arg(&package_path)
        .arg("--output")
        .arg(&output_path)
        .arg("--emit-requirements")
        .current_dir(&workspace_root)
        .output()
        .expect("Failed to run cribo");

    // Fail fast on bundling errors
    assert!(
        output.status.success(),
        "Bundling {} failed\nSTDOUT:\n{}\nSTDERR:\n{}",
        package_name,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    output
}

fn benchmark_ecosystem_bundling(c: &mut Criterion) {
    let mut group = c.benchmark_group("ecosystem_bundling");

    // Configure for longer benchmarks
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));

    // Benchmark bundling for each package
    let packages = ["requests", "rich", "idna", "pyyaml", "httpx"];
    let workspace_root = get_workspace_root();

    // Check if ecosystem packages are available
    let ecosystem_dir = workspace_root.join("ecosystem").join("packages");
    if !ecosystem_dir.exists() {
        warn!(
            "Ecosystem packages directory does not exist at {:?}",
            ecosystem_dir
        );
        warn!(
            "Skipping ecosystem benchmarks. Run 'git submodule update --init --recursive \
             ecosystem/' to initialize."
        );
        group.finish();
        return;
    }

    for package in packages {
        // Check if package directory exists (the parent directory)
        let package_dir = ecosystem_dir.join(package);

        if !package_dir.exists() {
            warn!(
                "Skipping {} - package directory not found at {:?}",
                package, package_dir
            );
            continue;
        }

        // Check various possible locations for the package entry point
        // Special handling for pyyaml which uses "yaml" as module name
        let module_name = if package == "pyyaml" { "yaml" } else { package };

        let possible_entries = if package == "pyyaml" {
            // PyYAML has a yaml/ directory with Cython files, skip it
            vec![
                package_dir.join("lib3").join(module_name), // In lib3: packages/pyyaml/lib3/yaml
                package_dir.join("lib").join(module_name),  // In lib: packages/pyyaml/lib/yaml
            ]
        } else {
            vec![
                package_dir.join(module_name), // Direct: packages/idna/idna
                package_dir.join("src").join(module_name), /* In src: packages/requests/src/
                                                * requests */
            ]
        };

        let entry_exists = possible_entries.iter().any(|p| p.exists());

        if !entry_exists {
            warn!(
                "Skipping {} - package entry point not found in any expected location",
                package
            );
            warn!("  Checked: {:?}", possible_entries);
            continue;
        }

        group.bench_function(format!("bundle_{}", package), |b| {
            b.iter(|| bundle_ecosystem_package(package));
        });
    }

    group.finish();
}

criterion_group!(benches, benchmark_ecosystem_bundling);
criterion_main!(benches);

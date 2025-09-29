use std::{path::PathBuf, process::Command, time::Duration};

use criterion::{Criterion, criterion_group, criterion_main};

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
        .to_string();

    PathBuf::from(cargo_toml_path)
        .parent()
        .expect("Failed to get parent directory")
        .to_path_buf()
}

fn bundle_ecosystem_package(package_name: &str) -> std::process::Output {
    let workspace_root = get_workspace_root();
    let package_path = workspace_root
        .join("ecosystem")
        .join("packages")
        .join(package_name)
        .join(package_name);

    // Create per-package output directory
    let output_dir = workspace_root.join("target").join("tmp").join(package_name);
    std::fs::create_dir_all(&output_dir).ok();
    let output_path = output_dir.join("bundled_bench.py");

    // Prefer prebuilt binary if available
    let mut cmd = if let Some(exe) = option_env!("CARGO_BIN_EXE_cribo") {
        Command::new(exe)
    } else {
        let mut c = Command::new("cargo");
        c.args(["run", "--release", "--"]);
        c
    };

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
    if !output.status.success() {
        panic!(
            "Bundling {} failed\nSTDOUT:\n{}\nSTDERR:\n{}",
            package_name,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

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

    for package in packages {
        // Check if package exists
        let package_path = workspace_root
            .join("ecosystem")
            .join("packages")
            .join(package);

        if !package_path.exists() {
            eprintln!(
                "Skipping {} - package not found at {:?}",
                package, package_path
            );
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

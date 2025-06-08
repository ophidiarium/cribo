# Cribo Benchmarking Guide

This document describes the benchmarking infrastructure for Cribo, designed to track performance regressions and ensure consistent bundling performance.

<!-- Baseline establishment trigger: This change establishes initial benchmarks -->

## Overview

Cribo uses [Bencher.dev](https://bencher.dev) with [Criterion.rs](https://github.com/bheisler/criterion.rs) and [Hyperfine](https://github.com/sharkdp/hyperfine) for comprehensive benchmarking with three layers of integration:

1. **Local Development**: Quick performance checks during development using Criterion.rs
2. **PR Comments**: Automatic performance comparison on pull requests via Bencher.dev
3. **Historical Tracking**: Long-term performance trends with statistical analysis via Bencher.dev dashboard

## Local Benchmarking

### Quick Start

```bash
# Run all benchmarks
cargo bench

# Run with our helper script (recommended)
./scripts/bench.sh

# Save a baseline
./scripts/bench.sh --save-baseline main

# Compare against baseline
./scripts/bench.sh --baseline main

# Open HTML report
./scripts/bench.sh --open
```

### Bencher.dev CLI Integration

For continuous performance tracking with Bencher.dev:

```bash
# Setup (one-time)
cp .env.example .env
# Edit .env and add your BENCHER_API_TOKEN

# Install Bencher CLI
cargo install bencher_cli

# Run benchmarks with cloud tracking
./scripts/bench-bencher.sh

# Or manually with Bencher CLI
bencher run \
    --project cribo \
    --token $BENCHER_API_TOKEN \
    --testbed local \
    --adapter rust_criterion \
    "cargo bench --bench bundling"
```

The results will be automatically uploaded to your Bencher.dev dashboard at:
https://bencher.dev/console/projects/cribo/perf

### Cargo Aliases

We provide convenient aliases for common operations:

```bash
cargo bench-local     # Run benchmarks
cargo bench-save      # Save baseline as 'main'
cargo bench-compare   # Compare against 'main' baseline
```

## Benchmarked Operations

### Core Operations (Critical Path)

1. **bundle_simple_project**: End-to-end bundling of a multi-module project
2. **parse_python_ast**: Python AST parsing performance
3. **resolve_module_path**: Module resolution speed

### Supporting Operations

4. **extract_imports**: Import statement extraction
5. **build_dependency_graph**: Dependency graph construction

## Performance Targets

### Acceptable Performance

- **Individual benchmarks**: ≤3% regression (within noise margin)
- **Overall bundling**: ≤1% regression
- **With justification**: Up to 5% for significant features

### Unacceptable Regressions

- **>5%** for any core operation without justification
- **>10%** for any benchmark (indicates algorithmic issue)
- **Any regression** in AST parsing (critical path)

## CI Integration

### Bencher.dev Integration

Every PR automatically receives benchmark comparison comments with visual charts and statistical analysis:

```
## 🐰 Bencher Report

### bundle_simple_project
- **Baseline**: 3,412 ns/iter (± 185) 
- **Current**: 2,987 ns/iter (± 147)
- **Change**: -12.45% 🎉 (Performance improved)
- **Statistical Significance**: ✅ Significant improvement

### parse_python_ast  
- **Baseline**: 45,123 ns/iter (± 2,341)
- **Current**: 46,789 ns/iter (± 2,156) 
- **Change**: +3.69% ⚠️ (Slight regression)
- **Statistical Significance**: ❌ Within noise threshold

[View detailed results on Bencher.dev →](https://bencher.dev/perf/cribo)
```

### Benchmark Types

1. **Micro-benchmarks**: Criterion.rs for individual function performance
2. **CLI benchmarks**: Hyperfine for end-to-end command performance
3. **Statistical analysis**: Bencher.dev prevents false positives from CI noise

### Workflow Files

- **`.github/workflows/base_benchmarks.yml`**: Baseline benchmarking for main branch with thresholds
- **`.github/workflows/pr_benchmarks.yml`**: PR benchmarking with comparison comments
- **`.github/workflows/pr_cleanup.yml`**: Archive closed PR branches for data hygiene

## Writing New Benchmarks

Add benchmarks to `crates/cribo/benches/bundling.rs`:

```rust
use criterion::{Criterion, black_box, criterion_group, criterion_main};

fn benchmark_new_feature(c: &mut Criterion) {
    c.bench_function("new_feature", |b| {
        b.iter(|| {
            // Your code here
            let result = expensive_operation(black_box(input));
            black_box(result); // Prevent optimization
        });
    });
}

// Add to the criterion_group!
criterion_group!(
    benches,
    benchmark_bundling,
    benchmark_new_feature, // Add here
                           // ... other benchmarks
);
```

## Performance Debugging

### When Benchmarks Regress

1. **Identify the regression**:
   ```bash
   cargo bench-compare
   ```

2. **Profile the code**:
   ```bash
   cargo install flamegraph
   cargo flamegraph --bench bundling
   ```

3. **Focus on hotspots**:
   - Check algorithmic complexity
   - Look for unnecessary allocations
   - Consider data structure choices

4. **Verify improvements**:
   ```bash
   cargo bench-compare
   ```

### Common Optimization Patterns

1. **Avoid repeated allocations**: Use `with_capacity()` for collections
2. **Minimize cloning**: Use references where possible
3. **Cache computations**: Store results of expensive operations
4. **Use efficient data structures**: `IndexMap` over `HashMap` for determinism

## Bencher.dev Features

### Current Integration

- **Historical performance tracking**: Automatic trend analysis over time
- **Statistical regression detection**: Prevents false positives from CI environment noise
- **Cross-platform benchmarking**: Consistent results across different environments
- **Visual dashboards**: Web UI with charts and performance insights
- **JSON API**: Machine-readable results for AI agent integration

### Future Enhancements

Additional metrics we plan to track:

- **Memory usage**: Heap allocation and peak memory consumption
- **Binary size**: Compiled binary size tracking for deployment optimization
- **Python execution performance**: Runtime performance of bundled code
- **Large project benchmarking**: Real-world scenarios with complex dependency graphs

## Maintenance

### Updating Baselines

After significant performance improvements:

```bash
# On main branch after merge
git checkout main
git pull
./scripts/bench.sh --save-baseline main
```

### Benchmark Stability

- Run benchmarks on quiet systems
- Close unnecessary applications
- Use consistent CPU governor settings
- Consider using `nice -n -20` for priority

## Troubleshooting

### "No baseline found"

Save a baseline first:

```bash
cargo bench-save
```

### Noisy Results

- Increase sample size in Criterion
- Run with higher priority: `nice -n -20 cargo bench`
- Check for background processes

### CI Failures

Check the workflow logs:

```bash
gh run list --workflow=base_benchmarks.yml
gh run list --workflow=pr_benchmarks.yml
gh run view <run-id>
```

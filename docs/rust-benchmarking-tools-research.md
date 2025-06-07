# Top 3 Rust Benchmarking Solutions for Performance Regression Testing

After analyzing benchmarking practices across major Rust projects (uv, ruff, pydantic, rspack, mako, biome) and the broader ecosystem, here are the top 3 recommended solutions that excel in GitHub integration, developer workflows, and meet your specific requirements.

## 1. **Bencher.dev + Criterion.rs/Hyperfine** - Most Comprehensive Solution

**Implementation Examples:**

- **Bencher Cloud**: https://github.com/bencherdev/bencher
- **Used by**: Microsoft CCF, Rustls, Diesel, GreptimeDB
- **Live Example**: https://bencher.dev/perf/diesel

**Key Features:**

- ✅ **Free for open source projects** with generous hosted tier
- ✅ **Automated PR comments** with visual regression alerts and charts
- ✅ **JSON output** via Bencher Metric Format (BMF) for AI agents
- ✅ **Micro + Macro benchmarking** - integrates with both Criterion.rs and Hyperfine
- ✅ **Execution time tracking** with statistical analysis
- ✅ **Binary size tracking** via custom adapters
- ✅ **Python runtime testing** - supports pytest-benchmark integration
- ✅ **VSCode friendly** with development container support
- ✅ **Historical tracking** with web dashboard

**GitHub Actions Setup:**

```yaml
name: Continuous Benchmarking
on: [push, pull_request]
jobs:
  benchmark:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: bencherdev/bencher@main
      - run: |
          bencher run \
            --github-actions "${{ secrets.GITHUB_TOKEN }}" \
            --testbed ubuntu-latest \
            --adapter json \
            "cargo bench --bench my_benchmark -- --output-format bencher | tee results.json"
```

**Why It's #1:** Bencher provides the most complete solution with excellent GitHub integration, supports both Rust and Python benchmarks (crucial for Python+Rust projects), and offers the best PR comment system with visual alerts. The statistical analysis prevents false positives in CI environments.

## 2. **criterion-compare-action + Criterion.rs** - Simplest Elegant Solution

**Implementation Examples:**

- **Action Repository**: https://github.com/boa-dev/criterion-compare-action
- **Used by**: Boa JavaScript Engine, multiple Rust projects
- **Live Example**: https://github.com/boa-dev/boa/pull/3678

**Key Features:**

- ✅ **Zero-config PR comments** comparing benchmarks between branches
- ✅ **Direct Criterion.rs integration** without additional tooling
- ✅ **JSON support** via Criterion's built-in capabilities
- ✅ **Execution time focus** with statistical significance
- ✅ **Free and open source**
- ✅ **Simple developer workflow** - just add one GitHub Action
- ✅ **Configurable features** for workspace packages

**GitHub Actions Setup:**

```yaml
name: Benchmark PR
on: [pull_request]
jobs:
  benchmark:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: boa-dev/criterion-compare-action@v3
        with:
          branchName: ${{ github.base_ref }}
          features: 'full,async'
          benchmarks: 'bench_core,bench_api'
```

**Example PR Comment Output:**

```
## Benchmark Results

### fibonacci
- **main**: 3,412 ns/iter (± 185)
- **PR**: 2,987 ns/iter (± 147)
- **Change**: -12.45% 🎉 (Performance improved)

### parse_json
- **main**: 45,123 ns/iter (± 2,341)
- **PR**: 46,789 ns/iter (± 2,156)
- **Change**: +3.69% ⚠️ (Regression detected)
```

**Why It's #2:** Perfect balance of simplicity and functionality. Solves the "benchmark results in PR comments" problem elegantly with minimal configuration. Ideal for teams wanting quick setup without external services.

## 3. **github-action-benchmark + cargo-criterion** - Most Flexible Solution

**Implementation Examples:**

- **Action Repository**: https://github.com/benchmark-action/github-action-benchmark
- **cargo-criterion**: https://github.com/bheisler/cargo-criterion
- **Live Demo**: https://benchmark-action.github.io/github-action-benchmark/dev/bench/

**Key Features:**

- ✅ **GitHub Pages integration** with automatic chart generation
- ✅ **Machine-readable JSON** via cargo-criterion
- ✅ **Multi-language support** (Rust, Python, JavaScript, etc.)
- ✅ **Custom metrics** beyond execution time
- ✅ **Alert thresholds** with automatic PR comments
- ✅ **Historical tracking** via Git history
- ✅ **Free for all projects**

**Complete Workflow:**

```yaml
name: Rust Benchmark
on:
  push:
    branches: [main]
  pull_request:

permissions:
  contents: write
  deployments: write

jobs:
  benchmark:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      # Install cargo-criterion for JSON output
      - run: cargo install cargo-criterion

      # Run benchmarks with JSON output
      - run: |
          cargo criterion --message-format=json > output.json

      # Store and analyze results
      - uses: benchmark-action/github-action-benchmark@v1
        with:
          tool: 'customBiggerIsBetter'
          output-file-path: output.json
          github-token: ${{ secrets.GITHUB_TOKEN }}
          auto-push: true
          alert-threshold: '150%'
          comment-on-alert: true
          summary-always: true
          gh-pages-branch: gh-pages
          benchmark-data-dir-path: dev/bench
```

**Why It's #3:** Maximum flexibility with support for custom metrics, multiple languages, and sophisticated visualization. The GitHub Pages integration provides free hosting for performance dashboards. cargo-criterion adds JSON output to standard Criterion benchmarks.

## Practical Implementation Strategy

Based on the research of successful Rust projects:

### For Python+Rust Projects (like uv, ruff, pydantic):

1. Use **Bencher.dev** for comprehensive tracking across both languages
2. Combine Criterion.rs (Rust micro-benchmarks) with pytest-benchmark (Python)
3. Add Hyperfine for end-to-end CLI performance testing

### For Pure Rust Projects:

1. Start with **criterion-compare-action** for immediate PR feedback
2. Add **Bencher.dev** as you scale for historical tracking
3. Use Hyperfine for macro-benchmarking scenarios

### Common Patterns from Top Projects:

- **ruff** uses ecosystem testing against real codebases - consider this approach
- **rspack** tracks HMR performance and bundle sizes - relevant for build tools
- **biome** achieves 7-100x performance gains through rigorous benchmarking

## Key Insights on Missing Features

**MCP/AI Agent Support:** Currently no benchmarking tools have native MCP support, but all recommended solutions provide JSON output suitable for AI consumption. Bencher's BMF format is particularly well-structured for automated analysis.

**VSCode Integration:** Direct IDE integration remains limited, but all tools work well with VSCode's terminal and task runner. Bencher provides the best developer experience with dev containers.

**Bundle Size Tracking:** While execution time is well-covered, bundle/binary size tracking requires custom adapters. Bencher supports this through custom metrics, and github-action-benchmark allows arbitrary metric tracking.

## Recommendation Summary

**Choose Bencher.dev** if you want:

- Professional performance tracking with minimal setup
- Support for both Rust and Python benchmarks
- The best PR comment integration with visual alerts
- Long-term performance trend analysis

**Choose criterion-compare-action** if you want:

- Quick setup with immediate value
- Simple PR comparison without external services
- Focus purely on Criterion.rs benchmarks

**Choose github-action-benchmark** if you want:

- Maximum customization flexibility
- Self-hosted performance dashboards
- Support for multiple benchmark formats

All three solutions are actively maintained, used by major Rust projects, and provide the JSON output necessary for AI agent integration. The choice depends on your specific needs for simplicity versus comprehensiveness.

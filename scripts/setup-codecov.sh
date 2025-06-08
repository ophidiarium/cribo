#!/bin/bash

# Codecov Setup Helper Script for Cribo Project
# This script helps with initial Codecov configuration

set -e

echo "🔧 Codecov Setup Helper for Cribo"
echo "=================================="
echo

# Check if we're in a git repository
if ! git rev-parse --git-dir >/dev/null 2>&1; then
  echo "❌ Error: This script must be run from within the cribo git repository"
  exit 1
fi

# Check if .codecov.yml exists
if [[ -f ".codecov.yml" ]]; then
  echo "✅ Codecov configuration file (.codecov.yml) already exists"
else
  echo "❌ Codecov configuration file (.codecov.yml) not found"
  echo "   Run this script from the project root directory"
  exit 1
fi

# Check if GitHub Actions workflow exists
if [[ -f ".github/workflows/coverage.yml" ]]; then
  echo "✅ GitHub Actions coverage workflow exists"
else
  echo "❌ GitHub Actions coverage workflow not found"
  exit 1
fi

echo
echo "📋 Next Steps for Codecov Integration:"
echo "1. Go to https://codecov.io and sign up with your GitHub account"
echo "2. Add the 'ophidiarium/cribo' repository to Codecov"
echo "3. Copy the repository upload token from Codecov dashboard"
echo "4. Add the token to GitHub repository secrets:"
echo "   → Go to: https://github.com/ophidiarium/cribo/settings/secrets/actions"
echo "   → Click 'New repository secret'"
echo "   → Name: CODECOV_TOKEN"
echo "   → Value: [paste your Codecov upload token]"
echo "5. Push changes to trigger the coverage workflow"
echo

echo "🧪 Test Coverage Locally:"
echo "Run 'cargo coverage-text' to see coverage report"
echo "Run 'cargo coverage' to open HTML coverage report"
echo
echo "📊 Branch Coverage (Experimental):"
echo "• Install nightly: rustup toolchain install nightly"
echo "• Add component: rustup component add llvm-tools-preview --toolchain nightly"
echo "• Run branch coverage: cargo +nightly coverage-branch"
echo "• Note: Branch coverage is experimental and requires nightly Rust"
echo

echo "📊 Expected Coverage Targets (configured in .codecov.yml):"
echo "• Project coverage: 80% target"
echo "• Patch coverage: 80% target"
echo "• Threshold: 1% change tolerance"
echo

# Check current coverage if possible
if command -v cargo-llvm-cov &>/dev/null; then
  echo "🎯 Current Coverage Status:"
  echo "Running quick coverage check..."

  # Create coverage directory if it doesn't exist
  mkdir -p target/llvm-cov

  # Generate quick coverage report
  if cargo coverage-text --quiet; then
    echo "Coverage report generated successfully!"
  else
    echo "❌ Failed to generate coverage report"
    echo "   Make sure tests pass: cargo test"
  fi
else
  echo "⚠️  cargo-llvm-cov not installed"
  echo "   Install with: cargo install cargo-llvm-cov"
  echo "   Add component: rustup component add llvm-tools-preview"
fi

echo
echo "✅ Codecov setup is ready!"
echo "Once you've added the CODECOV_TOKEN secret, coverage will be"
echo "automatically uploaded on every push and PR."

#!/bin/bash

# Branch Coverage Setup and Testing Script for Cribo Project
# This script helps set up and test branch coverage functionality

set -e

echo "🌿 Branch Coverage Setup for Cribo"
echo "==================================="
echo

# Check if we're in a git repository
if ! git rev-parse --git-dir >/dev/null 2>&1; then
    echo "❌ Error: This script must be run from within the cribo git repository"
    exit 1
fi

# Check if nightly toolchain is installed
echo "🔍 Checking Rust toolchains..."
if rustup toolchain list | grep -q "nightly"; then
    echo "✅ Nightly toolchain is available"
    nightly_version=$(rustup run nightly rustc --version)
    echo "   Version: $nightly_version"
else
    echo "❌ Nightly toolchain not found"
    echo "📥 Installing nightly toolchain..."
    rustup toolchain install nightly
    echo "✅ Nightly toolchain installed"
fi

# Check if llvm-tools-preview is installed for nightly
echo
echo "🔍 Checking llvm-tools-preview component for nightly..."
if rustup component list --toolchain nightly | grep -q "llvm-tools-preview.*installed"; then
    echo "✅ llvm-tools-preview is installed for nightly"
else
    echo "❌ llvm-tools-preview not found for nightly"
    echo "📥 Installing llvm-tools-preview for nightly..."
    rustup component add llvm-tools-preview --toolchain nightly
    echo "✅ llvm-tools-preview installed for nightly"
fi

# Check if cargo-llvm-cov is installed
echo
echo "🔍 Checking cargo-llvm-cov installation..."
if command -v cargo-llvm-cov &>/dev/null; then
    echo "✅ cargo-llvm-cov is installed"
    cov_version=$(cargo llvm-cov --version)
    echo "   Version: $cov_version"
else
    echo "❌ cargo-llvm-cov not found"
    echo "📥 Installing cargo-llvm-cov..."
    cargo install cargo-llvm-cov
    echo "✅ cargo-llvm-cov installed"
fi

echo
echo "🌿 Branch Coverage Information:"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "• Branch coverage tracks which branches of conditional statements are executed"
echo "• Supported: if, while, match guards, && and || operators"
echo "• Not supported: if let, while let, for, match arms"
echo "• Branches in macro expansions are ignored"
echo

echo "🧪 Available Branch Coverage Commands:"
echo "cargo +nightly coverage-branch       # HTML report with branch coverage"
echo "cargo +nightly coverage-branch-text  # Text report with branch coverage"
echo "cargo +nightly coverage-branch-lcov  # LCOV format with branch coverage"
echo

echo "⚠️  Why Branch Coverage Shows Zeros with Stable Rust:"
echo "Branch coverage is experimental and only works with nightly Rust."
echo "When using stable Rust, the Branch Coverage column will show all zeros."
echo "This is expected behavior, not a configuration issue."
echo

# Test branch coverage if possible
echo "🧪 Testing Branch Coverage:"
echo "Running quick branch coverage test with nightly..."

# Create coverage directory if it doesn't exist
mkdir -p target/llvm-cov

# Generate quick branch coverage report
if cargo +nightly coverage-branch-text --quiet 2>/dev/null; then
    echo "✅ Branch coverage test successful!"
    echo "   Branch coverage data is now available in HTML reports"
else
    echo "❌ Branch coverage test failed"
    echo "   This may be due to test failures or configuration issues"
    echo "   Try running: cargo +nightly test"
fi

echo
echo "🎯 Next Steps:"
echo "1. Use 'cargo +nightly coverage-branch' to see branch coverage in HTML"
echo "2. Compare with 'cargo coverage' (stable) to see the difference"
echo "3. Branch coverage provides more detailed conditional analysis"
echo
echo "✨ Branch coverage setup complete!"

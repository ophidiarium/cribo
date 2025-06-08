#!/bin/bash
# Cribo - Python Source Bundler
# Coverage script for development

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Function to print colored output
print_status() {
  echo -e "${GREEN}[INFO]${NC} $1"
}

print_warning() {
  echo -e "${YELLOW}[WARN]${NC} $1"
}

print_error() {
  echo -e "${RED}[ERROR]${NC} $1"
}

# Function to install cargo-llvm-cov if not already installed
install_cargo_llvm_cov() {
  if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
    print_status "Installing cargo-llvm-cov..."
    cargo install cargo-llvm-cov --locked
  else
    print_status "cargo-llvm-cov is already installed"
  fi
}

# Function to show help
show_help() {
  echo "Cribo Coverage Tool"
  echo ""
  echo "Usage: $0 [command]"
  echo ""
  echo "Commands:"
  echo "  test             - Run all tests"
  echo "  coverage         - Run tests with coverage (text report)"
  echo "  coverage-html    - Generate HTML coverage report"
  echo "  coverage-lcov    - Generate LCOV coverage report"
  echo "  install          - Install cargo-llvm-cov"
  echo "  clean            - Clean build artifacts and coverage reports"
  echo "  help             - Show this help message"
  echo ""
}

# Function to run tests
run_tests() {
  print_status "Running all tests..."
  cargo test --workspace
}

# Function to run coverage with text output (like Istanbul)
run_coverage() {
  install_cargo_llvm_cov
  print_status "Running tests with coverage..."
  cargo coverage-text
}

# Function to generate HTML coverage report
generate_html_coverage() {
  install_cargo_llvm_cov
  print_status "Generating HTML coverage report..."
  cargo coverage
  print_status "Coverage report generated and opened in browser"
}

# Function to generate LCOV coverage report
generate_lcov_coverage() {
  install_cargo_llvm_cov
  print_status "Generating LCOV coverage report..."
  mkdir -p target/llvm-cov
  cargo coverage-lcov
  print_status "LCOV report generated at target/llvm-cov/lcov.info"
}

# Function to clean build artifacts and coverage reports
clean_artifacts() {
  print_status "Cleaning build artifacts and coverage reports..."
  cargo coverage-clean
  rm -rf target/llvm-cov coverage/ coverage.lcov
  print_status "Clean complete"
}

# Main script logic
case "${1:-help}" in
test)
  run_tests
  ;;
coverage)
  run_coverage
  ;;
coverage-html)
  generate_html_coverage
  ;;
coverage-lcov)
  generate_lcov_coverage
  ;;
install)
  install_cargo_llvm_cov
  ;;
clean)
  clean_artifacts
  ;;
help | --help | -h)
  show_help
  ;;
*)
  print_error "Unknown command: $1"
  echo ""
  show_help
  exit 1
  ;;
esac

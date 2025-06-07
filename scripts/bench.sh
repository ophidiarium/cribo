#!/usr/bin/env bash
# Script to run benchmarks with human-readable output

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}🚀 Running Serpen Benchmarks${NC}"
echo "================================="

# Check if we're in the right directory
if [[ ! -f "Cargo.toml" ]]; then
    echo -e "${RED}Error: Must run from project root directory${NC}"
    exit 1
fi

# Parse arguments
SAVE_BASELINE=false
COMPARE_BASELINE=false
BASELINE_NAME=""
OPEN_REPORT=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --save-baseline)
            SAVE_BASELINE=true
            BASELINE_NAME="${2:-main}"
            shift 2
            ;;
        --baseline)
            COMPARE_BASELINE=true
            BASELINE_NAME="${2:-main}"
            shift 2
            ;;
        --open)
            OPEN_REPORT=true
            shift
            ;;
        --help|-h)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --save-baseline <name>  Save results as baseline (default: main)"
            echo "  --baseline <name>       Compare against baseline (default: main)"
            echo "  --open                  Open HTML report in browser"
            echo "  --help                  Show this help message"
            echo ""
            echo "Examples:"
            echo "  $0                      # Run benchmarks"
            echo "  $0 --save-baseline main # Save as 'main' baseline"
            echo "  $0 --baseline main      # Compare against 'main' baseline"
            echo "  $0 --open               # Run and open HTML report"
            exit 0
            ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}"
            echo "Use --help for usage information"
            exit 1
            ;;
    esac
done

# Build in release mode first
echo -e "${YELLOW}Building in release mode...${NC}"
cargo build --release

# Run the benchmarks
echo -e "${GREEN}Running benchmarks...${NC}"
echo ""

BENCH_CMD="cargo bench --bench bundling"

if [[ "$SAVE_BASELINE" == true ]]; then
    echo -e "${YELLOW}Saving baseline as '${BASELINE_NAME}'...${NC}"
    $BENCH_CMD -- --save-baseline "$BASELINE_NAME"
elif [[ "$COMPARE_BASELINE" == true ]]; then
    echo -e "${YELLOW}Comparing against baseline '${BASELINE_NAME}'...${NC}"
    $BENCH_CMD -- --baseline "$BASELINE_NAME" | tee benchmark_results.txt
    
    # Extract and highlight performance changes
    echo ""
    echo -e "${BLUE}Performance Summary:${NC}"
    echo "===================="
    
    # Look for performance improvements (green)
    if grep -E "improved|faster" benchmark_results.txt >/dev/null 2>&1; then
        echo -e "${GREEN}Improvements:${NC}"
        grep -E "improved|faster" benchmark_results.txt | sed 's/^/  /'
    fi
    
    # Look for regressions (red)
    if grep -E "regressed|slower" benchmark_results.txt >/dev/null 2>&1; then
        echo -e "${RED}Regressions:${NC}"
        grep -E "regressed|slower" benchmark_results.txt | sed 's/^/  /'
    fi
    
    rm -f benchmark_results.txt
else
    $BENCH_CMD
fi

# Report location
echo ""
echo -e "${BLUE}Detailed report available at:${NC}"
echo "  target/criterion/report/index.html"

# Open report if requested
if [[ "$OPEN_REPORT" == true ]]; then
    if command -v open >/dev/null 2>&1; then
        open target/criterion/report/index.html
    elif command -v xdg-open >/dev/null 2>&1; then
        xdg-open target/criterion/report/index.html
    else
        echo -e "${YELLOW}Could not auto-open report. Please open manually.${NC}"
    fi
fi

echo ""
echo -e "${GREEN}✓ Benchmarks completed successfully!${NC}"
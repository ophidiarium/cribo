#!/bin/bash

# Build Rust binaries for all npm-supported platforms
# This script uses cross-compilation to build binaries for multiple targets

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Logging functions
log_info() {
    echo -e "${BLUE}ℹ️  $1${NC}"
}

log_success() {
    echo -e "${GREEN}✅ $1${NC}"
}

log_warning() {
    echo -e "${YELLOW}⚠️  $1${NC}"
}

log_error() {
    echo -e "${RED}❌ $1${NC}"
}

# Default values
TARGETS_FILE=""
OUTPUT_DIR="target/npm-binaries"
PROFILE="release"
PACKAGE="serpen"

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
    --targets-file)
        TARGETS_FILE="$2"
        shift 2
        ;;
    --output-dir)
        OUTPUT_DIR="$2"
        shift 2
        ;;
    --profile)
        PROFILE="$2"
        shift 2
        ;;
    --package)
        PACKAGE="$2"
        shift 2
        ;;
    -h | --help)
        echo "Usage: $0 [OPTIONS]"
        echo ""
        echo "Options:"
        echo "  --targets-file FILE    File containing list of targets to build (one per line)"
        echo "  --output-dir DIR       Output directory for binaries (default: target/npm-binaries)"
        echo "  --profile PROFILE      Build profile (default: release)"
        echo "  --package PACKAGE      Package to build (default: serpen)"
        echo "  -h, --help            Show this help message"
        echo ""
        echo "If --targets-file is not specified, builds for all npm-supported targets."
        exit 0
        ;;
    *)
        log_error "Unknown option: $1"
        exit 1
        ;;
    esac
done

# Default targets for npm packages
DEFAULT_TARGETS=(
    "x86_64-unknown-linux-gnu"
    "x86_64-unknown-linux-musl"
    "aarch64-unknown-linux-gnu"
    "aarch64-unknown-linux-musl"
    "x86_64-apple-darwin"
    "aarch64-apple-darwin"
    "x86_64-pc-windows-msvc"
    "i686-pc-windows-msvc"
)

# Read targets from file or use defaults
if [[ -n "$TARGETS_FILE" && -f "$TARGETS_FILE" ]]; then
    log_info "Reading targets from $TARGETS_FILE"
    mapfile -t TARGETS <"$TARGETS_FILE"
else
    log_info "Using default npm targets"
    TARGETS=("${DEFAULT_TARGETS[@]}")
fi

log_info "Building $PACKAGE for ${#TARGETS[@]} targets with profile: $PROFILE"
log_info "Output directory: $OUTPUT_DIR"

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Check if cross is available for cross-compilation
if ! command -v cross &>/dev/null; then
    log_warning "cross not found. Installing cross for cross-compilation..."
    cargo install cross --git https://github.com/cross-rs/cross
fi

# Function to get binary name for target
get_binary_name() {
    local target=$1
    if [[ $target == *"windows"* ]]; then
        echo "${PACKAGE}.exe"
    else
        echo "$PACKAGE"
    fi
}

# Function to build for a specific target
build_target() {
    local target=$1
    local binary_name
    binary_name=$(get_binary_name "$target")

    log_info "Building for target: $target"

    # Use cross for cross-compilation, cargo for native builds
    local build_cmd
    if [[ "$target" == "$(rustc -vV | grep host | cut -d' ' -f2)" ]]; then
        build_cmd="cargo"
    else
        build_cmd="cross"
    fi

    # Add target if not already installed
    if [[ "$build_cmd" == "cargo" ]]; then
        rustup target add "$target" 2>/dev/null || true
    fi

    # Build command
    if ! $build_cmd build --package "$PACKAGE" --target "$target" --profile "$PROFILE"; then
        log_error "Failed to build for target: $target"
        return 1
    fi

    # Copy binary to output directory
    local source_path="target/$target/$PROFILE/$binary_name"
    local dest_path="$OUTPUT_DIR/$binary_name"

    if [[ -f "$source_path" ]]; then
        cp "$source_path" "$dest_path"
        log_success "Built and copied binary for $target -> $dest_path"

        # Print binary info
        ls -lh "$dest_path"
        if command -v file &>/dev/null; then
            file "$dest_path"
        fi
    else
        log_error "Binary not found at expected path: $source_path"
        return 1
    fi
}

# Build for all targets
failed_targets=()
successful_targets=()

for target in "${TARGETS[@]}"; do
    # Skip empty lines
    [[ -z "$target" ]] && continue

    if build_target "$target"; then
        successful_targets+=("$target")
    else
        failed_targets+=("$target")
    fi
    echo "" # Add spacing between builds
done

# Summary
echo "======================================"
log_info "Build Summary"
echo "======================================"

if [[ ${#successful_targets[@]} -gt 0 ]]; then
    log_success "Successfully built for ${#successful_targets[@]} targets:"
    printf '  %s\n' "${successful_targets[@]}"
fi

if [[ ${#failed_targets[@]} -gt 0 ]]; then
    log_error "Failed to build for ${#failed_targets[@]} targets:"
    printf '  %s\n' "${failed_targets[@]}"
fi

echo ""
log_info "Binaries available in: $OUTPUT_DIR"
ls -la "$OUTPUT_DIR/"

# Exit with error if any builds failed
if [[ ${#failed_targets[@]} -gt 0 ]]; then
    exit 1
fi

log_success "All builds completed successfully!"

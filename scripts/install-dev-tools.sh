#!/usr/bin/env bash
set -euo pipefail

echo "Installing development tools..."

# Install cargo-binstall if not already present
if ! command -v cargo-binstall &> /dev/null; then
    echo "Installing cargo-binstall..."
    cargo install cargo-binstall
fi

# Use cargo-binstall to install Rust dev tools
echo "Installing Rust dev tools via cargo-binstall..."
cargo binstall -y cargo-nextest cargo-insta insta-cmd taplo-cli cargo-llvm-cov

# Install bencher CLI for benchmarking
if ! command -v bencher &> /dev/null; then
    echo "Installing bencher CLI via install script..."
    curl --proto '=https' --tlsv1.2 -sSfL https://bencher.dev/download/install-cli.sh | sh
else
    echo "✓ bencher CLI installed successfully"
fi

# Install uv for Python package management
if ! command -v uv &> /dev/null; then
    echo "Installing uv (Python package manager)..."
    curl -LsSf https://astral.sh/uv/install.sh | sh
else
    echo "✓ uv already installed"
fi

# Install bun for Node.js package management
if ! command -v bun &> /dev/null; then
    echo "Installing bun (JavaScript runtime)..."
    curl -fsSL https://bun.sh/install | bash
else
    echo "✓ bun already installed"
fi

# Install lefthook for git hooks
if ! command -v lefthook &> /dev/null; then
    echo "Installing lefthook (git hooks manager)..."
    if command -v brew &> /dev/null; then
        brew install lefthook
    else
        curl -1sLf 'https://dl.cloudsmith.io/public/evilmartians/lefthook/setup.bash.sh' | sudo -E bash
        sudo apt-get install -y lefthook || sudo yum install -y lefthook || echo "⚠ Could not install lefthook via package manager, please install manually"
    fi
else
    echo "✓ lefthook already installed"
fi

# Install actionlint for GitHub Actions validation
if ! command -v actionlint &> /dev/null; then
    echo "Installing actionlint (GitHub Actions linter)..."
    if command -v brew &> /dev/null; then
        brew install actionlint
    else
        bash <(curl https://raw.githubusercontent.com/rhysd/actionlint/main/scripts/download-actionlint.bash)
        echo "⚠ actionlint downloaded to current directory, please move to PATH manually"
    fi
else
    echo "✓ actionlint already installed"
fi

echo "✓ All development tools installed successfully!"

#!/bin/bash

# Claude Code PostToolUse hook for ruff format
# WE ARE NOT LINTING HERE, JUST FORMATTING!
# Automatically runs ruff check and fix on Python files after Write/Edit operations
# Configure this as a PostToolUse hook with matcher: "Write|Edit|MultiEdit"

# Read JSON from stdin
input=$(cat)

# Extract file path from the input JSON
file_path=$(echo "$input" | jq -r '.tool_input.file_path // empty')

# Only process Python files
if [[ "$file_path" =~ \.py$ ]]; then
  uv tool run ruff format --config $CLAUDE_PROJECT_DIR/pyproject.toml "$file_path" 2>&1 || echo "ruff format failed"
fi

# # we will format only fixtures at crates/cribo/tests/fixtures
# # Check if it's a Python file at crates/cribo/tests/fixtures
# if [[ "$FILE_PATH" != *crates/cribo/tests/fixtures* ]]; then
#     # Not in crates/cribo/tests/fixtures, skipping
#     exit 0
# fi

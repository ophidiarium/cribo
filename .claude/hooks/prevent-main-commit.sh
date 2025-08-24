#!/bin/bash

# Hook to prevent commits to main branch

# Get current branch
CURRENT_BRANCH=$(git branch --show-current --no-color --quiet 2>/dev/null)

if [[ "$CURRENT_BRANCH" == "main" ]]; then
    cat <<EOF
{
  "decision": "block",
  "message": "❌ Commits to main branch are not allowed. Please create a feature branch first.",
  "additionalContext": "Use 'git checkout -b feature/branch-name origin/main' to create a new branch"
}
EOF
    exit 1
fi


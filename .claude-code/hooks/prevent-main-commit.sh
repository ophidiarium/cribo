#!/bin/bash

# Hook to prevent commits to main branch
# This blocks git commit commands when on main branch

# Debug logging to stderr (won't interfere with JSON output)
echo "Hook triggered: TOOL_NAME=$TOOL_NAME" >&2
echo "TOOL_PARAMS=$TOOL_PARAMS" >&2

# Check if this is a Bash tool call
if [[ "$TOOL_NAME" != "Bash" ]]; then
  exit 0
fi

# Parse the command from TOOL_PARAMS (JSON)
COMMAND=$(echo "$TOOL_PARAMS" | jq -r '.command // ""')
echo "Parsed command: $COMMAND" >&2

# Check if it's a git commit command
if [[ "$COMMAND" =~ ^git[[:space:]]+commit ]]; then
  # Get current branch
  CURRENT_BRANCH=$(git branch --show-current 2>/dev/null)
  
  if [[ "$CURRENT_BRANCH" == "main" ]]; then
    cat <<EOF
{
  "decision": "block",
  "message": "❌ Commits to main branch are not allowed. Please create a feature branch first.",
  "additionalContext": "Use 'git checkout -b feature/branch-name' to create a new branch"
}
EOF
    exit 1
  fi
fi

# Allow all other commands
exit 0
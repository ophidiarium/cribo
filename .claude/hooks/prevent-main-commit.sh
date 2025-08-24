#!/bin/bash

# Hook to prevent commits to main branch

json_input=$(cat)

tool_input_command=$(echo "$json_input" | jq -r '
if .tool_input.command then
    .tool_input.command
else
    empty
end')

if [ -z "$tool_input_command" ]; then
    exit 0
fi

# Helper function to output JSON response
output_json() {
  local decision="$1"
  local reason="$2"
  jq -n \
    --arg decision "$decision" \
    --arg reason "$reason" \
    '{
      hookSpecificOutput: {
        hookEventName: "PreToolUse",
        permissionDecision: $decision,
        permissionDecisionReason: $reason
      }
    }'
}

if [[ $tool_input_command =~ "git commit" ]]; then
  # Get current branch
  CURRENT_BRANCH=$(git branch --show-current --no-color --quiet 2>/dev/null)

  # see https://docs.anthropic.com/en/docs/claude-code/hooks#advanced%3A-json-output
  if [[ "$CURRENT_BRANCH" =~ ^(main|master)$ ]]; then
    output_json "ask" " ⚠️ You are about to commit directly to the $CURRENT_BRANCH branch. Are you sure?"
  fi
fi

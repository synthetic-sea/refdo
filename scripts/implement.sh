#!/bin/sh
set -eu

launch_agent_in_worktree() {
  if [ "$#" -ne 1 ]; then
    printf 'Error: internal launch requires a worktree path.\n' >&2
    exit 2
  fi

  worktree_path=$1
  cd "$worktree_path"

  opened=$(herdr worktree open \
    --workspace "$HERDR_PARENT_WORKSPACE_ID" \
    --path "$PWD" \
    --label "$WT_TARGET_BRANCH" \
    --focus)

  pane_id=$(printf '%s\n' "$opened" | jq -er '.result.root_pane.pane_id')
  quoted_agent=$(printf '%s' "$AGENT_EXECUTABLE" | jq -Rrs @sh)
  quoted_prompt=$(printf '%s' "$INITIAL_PROMPT" | jq -Rrs @sh)

  herdr pane run "$pane_id" "$quoted_agent $quoted_prompt"
}

if [ "${1:-}" = "--launch-worktree" ]; then
  shift
  launch_agent_in_worktree "$@"
  exit 0
fi

if [ "$#" -ne 3 ]; then
  printf 'Usage: %s <branch-name> <agent> <initial-prompt>\n' "$0" >&2
  exit 2
fi

if [ "${HERDR_ENV:-}" != 1 ] || [ -z "${HERDR_WORKSPACE_ID:-}" ]; then
  printf 'Error: this script must be executed from inside a Herdr workspace terminal.\n' >&2
  exit 1
fi

branch=$1
agent=$2
prompt=$3

export HERDR_PARENT_WORKSPACE_ID="$HERDR_WORKSPACE_ID"
export WT_TARGET_BRANCH="$branch"
export AGENT_EXECUTABLE="$agent"
export INITIAL_PROMPT="$prompt"

wt switch \
  --create \
  --no-cd \
  --execute sh \
  "$branch" \
  -- \
  "$0" \
  --launch-worktree \
  "{{ worktree_path }}"


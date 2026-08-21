# refdo

refdo is a Git-aware terminal todo manager. It organizes todos by branches checked out in a repository's worktrees; branches with saved todos remain listed, and all worktrees share the same repository-local data.

## Install and run

You need a Rust toolchain. Git is required for normal use.

Install from this checkout:

```sh
cargo install --path .
```

From a Git repository or worktree, start refdo with:

```sh
refdo
```

For development, run the checkout directly:

```sh
cargo run --release
```

Always launch refdo from within the Git repository or worktree whose todos you want to manage.

## Quick workflow

1. Move through rows with `j`/`k` or jump between branch headers with `]`/`[`.
2. Press `o` on a branch to add a todo at its start, or on a todo to add one after it.
3. Type the todo and press `Enter`. Creation stays open so you can enter several todos quickly; press `Esc` when finished.
4. Press `Space` or `x` to mark the selected todo complete.
5. Use `dd` to cut a todo and `p` or `P` to paste it elsewhere.
6. Press `q` to quit.

## Keys

### Normal mode

| Key | Effect |
| --- | --- |
| `j` / `Down` | Select the next row. |
| `k` / `Up` | Select the previous row. |
| `]` | Select the next branch. |
| `[` | Select the current branch header from a todo, or the previous branch from a branch header. |
| `o` | Create a todo at the start of the selected branch, or after the selected todo. `Enter` saves it and remains in create mode for rapid entry. |
| `i` | Edit the selected todo. |
| `x` / `Space` | Toggle the selected todo between incomplete and complete. |
| `yy` | Yank the selected todo into the internal register and copy its text to the system clipboard. |
| `dd` | Cut the selected todo into the internal register. The todo is deleted until it is pasted. |
| `p` | Paste the registered todo below the selection; on a branch header, paste at the end of that branch. |
| `P` | Paste the registered todo above the selection; on a branch header, paste at the start of that branch. |
| `:` | Open colon-command entry. |
| `Esc` | Clear the current selection. |
| `q` | Quit refdo. |

### Text entry

These controls apply while creating or editing a todo and while entering a command.

| Key | Effect |
| --- | --- |
| Any text | Insert text at the cursor. |
| `Left` / `Right` | Move the cursor by one character. |
| `Ctrl+Left` / `Ctrl+Right` | Move the cursor backward or forward by one word. |
| `Shift+Left` / `Shift+Right` | Move the cursor backward or forward by one word. |
| `Home` / `End` | Move to the start or end of the text. |
| `Backspace` | Delete the character before the cursor. |
| `Delete` | Delete the character at the cursor. |
| `Enter` | Submit the text or command. |
| `Esc` | Cancel text entry. |

### Colon commands

The built-in commands affect the selected todo's branch, or the selected branch when a branch header is selected. Dispatches require a selected todo in a Git worktree.

| Command | Effect |
| --- | --- |
| `:prune` | Delete all completed todos in the affected branch. |
| `:sort` | Put incomplete todos before completed todos, ordering each group by creation time. |
| `:group` | Put incomplete todos before completed todos while preserving the current order within each group. |
| `:clear` | Ask for confirmation, then delete every todo in the affected branch. |
| `:dispatch <name>` | Run the configured named dispatch for the selected todo. |
| `:dispatch-trust` | Review and approve the selected todo's committed `.refdo.toml`. |

When confirmation is shown, press `y` or `Y` to confirm. Press `n`, `N`, `Enter`, or `Esc` to cancel.

### Mouse

In normal mode, click a row to select it, or double-click a todo's text to edit it with the cursor at the clicked location. Use the mouse wheel over the todo list to scroll.

## Configuration

On first startup, refdo creates `config.toml` in the platform-standard configuration directory:

- Linux: `$XDG_CONFIG_HOME/refdo/config.toml`, or `~/.config/refdo/config.toml` when `XDG_CONFIG_HOME` is unset
- macOS: `~/.config/refdo/config.toml`
- Windows: `%APPDATA%\refdo\config.toml`

Theme selection is configured with:

```toml
[theme]
light = "tokyo-night-day"
dark = "tokyo-night"
mode = "system"
```

`light` and `dark` select the built-in themes used for each appearance. Setting `mode` to `light` or `dark` keeps that appearance fixed; `system` follows operating-system appearance changes while refdo is running and falls back to light on startup only when the appearance cannot be detected.

### Named dispatches

Named dispatches are repository capabilities committed in `.refdo.toml` at the repository root. For every `:dispatch` or `:dispatch-trust`, refdo reads the file from the selected todo's worktree; it does not use the worktree from which refdo was launched. Relative command and script paths are also resolved from that selected worktree, so branches may intentionally carry different definitions. Changes are read on demand and do not require restarting refdo.

#### Implementing a todo with OMP, Worktrunk, and Herdr

The following example generates a branch name with a one-shot OMP invocation, creates a worktree with [Worktrunk](https://worktrunk.dev/), opens that worktree in Herdr, and launches an OMP agent with the todo title as its initial prompt.

It requires:

- `omp`, configured with access to the selected model;
- `wt` from Worktrunk;
- the `herdr` CLI and a refdo process launched inside a Herdr workspace terminal;
- `jq`; and
- Bash, which refdo uses to execute dispatch commands.

Keep the theme and optional branch-name generator in the platform `config.toml` described above. For this example, add the global generator:

```toml
[dispatch]
generate_branch_name_command = "omp -p --no-session --no-tools --no-extensions --no-skills --no-rules --no-lsp --no-title --max-time 30s --model gemini-3.7-flash --thinking low --system-prompt 'Generate a Git branch name from the user message. Output exactly one non-empty line and nothing else. The name must pass git check-ref-format --branch. Use lowercase ASCII letters, digits, hyphens, and forward slashes. Prefer a prefix such as feat/, fix/, docs/, refactor/, test/, or chore/. Maximum 60 characters. Do not output Markdown, quotes, explanation, or trailing punctuation.' -- {{CONTENT}}"
```

Then add the named definition to `.refdo.toml` in the repository root:

```toml
[dispatches.implement]
command = './scripts/implement.sh {{BRANCH}} omp {{CONTENT}}'
```

The `-p` flag makes OMP process the prompt non-interactively, print its response, and exit. `--no-session` avoids retaining a branch-generation session, while `--no-tools` and the other `--no-*` flags keep this small request isolated from repository tools and customization. `--max-time 30s` prevents the generator from remaining open indefinitely. The `--` before `{{CONTENT}}` ends option parsing, so a todo title beginning with `-` remains input data.

Place the following executable at `scripts/implement.sh` in the repository:

```sh
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
```

Make the script executable:

```sh
chmod +x scripts/implement.sh
```

Commit the definition and the script together so the capability travels with the repository:

```sh
git add .refdo.toml scripts/implement.sh
```

On first use, select a todo in the target worktree and enter `:dispatch implement`. When refdo reports that the repository configuration is untrusted, inspect the committed `.refdo.toml`, enter `:dispatch-trust`, and press `y` to confirm. Trusting does not run the dispatch: enter `:dispatch implement` again. The global generator then receives the todo title and emits a single branch name. refdo runs the selected worktree's `./scripts/implement.sh` with exactly three arguments: the generated branch name, `omp`, and the literal todo title. The script creates the worktree, opens it in Herdr, and starts OMP in its root pane.

#### Templates and execution

Dispatch commands support `{{CONTENT}}`, which is the selected todo title, and `{{BRANCH}}`, which is the generator's output. Placeholders must appear unquoted in the configured shell source. refdo replaces them with quoted Bash positional parameters (`"$1"` and `"$2"`), so spaces, quotes, newlines, substitutions, and other shell metacharacters in their values remain data rather than being reparsed as commands.

The platform `generate_branch_name_command` is optional and supports `{{CONTENT}}` but not `{{BRANCH}}`. It runs first, in the selected todo's worktree, only when the selected dispatch contains `{{BRANCH}}`. Its stdout must be valid UTF-8 containing exactly one non-empty line after trimming. A dispatch without `{{BRANCH}}` does not invoke the generator; for example, `.refdo.toml` could contain:

```toml
[dispatches.notify]
command = 'printf "%s\n" {{CONTENT}} > dispatch-result.txt'
```

A dispatch requires a selected todo whose branch has a worktree and an explicitly trusted `.refdo.toml`; missing, invalid, empty, or untrusted repository configuration does not fall back to global definitions. Each distinct SHA-256 hash of the exact `.refdo.toml` bytes must be trusted once. Editing the file—including changing only comments or whitespace—changes the hash and blocks dispatch until the new bytes are trusted, without requiring a refdo restart. Trust is stored in the repository database, so all local worktrees share the set of previously approved hashes.

Trust approves only the `.refdo.toml` definition bytes. It does not approve, authenticate, or pin scripts and other files that a command invokes transitively. Dispatch definitions execute as unsandboxed, trusted Bash through `bash -lc`, and refdo runs one only after an explicit `:dispatch <name>` invocation; review both the definition and everything it can execute before trusting and invoking it.

Only one dispatch may run at a time. Execution is asynchronous, so refdo remains interactive while the footer reports running and completion status. Subprocess stdout and stderr are captured rather than inherited by the TUI: generator stdout supplies the branch name, while dispatch stdout is not displayed. A nonzero process reports the first non-empty stderr line when available or its exit status otherwise; startup, working-directory, and invalid generator-output errors are reported directly.

## Storage

refdo stores its database at `<git-common-dir>/refdo/data.db`. The database is shared by the repository's worktrees, while each listed branch has its own todos. Because it lives in Git's common directory rather than the working tree, the database is not committed to Git. refdo periodically refreshes live worktree and HEAD state while running, keeps todo-backed removed branches as stored-only, and retains the last known repository view during a temporary discovery failure.

No persistence is available when refdo is run outside a Git repository.

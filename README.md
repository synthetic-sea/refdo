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
| `dd` | Cut the selected todo into an in-session buffer. The todo is deleted until it is pasted. |
| `p` | Paste the cut todo below the selection; on a branch header, paste at the end of that branch. |
| `P` | Paste the cut todo above the selection; on a branch header, paste at the start of that branch. |
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

Commands affect the selected todo's branch, or the selected branch when a branch header is selected.

| Command | Effect |
| --- | --- |
| `:prune` | Delete all completed todos in the affected branch. |
| `:sort` | Put incomplete todos before completed todos, ordering each group by creation time. |
| `:group` | Put incomplete todos before completed todos while preserving the current order within each group. |
| `:clear` | Ask for confirmation, then delete every todo in the affected branch. |

When confirmation is shown, press `y` or `Y` to confirm. Press `n`, `N`, `Enter`, or `Esc` to cancel.

### Mouse

In normal mode, click a row to select it. Use the mouse wheel over the todo list to scroll.

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

## Storage

refdo stores its database at `<git-common-dir>/refdo/data.db`. The database is shared by the repository's worktrees, while each listed branch has its own todos. Because it lives in Git's common directory rather than the working tree, the database is not committed to Git.

No persistence is available when refdo is run outside a Git repository.

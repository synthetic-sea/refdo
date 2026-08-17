# Plan 001: Copy the selected todo to the system clipboard

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat c71361d..HEAD -- Cargo.toml Cargo.lock README.md src/app/mod.rs src/app/actions.rs src/app/events.rs src/app/tests/support.rs src/app/tests/actions.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" facts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: direction
- **Planned at**: commit `c71361d`, 2026-08-17

## Why this matters

Users need to move todo text from refdo into editors, browsers, chat clients, and other applications without retyping it. OSC 52 is the terminal-native mechanism: it asks the terminal host to populate its system clipboard, works in remote sessions when supported, and avoids a platform-specific GUI clipboard dependency. The normal-mode `y` binding matches refdo's existing Vim-like `dd`, `p`, and `P` vocabulary.

## Current state

- `Cargo.toml:6-15` depends on Ratatui 0.30.2; `Cargo.lock:519-524` locks Ratatui's Crossterm dependency at 0.29.0. Crossterm's `CopyToClipboard` API is feature-gated behind `osc52`.
- `src/app/mod.rs:101-119` owns application state, including `error: Option<String>`, which the footer already displays as a generic transient message.
- `src/app/mod.rs:237-251` owns terminal I/O in `App::run`; terminal escape-sequence emission belongs here, not in an action.
- `src/app/actions.rs:39-49` demonstrates resolving a focused todo with `Focus::Todo(id)` and finding the matching `Todo`; the text to copy is `Todo.title`.
- `src/app/events.rs:71-97` maps normal-mode keys. `dd` cuts and `p`/`P` paste; lowercase `y` is currently unused in normal mode. Confirmation mode independently accepts `y`/`Y` and must remain unchanged.
- `src/app/tests/support.rs:27-54` constructs `App` directly and must initialize any new field.
- `src/app/tests/actions.rs` exercises normal-mode actions through `handle_key_event`; match this style.
- `README.md:40-56` documents the normal-mode key table.

The implementation must keep actions free of global stdout writes. Model the clipboard operation as pending application state and flush it at the terminal boundary.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Format check | `cargo fmt --check` | exit 0, no diff |
| Focused tests | `cargo test app::tests::actions` | all action tests pass |
| Full tests | `cargo test` | all tests pass |
| Static analysis | `cargo clippy --all-targets --all-features -- -A clippy::double_ended_iterator_last -A clippy::too_many_arguments -A clippy::derivable_impls -A clippy::result_large_err -D warnings` | exit 0, no warnings beyond four explicitly allowed pre-existing lints |

## Scope

**In scope** (the only files you should modify):
- `Cargo.toml`
- `Cargo.lock`
- `README.md`
- `src/app/mod.rs`
- `src/app/actions.rs`
- `src/app/events.rs`
- `src/app/tests/support.rs`
- `src/app/tests/actions.rs`

**Out of scope**:
- Native clipboard crates such as `arboard`.
- Copying branch names, status markers, database metadata, or multiple todos.
- New notification abstractions or renaming the existing `error` field.
- Changing insert, command, or clear-confirmation mode behavior.
- Clipboard read/paste support.

## Git workflow

- Branch: `advisor/001-copy-todo-to-clipboard`.
- Use the repository's conventional commit style; commit as `feat(app): copy todo text to clipboard`.
- Do not push or open a pull request.

## Steps

### Step 1: Enable Crossterm OSC 52 support

Add a direct, version-aligned dependency in `Cargo.toml`:

```toml
crossterm = { version = "0.29.0", features = ["osc52"] }
```

Keep Ratatui's re-exported Crossterm types where already used, but import `CopyToClipboard` through the direct `crossterm` dependency. Refresh `Cargo.lock` using Cargo; do not hand-edit it.

**Verify**: `cargo check` → exit 0.

### Step 2: Add the copy request and terminal-boundary flush

In `App`, add `clipboard_request: Option<String>` and initialize it to `None` in both production and test constructors.

Add `copy_focused_todo()` in `src/app/actions.rs`. It must:

1. Return without changing state unless focus is `Focus::Todo(id)` and that todo exists.
2. Clone exactly `todo.title` into `clipboard_request`.
3. Not mutate the todo, persistence, focus, mode, or cut buffer.

Add a small `App` helper in `src/app/mod.rs` that accepts a generic `std::io::Write`, takes the pending request, and executes `CopyToClipboard::to_clipboard_from(text)` against that writer. On a successful writer operation, set the footer message to `Copied todo text`. On an I/O failure, set it to `copy: {error}`. Consume the request in either case so it is not emitted repeatedly.

Call this helper in `App::run` immediately after `handle_events()` so all terminal writes stay at the existing terminal boundary. An OSC 52 write succeeding only means the sequence was emitted; do not claim that the terminal accepted it.

**Verify**: `cargo check` → exit 0.

### Step 3: Bind and document normal-mode yank

Map lowercase `y` in `handle_normal_key` to `copy_focused_todo()`. Preserve `pending_cut` reset behavior and confirmation-mode `y` handling. Add `y` to the README normal-mode table as copying the selected todo text to the system clipboard.

**Verify**: `cargo fmt --check` → exit 0.

### Step 4: Add behavioral tests

In `src/app/tests/actions.rs`, add tests that prove:

- Pressing `y` with a focused Unicode todo queues exactly its title and does not mutate `todos`, the store, focus, completion state, or cut buffer.
- Pressing `y` on a branch or with no focus queues nothing.
- Flushing a pending request into an in-memory byte buffer consumes the request, emits a non-empty OSC 52 sequence containing the base64-encoded todo text, and sets `Copied todo text`.
- Flushing through a deliberately failing `Write` implementation consumes the request and sets a footer message beginning with `copy: `.

Do not test Crossterm internals more precisely than needed; the purpose of the buffer assertion is to prove refdo actually emits the queued command.

**Verify**: `cargo test app::tests::actions` → all action tests pass.

## Test plan

Use `src/app/tests/actions.rs` and its `app_with_sections`, store insertion, focus assignment, and `handle_key_event` conventions. Tests must cover the normal success path, Unicode preservation, non-todo focus, no focus, request consumption, emitted output, and writer failure. Then run the entire suite to catch constructor or interaction regressions.

## Done criteria

- [ ] `cargo fmt --check` exits 0.
- [ ] `cargo test app::tests::actions` exits 0.
- [ ] `cargo test` exits 0.
- [ ] `cargo clippy --all-targets --all-features -- -A clippy::double_ended_iterator_last -A clippy::too_many_arguments -A clippy::derivable_impls -A clippy::result_large_err -D warnings` exits 0; all non-baseline warnings remain denied.
- [ ] `y` copies exactly the selected todo title through a queued OSC 52 command.
- [ ] Branch/no focus is a no-op and copying never changes persisted todo data.
- [ ] Success and writer failure produce the specified footer messages.
- [ ] README documents the new key.
- [ ] No files outside the in-scope list are modified.

## STOP conditions

Stop and report instead of improvising if:

- Any in-scope current-state fact above no longer matches the live code.
- Crossterm 0.29.0 cannot be enabled through Cargo feature unification with Ratatui's locked dependency.
- Ratatui's Crossterm backend cannot be used as the `Write` target for the command without changing terminal ownership or architecture.
- The implementation requires a native clipboard crate, platform-specific code, or changes outside the in-scope files.
- A verification command still fails after one reasonable correction.

## Maintenance notes

OSC 52 requires support from the user's terminal and any multiplexer in the path. Refdo can detect writer errors but cannot detect whether a terminal ignored a valid sequence. If a future product requirement prioritizes unsupported local terminals over SSH behavior, evaluate a native clipboard fallback separately rather than mixing platform code into this path.

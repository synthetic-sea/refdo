use std::path::{Path, PathBuf};

use crate::{
    config::{LoadedDispatchConfig, load_repository_dispatch_config},
    storage::TodoId,
};

use super::{App, ClearConfirmation, CommandLine, DispatchTrustConfirmation, Focus, Mode};

struct ResolvedDispatchTarget {
    content: String,
    display_name: String,
    worktree_path: PathBuf,
}

impl App {
    pub(in crate::app) fn open_command_line(&mut self) {
        let target_branch = match self.focus.as_ref() {
            Some(Focus::Branch(branch_ref))
                if self
                    .repository
                    .sections
                    .iter()
                    .any(|section| section.full_ref_name == *branch_ref) =>
            {
                Some(branch_ref.clone())
            }
            Some(Focus::Todo(id)) => self
                .todos
                .iter()
                .find(|todo| todo.id == *id)
                .map(|todo| todo.branch_ref.clone()),
            _ => None,
        };
        let target_todo = match self.focus.as_ref() {
            Some(Focus::Todo(id)) if self.todos.iter().any(|todo| todo.id == *id) => Some(*id),
            _ => None,
        };
        self.mode = Mode::Command(CommandLine {
            target_branch,
            target_todo,
            text: String::new(),
            cursor: 0,
        });
        self.pending_operator = None;
        self.error = None;
    }

    pub(in crate::app) fn discard_command_line(&mut self) {
        self.mode = Mode::Normal;
        self.error = None;
    }

    fn begin_clear_confirmation(&mut self, target_branch: String) {
        let display_name = self
            .repository
            .sections
            .iter()
            .find(|section| section.full_ref_name == target_branch)
            .map(|section| section.display_name.as_str())
            .unwrap_or(&target_branch);
        let prompt = format!("clear: remove all items from {display_name}? [y/N]");
        self.mode = Mode::ConfirmClear(ClearConfirmation {
            target_branch,
            prompt,
        });
    }

    pub(in crate::app) fn discard_clear_confirmation(&mut self) {
        self.mode = Mode::Normal;
        self.error = None;
    }

    pub(in crate::app) fn confirm_clear(&mut self) {
        let Mode::ConfirmClear(confirmation) = &self.mode else {
            return;
        };
        let target_branch = confirmation.target_branch.clone();
        self.mode = Mode::Normal;

        match self.store.delete_all_todos(&target_branch) {
            Ok(count) => {
                if !self.reload() {
                    return;
                }
                self.focus = Some(Focus::Branch(target_branch));
                self.error = Some(format!("clear: removed {count} items"));
            }
            Err(error) => self.error = Some(format!("clear: {error}")),
        }
    }

    pub(in crate::app) fn execute_command_line(&mut self) {
        let Mode::Command(command) = &self.mode else {
            return;
        };
        let name = command.text.trim().to_owned();
        let target_branch = command.target_branch.clone();
        let target_todo = command.target_todo;
        self.mode = Mode::Normal;

        if name.is_empty() {
            self.error = None;
            return;
        }

        let mut tokens = name.split_whitespace();
        match tokens.next() {
            Some("dispatch") => {
                let Some(dispatch_name) = tokens.next() else {
                    self.error = Some("dispatch: expected :dispatch <name>".to_owned());
                    return;
                };
                if tokens.next().is_some() {
                    self.error = Some("dispatch: expected :dispatch <name>".to_owned());
                    return;
                }
                self.start_dispatch(dispatch_name, target_todo, target_branch);
                return;
            }
            Some("dispatch-trust") => {
                if tokens.next().is_some() {
                    self.error = Some("dispatch-trust: expected :dispatch-trust".to_owned());
                    return;
                }
                self.begin_dispatch_trust(target_todo, target_branch);
                return;
            }
            Some(_) | None => {}
        }

        if name != "prune" && name != "sort" && name != "group" && name != "clear" {
            self.error = Some(format!("Unknown command: {name}"));
            return;
        }
        if !self.persistence_available {
            self.error = Some(format!("{name}: persistence unavailable"));
            return;
        }
        let Some(target_branch) = target_branch else {
            self.error = Some(format!("{name}: no focused branch"));
            return;
        };
        if name == "clear" {
            self.begin_clear_confirmation(target_branch);
            return;
        }

        let previous_focus = self.focus.clone();
        let result = match name.as_str() {
            "prune" => self
                .store
                .delete_completed_todos(&target_branch)
                .map(|count| format!("prune: removed {count} completed items")),
            "sort" => self
                .store
                .sort_todos(&target_branch)
                .map(|count| format!("sort: sorted {count} items")),
            "group" => self
                .store
                .group_todos(&target_branch)
                .map(|count| format!("group: grouped {count} items")),
            _ => unreachable!("command name was validated"),
        };
        match result {
            Ok(message) => {
                if !self.reload() {
                    return;
                }
                let focus_survives = previous_focus.as_ref().is_some_and(|focus| match focus {
                    Focus::Branch(branch_ref) => self
                        .repository
                        .sections
                        .iter()
                        .any(|section| section.full_ref_name == *branch_ref),
                    Focus::Todo(id) => self.todos.iter().any(|todo| todo.id == *id),
                });
                self.focus = if focus_survives {
                    previous_focus
                } else {
                    Some(Focus::Branch(target_branch))
                };
                self.error = Some(message);
            }
            Err(error) => self.error = Some(format!("{name}: {error}")),
        }
    }

    fn resolve_dispatch_target(
        &self,
        target_todo: Option<TodoId>,
        target_branch: Option<String>,
    ) -> Result<ResolvedDispatchTarget, &'static str> {
        let target_todo = target_todo.ok_or("dispatch: no todo selected")?;
        let content = self
            .todos
            .iter()
            .find(|todo| todo.id == target_todo)
            .map(|todo| todo.title.clone())
            .ok_or("dispatch: selected todo no longer exists")?;
        let section = target_branch
            .and_then(|branch_ref| {
                self.repository
                    .sections
                    .iter()
                    .find(|section| section.full_ref_name == branch_ref)
            })
            .filter(|section| {
                !section.is_stored_only && !section.worktree_path.as_os_str().is_empty()
            })
            .ok_or("dispatch: selected todo has no worktree")?;
        Ok(ResolvedDispatchTarget {
            content,
            display_name: section.display_name.clone(),
            worktree_path: section.worktree_path.clone(),
        })
    }

    fn load_dispatch_config(worktree_path: &Path) -> Result<LoadedDispatchConfig, String> {
        match load_repository_dispatch_config(worktree_path) {
            Ok(Some(config)) => Ok(config),
            Ok(None) => Err("dispatch: selected todo's worktree has no .refdo.toml".to_owned()),
            Err(error) => Err(format!("dispatch: {error}")),
        }
    }

    fn start_dispatch(
        &mut self,
        name: &str,
        target_todo: Option<TodoId>,
        target_branch: Option<String>,
    ) {
        let target = match self.resolve_dispatch_target(target_todo, target_branch) {
            Ok(target) => target,
            Err(error) => {
                self.error = Some(error.to_owned());
                return;
            }
        };
        if !self.persistence_available {
            self.error = Some("dispatch: repository trust unavailable".to_owned());
            return;
        }
        let loaded = match Self::load_dispatch_config(&target.worktree_path) {
            Ok(loaded) => loaded,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let Some(definition) = loaded.dispatches.get(name).cloned() else {
            self.error = Some(format!("dispatch: unknown dispatch '{name}'"));
            return;
        };
        match self.store.is_dispatch_config_trusted(&loaded.digest) {
            Ok(true) => {}
            Ok(false) => {
                self.error = Some(
                    "dispatch: repository configuration is untrusted; run :dispatch-trust"
                        .to_owned(),
                );
                return;
            }
            Err(error) => {
                self.error = Some(format!("dispatch: {error}"));
                return;
            }
        }

        match self
            .dispatch
            .start(name, definition, target.content, target.worktree_path)
        {
            Ok(()) => self.error = Some(format!("dispatch: running '{name}'")),
            Err(error) => self.error = Some(error),
        }
    }

    fn begin_dispatch_trust(&mut self, target_todo: Option<TodoId>, target_branch: Option<String>) {
        let target = match self.resolve_dispatch_target(target_todo, target_branch) {
            Ok(target) => target,
            Err(error) => {
                self.error = Some(error.to_owned());
                return;
            }
        };
        if !self.persistence_available {
            self.error = Some("dispatch: repository trust unavailable".to_owned());
            return;
        }
        let loaded = match Self::load_dispatch_config(&target.worktree_path) {
            Ok(loaded) => loaded,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        if loaded.dispatches.is_empty() {
            self.error = Some("dispatch: .refdo.toml defines no dispatches".to_owned());
            return;
        }
        match self.store.is_dispatch_config_trusted(&loaded.digest) {
            Ok(true) => {
                self.error = Some(format!(
                    "dispatch: .refdo.toml is already trusted for {}",
                    target.display_name
                ));
            }
            Ok(false) => {
                let prompt = format!(
                    "dispatch: trust .refdo.toml from {}? [y/N]",
                    target.display_name
                );
                self.mode = Mode::ConfirmDispatchTrust(DispatchTrustConfirmation {
                    digest: loaded.digest,
                    display_name: target.display_name,
                    worktree_path: target.worktree_path,
                    prompt,
                });
            }
            Err(error) => self.error = Some(format!("dispatch: {error}")),
        }
    }

    pub(in crate::app) fn discard_dispatch_trust_confirmation(&mut self) {
        self.mode = Mode::Normal;
        self.error = None;
    }

    pub(in crate::app) fn confirm_dispatch_trust(&mut self) {
        let Mode::ConfirmDispatchTrust(confirmation) = &self.mode else {
            return;
        };
        let confirmation = confirmation.clone();
        self.mode = Mode::Normal;

        let loaded = match Self::load_dispatch_config(&confirmation.worktree_path) {
            Ok(loaded) => loaded,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        if loaded.dispatches.is_empty() {
            self.error = Some("dispatch: .refdo.toml defines no dispatches".to_owned());
            return;
        }
        if loaded.digest != confirmation.digest {
            self.error =
                Some("dispatch: .refdo.toml changed; run :dispatch-trust again".to_owned());
            return;
        }
        self.error = Some(match self.store.trust_dispatch_config(&loaded.digest) {
            Ok(()) => format!(
                "dispatch: trusted .refdo.toml for {}",
                confirmation.display_name
            ),
            Err(error) => format!("dispatch: {error}"),
        });
    }
}

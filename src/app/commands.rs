use super::{App, ClearConfirmation, CommandLine, Focus, Mode};

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
        self.mode = Mode::Command(CommandLine {
            target_branch,
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
        self.error = Some(format!(
            "clear: remove all items from {display_name}? [y/N]"
        ));
        self.mode = Mode::ConfirmClear(ClearConfirmation { target_branch });
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
        self.mode = Mode::Normal;

        if name.is_empty() {
            self.error = None;
            return;
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
}

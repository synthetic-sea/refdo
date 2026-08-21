use super::{App, Focus, Mode};

impl App {
    pub(in crate::app) fn repair_focus(&mut self) {
        let Some(focus) = self.focus.as_ref() else {
            return;
        };
        let valid = match focus {
            Focus::Branch(branch_ref) => self
                .repository
                .sections
                .iter()
                .any(|section| section.full_ref_name == *branch_ref),
            Focus::Todo(id) => self.todos.iter().any(|todo| todo.id == *id),
        };
        if !valid {
            self.focus = self.flattened_focuses().into_iter().next();
        }
    }

    pub(in crate::app) fn repair_select_mode(&mut self) {
        let Mode::Select(select_state) = &mut self.mode else {
            return;
        };

        let has_section = self
            .repository
            .sections
            .iter()
            .any(|section| section.full_ref_name == select_state.branch_ref);

        let first_branch_todo_id = self
            .todos
            .iter()
            .find(|todo| todo.branch_ref == select_state.branch_ref)
            .map(|todo| todo.id);

        let Some(first_todo_id) = first_branch_todo_id.filter(|_| has_section) else {
            let exit_focus = has_section.then(|| Focus::Branch(select_state.branch_ref.clone()));
            self.mode = Mode::Normal;
            if let Some(exit_focus) = exit_focus {
                self.focus = Some(exit_focus);
            }
            return;
        };

        select_state.selected_todo_ids.retain(|id| {
            self.todos
                .iter()
                .any(|todo| todo.branch_ref == select_state.branch_ref && todo.id == *id)
        });

        let focus_valid = match self.focus.as_ref() {
            Some(Focus::Todo(id)) => self
                .todos
                .iter()
                .any(|todo| todo.branch_ref == select_state.branch_ref && todo.id == *id),
            _ => false,
        };

        if !focus_valid {
            self.focus = Some(Focus::Todo(first_todo_id));
        }
    }

    pub(in crate::app) fn repair_preview_mode(&mut self) {
        let Mode::Preview(preview) = &self.mode else {
            return;
        };
        let remains_previewable = self
            .todos
            .iter()
            .any(|todo| todo.id == preview.todo_id && !todo.body.is_empty());
        if !remains_previewable {
            self.mode = Mode::Normal;
        }
    }

    pub(in crate::app) fn flattened_focuses(&self) -> Vec<Focus> {
        let mut rows = Vec::with_capacity(self.repository.sections.len() + self.todos.len());
        for section in &self.repository.sections {
            rows.push(Focus::Branch(section.full_ref_name.clone()));
            rows.extend(
                self.todos
                    .iter()
                    .filter(|todo| todo.branch_ref == section.full_ref_name)
                    .map(|todo| Focus::Todo(todo.id)),
            );
        }
        rows
    }

    pub(in crate::app) fn move_focus(&mut self, delta: isize) {
        let rows = self.flattened_focuses();
        self.reveal_focus = true;
        let Some(current) = self
            .focus
            .as_ref()
            .and_then(|focus| rows.iter().position(|row| row == focus))
        else {
            self.focus = if delta < 0 {
                rows.last().cloned()
            } else {
                rows.first().cloned()
            };
            return;
        };
        let next = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current.saturating_add(delta as usize).min(rows.len() - 1)
        };
        self.focus = Some(rows[next].clone());
    }

    pub(in crate::app) fn move_section_focus(&mut self, forward: bool) {
        self.reveal_focus = true;
        let Some((current, on_header)) = self.focus.as_ref().and_then(|focus| match focus {
            Focus::Branch(branch_ref) => self
                .repository
                .sections
                .iter()
                .position(|section| section.full_ref_name == *branch_ref)
                .map(|index| (index, true)),
            Focus::Todo(id) => self
                .todos
                .iter()
                .find(|todo| todo.id == *id)
                .and_then(|todo| {
                    self.repository
                        .sections
                        .iter()
                        .position(|section| section.full_ref_name == todo.branch_ref)
                })
                .map(|index| (index, false)),
        }) else {
            self.focus = if forward {
                self.repository.sections.first()
            } else {
                self.repository.sections.last()
            }
            .map(|section| Focus::Branch(section.full_ref_name.clone()));
            return;
        };

        let target = if forward {
            current.checked_add(1)
        } else if on_header {
            current.checked_sub(1)
        } else {
            Some(current)
        };
        if let Some(section) = target.and_then(|index| self.repository.sections.get(index)) {
            self.focus = Some(Focus::Branch(section.full_ref_name.clone()));
        }
    }

    pub(in crate::app) fn move_focus_within_branch(&mut self, branch_ref: &str, delta: isize) {
        self.reveal_focus = true;
        let mut branch_todos = self
            .todos
            .iter()
            .filter(|todo| todo.branch_ref == branch_ref);
        let Some(first_todo) = branch_todos.next() else {
            return;
        };

        let mut count: usize = 1;
        let mut current_pos: Option<usize> = if matches!(self.focus, Some(Focus::Todo(id)) if id == first_todo.id)
        {
            Some(0)
        } else {
            None
        };

        for todo in branch_todos {
            if current_pos.is_none() && matches!(self.focus, Some(Focus::Todo(id)) if id == todo.id)
            {
                current_pos = Some(count);
            }
            count += 1;
        }

        let target_pos = match current_pos {
            Some(current) => {
                if delta < 0 {
                    current.saturating_sub(delta.unsigned_abs())
                } else {
                    current.saturating_add(delta as usize).min(count - 1)
                }
            }
            None => {
                if delta < 0 {
                    0
                } else {
                    (delta as usize).min(count - 1)
                }
            }
        };

        if let Some(target_todo) = self
            .todos
            .iter()
            .filter(|todo| todo.branch_ref == branch_ref)
            .nth(target_pos)
        {
            self.focus = Some(Focus::Todo(target_todo.id));
        }
    }
}

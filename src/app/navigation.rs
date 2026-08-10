use super::{App, Focus};

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
}

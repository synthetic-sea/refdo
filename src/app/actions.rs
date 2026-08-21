use std::collections::HashSet;

use super::{App, Editor, EditorTarget, Focus, Mode, SelectState};

impl App {
    pub(in crate::app) fn copy_focused_todo(&mut self) {
        let Some(Focus::Todo(id)) = self.focus.as_ref() else {
            return;
        };
        let Some(todo) = self.todos.iter().find(|todo| todo.id == *id).cloned() else {
            return;
        };
        self.clipboard_request = Some(todo.title.clone());
        self.todo_register = Some(todo);
    }

    pub(in crate::app) fn open_create_editor(&mut self) {
        if !self.persistence_available {
            return;
        }

        let Some(origin) = self.focus.clone() else {
            return;
        };
        let (branch_ref, after) = match &origin {
            Focus::Branch(branch_ref) => (branch_ref.clone(), None),
            Focus::Todo(id) => {
                let Some(todo) = self.todos.iter().find(|todo| todo.id == *id) else {
                    return;
                };
                (todo.branch_ref.clone(), Some(*id))
            }
        };
        self.mode = Mode::Insert(Editor {
            target: EditorTarget::Create {
                branch_ref,
                after,
                origin,
            },
            text: String::new(),
            cursor: 0,
        });
        self.reveal_focus = true;
        self.error = None;
    }

    pub(in crate::app) fn open_update_editor(&mut self, cursor: Option<usize>) {
        if !self.persistence_available {
            return;
        }

        let Some(Focus::Todo(id)) = self.focus.as_ref() else {
            return;
        };
        let Some(todo) = self.todos.iter().find(|todo| todo.id == *id) else {
            return;
        };
        let cursor = cursor
            .filter(|cursor| todo.title.is_char_boundary(*cursor))
            .unwrap_or(todo.title.len());
        self.mode = Mode::Insert(Editor {
            target: EditorTarget::Update { id: *id },
            text: todo.title.clone(),
            cursor,
        });
        self.reveal_focus = true;
        self.error = None;
    }

    pub(in crate::app) fn discard_editor(&mut self) {
        if let Mode::Insert(Editor {
            target: EditorTarget::Create { origin, .. },
            ..
        }) = &self.mode
        {
            self.focus = Some(origin.clone());
        }
        self.mode = Mode::Normal;
        self.error = None;
        self.reveal_focus = true;
    }

    pub(in crate::app) fn commit_editor(&mut self) {
        let Mode::Insert(editor) = &self.mode else {
            return;
        };
        let target = editor.target.clone();
        let text = editor.text.clone();

        match target {
            EditorTarget::Create {
                branch_ref, after, ..
            } => {
                if text.trim().is_empty() {
                    self.discard_editor();
                    return;
                }
                match self.store.insert_todo(&branch_ref, &text, after) {
                    Ok(todo) => {
                        let committed = Focus::Todo(todo.id);
                        let branch_ref = todo.branch_ref.clone();
                        let todo_id = todo.id;
                        self.integrate_todo(todo);
                        self.focus = Some(committed.clone());
                        self.mode = Mode::Insert(Editor {
                            target: EditorTarget::Create {
                                branch_ref,
                                after: Some(todo_id),
                                origin: committed,
                            },
                            text: String::new(),
                            cursor: 0,
                        });
                        if self.reload() {
                            self.error = None;
                        }
                        self.reveal_focus = true;
                    }
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
            EditorTarget::Update { id } => match self.store.update_todo_title(id, &text) {
                Ok(todo) => {
                    if let Some(existing) = self.todos.iter_mut().find(|todo| todo.id == id) {
                        *existing = todo;
                    }
                    self.focus = Some(Focus::Todo(id));
                    self.mode = Mode::Normal;
                    self.error = None;
                    self.reveal_focus = true;
                }
                Err(error) => self.error = Some(error.to_string()),
            },
        }
    }

    pub(in crate::app) fn toggle_focused_todo(&mut self) {
        if !self.persistence_available {
            return;
        }

        let Some(Focus::Todo(id)) = self.focus.as_ref() else {
            return;
        };
        let id = *id;
        match self.store.toggle_todo(id) {
            Ok(todo) => {
                if let Some(existing) = self.todos.iter_mut().find(|todo| todo.id == id) {
                    *existing = todo;
                }
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    pub(in crate::app) fn cut_focused_todo(&mut self) {
        if !self.persistence_available {
            return;
        }

        let Some(Focus::Todo(id)) = self.focus.as_ref() else {
            return;
        };
        let id = *id;
        let focuses = self.flattened_focuses();
        let removed_index = focuses
            .iter()
            .position(|candidate| candidate == &Focus::Todo(id));
        match self.store.delete_todo(id) {
            Ok(todo) => {
                self.todos.retain(|candidate| candidate.id != id);
                self.repository.reconcile_stored_branches(
                    self.todos.iter().map(|todo| todo.branch_ref.as_str()),
                );
                self.todo_register = Some(todo);
                let remaining = self.flattened_focuses();
                self.focus = removed_index
                    .and_then(|index| remaining.get(index).or_else(|| remaining.last()))
                    .cloned();
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    pub(in crate::app) fn paste_registered_todo(&mut self, below: bool) {
        if !self.persistence_available {
            return;
        }

        let Some(registered) = self.todo_register.as_ref() else {
            return;
        };
        let Some(focus) = self.focus.as_ref() else {
            return;
        };
        let (branch_ref, after) = match focus {
            Focus::Branch(branch_ref) => {
                let after = if below {
                    self.todos
                        .iter()
                        .filter(|todo| todo.branch_ref == *branch_ref)
                        .max_by_key(|todo| (todo.sort_order, todo.id))
                        .map(|todo| todo.id)
                } else {
                    None
                };
                (branch_ref.clone(), after)
            }
            Focus::Todo(id) => {
                let Some(target) = self.todos.iter().find(|todo| todo.id == *id) else {
                    return;
                };
                let after = if below {
                    Some(*id)
                } else {
                    self.todos
                        .iter()
                        .filter(|todo| {
                            todo.branch_ref == target.branch_ref
                                && (todo.sort_order, todo.id) < (target.sort_order, target.id)
                        })
                        .max_by_key(|todo| (todo.sort_order, todo.id))
                        .map(|todo| todo.id)
                };
                (target.branch_ref.clone(), after)
            }
        };

        match self.store.insert_todo_with_completion(
            &branch_ref,
            &registered.title,
            registered.completed,
            after,
        ) {
            Ok(todo) => {
                let pasted = todo.id;
                self.integrate_todo(todo);
                self.focus = Some(Focus::Todo(pasted));
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    pub(in crate::app) fn enter_select_mode(&mut self) {
        let (branch_ref, focus_todo, selected_todo_ids) = match self.focus.as_ref() {
            Some(Focus::Todo(id)) => {
                let Some(todo) = self.todos.iter().find(|todo| todo.id == *id) else {
                    return;
                };
                let mut selected = HashSet::new();
                selected.insert(*id);
                (todo.branch_ref.clone(), *id, selected)
            }
            Some(Focus::Branch(branch_ref)) => {
                let Some(first_todo) = self
                    .todos
                    .iter()
                    .find(|todo| todo.branch_ref == *branch_ref)
                else {
                    return;
                };
                (branch_ref.clone(), first_todo.id, HashSet::new())
            }
            None => {
                let Some(first_todo) = self.repository.sections.iter().find_map(|section| {
                    self.todos
                        .iter()
                        .find(|todo| todo.branch_ref == section.full_ref_name)
                }) else {
                    return;
                };
                (first_todo.branch_ref.clone(), first_todo.id, HashSet::new())
            }
        };

        self.focus = Some(Focus::Todo(focus_todo));
        self.mode = Mode::Select(SelectState {
            branch_ref,
            selected_todo_ids,
        });
        self.pending_operator = None;
        self.error = None;
        self.reveal_focus = true;
    }

    pub(in crate::app) fn toggle_focused_selection(&mut self) {
        let Mode::Select(select_state) = &mut self.mode else {
            return;
        };
        let Some(Focus::Todo(id)) = self.focus.as_ref() else {
            return;
        };
        let Some(todo) = self.todos.iter().find(|todo| todo.id == *id) else {
            return;
        };
        if todo.branch_ref != select_state.branch_ref {
            return;
        }
        if !select_state.selected_todo_ids.remove(id) {
            select_state.selected_todo_ids.insert(*id);
        }
    }

    pub(in crate::app) fn exit_select_mode(&mut self) {
        self.mode = Mode::Normal;
        self.error = None;
    }
}

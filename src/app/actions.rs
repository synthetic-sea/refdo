use super::{App, Editor, EditorTarget, Focus, Mode};

impl App {
    pub(in crate::app) fn copy_focused_todo(&mut self) {
        let Some(Focus::Todo(id)) = self.focus.as_ref() else {
            return;
        };
        let Some(todo) = self.todos.iter().find(|todo| todo.id == *id) else {
            return;
        };
        self.clipboard_request = Some(todo.title.clone());
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

    pub(in crate::app) fn open_update_editor(&mut self) {
        if !self.persistence_available {
            return;
        }

        let Some(Focus::Todo(id)) = self.focus.as_ref() else {
            return;
        };
        let Some(todo) = self.todos.iter().find(|todo| todo.id == *id) else {
            return;
        };
        self.mode = Mode::Insert(Editor {
            target: EditorTarget::Update { id: *id },
            text: todo.title.clone(),
            cursor: todo.title.len(),
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
                self.cut_buffer = Some(todo);
                let remaining = self.flattened_focuses();
                self.focus = removed_index
                    .and_then(|index| remaining.get(index).or_else(|| remaining.last()))
                    .cloned();
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    pub(in crate::app) fn paste_cut_todo(&mut self, below: bool) {
        if !self.persistence_available {
            return;
        }

        let Some(cut) = self.cut_buffer.as_ref() else {
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

        match self
            .store
            .insert_todo_with_completion(&branch_ref, &cut.title, cut.completed, after)
        {
            Ok(todo) => {
                let pasted = todo.id;
                self.integrate_todo(todo);
                self.focus = Some(Focus::Todo(pasted));
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
}

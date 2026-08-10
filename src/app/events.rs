use std::{io, time::Duration};

use ratatui::{
    crossterm::event::{
        self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    },
    layout::Position,
};

use super::{App, Mode, text_input, ui};

impl App {
    pub(in crate::app) fn handle_events(&mut self) -> io::Result<()> {
        if event::poll(Duration::from_millis(75))? {
            match event::read()? {
                Event::Key(key) => self.handle_key_event(key),
                Event::Mouse(mouse_event) => self.handle_mouse_event(mouse_event),
                _ => {}
            }
        }
        self.refresh_external();
        self.refresh_system_theme();
        Ok(())
    }

    pub(in crate::app) fn handle_mouse_event(&mut self, mouse_event: MouseEvent) {
        let position = Position::new(mouse_event.column, mouse_event.row);
        self.pointer_position = Some(position);
        if matches!(
            mouse_event.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        ) && ui::todo_viewport_area(self.frame_area).contains(position)
        {
            let area = ui::todo_viewport_area(self.frame_area);
            let rows = ui::build_display_layout(
                &self.repository.sections,
                &self.todos,
                self.mode.editor(),
                area.width,
            );
            let maximum = ui::maximum_viewport_start(&rows, area.height);
            self.viewport_start = match mouse_event.kind {
                MouseEventKind::ScrollUp => self.viewport_start.saturating_sub(1),
                MouseEventKind::ScrollDown => self.viewport_start.saturating_add(1).min(maximum),
                _ => unreachable!("only wheel events enter this branch"),
            }
            .min(maximum);
            self.reveal_focus = false;
        } else if matches!(&self.mode, Mode::Normal)
            && mouse_event.kind == MouseEventKind::Down(MouseButton::Left)
        {
            let focus = {
                let area = ui::todo_viewport_area(self.frame_area);
                if area.contains(position) {
                    let rows = ui::build_display_layout(
                        &self.repository.sections,
                        &self.todos,
                        self.mode.editor(),
                        area.width,
                    );
                    ui::hit_test_display_rows(&rows, area, self.viewport_start, position)
                } else {
                    None
                }
            };
            self.focus = focus;
        }
    }

    fn handle_normal_key(&mut self, code: KeyCode) {
        if code == KeyCode::Char('d') {
            if self.pending_cut {
                self.pending_cut = false;
                self.cut_focused_todo();
            } else {
                self.pending_cut = true;
            }
            return;
        }

        self.pending_cut = false;
        match code {
            KeyCode::Char('j') | KeyCode::Down => self.move_focus(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_focus(-1),
            KeyCode::Char(']') => self.move_section_focus(true),
            KeyCode::Char('[') => self.move_section_focus(false),
            KeyCode::Char('o') => self.open_create_editor(),
            KeyCode::Char(':') => self.open_command_line(),
            KeyCode::Char('i') => self.open_update_editor(),
            KeyCode::Char('x' | ' ') => self.toggle_focused_todo(),
            KeyCode::Char('p') => self.paste_cut_todo(true),
            KeyCode::Char('P') => self.paste_cut_todo(false),
            KeyCode::Esc => self.focus = None,
            KeyCode::Char('q') => self.exit = true,
            _ => {}
        }
    }

    pub(in crate::app) fn handle_key_event(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        match &self.mode {
            Mode::Normal => {
                self.handle_normal_key(key.code);
                return;
            }
            Mode::Command(_) => match key.code {
                KeyCode::Enter => {
                    self.execute_command_line();
                    return;
                }
                KeyCode::Esc => {
                    self.discard_command_line();
                    return;
                }
                _ => {}
            },
            Mode::ConfirmClear(_) => {
                match key.code {
                    KeyCode::Char('y' | 'Y')
                        if key.modifiers == KeyModifiers::NONE
                            || key.modifiers == KeyModifiers::SHIFT =>
                    {
                        self.confirm_clear();
                    }
                    KeyCode::Char('n' | 'N') | KeyCode::Enter | KeyCode::Esc => {
                        self.discard_clear_confirmation();
                    }
                    _ => {}
                }
                return;
            }
            Mode::Insert(_) => match key.code {
                KeyCode::Enter => {
                    self.commit_editor();
                    return;
                }
                KeyCode::Esc => {
                    self.discard_editor();
                    return;
                }
                _ => {}
            },
        }

        match &mut self.mode {
            Mode::Insert(editor) => {
                if text_input::edit_line(&mut editor.text, &mut editor.cursor, &key) {
                    self.reveal_focus = true;
                }
            }
            Mode::Command(command) => {
                text_input::edit_line(&mut command.text, &mut command.cursor, &key);
            }
            Mode::Normal | Mode::ConfirmClear(_) => {}
        }
    }
}

mod repository;
mod storage;
mod theme;

use std::{io, ops::Range, time::Duration};

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::{
        event::{
            self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
            KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
        },
        execute,
    },
    layout::{Constraint, Layout, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use repository::{BranchSection, RepositoryContext};
use storage::{Todo, TodoId, TodoStore};
use theme::{TOKYO_NIGHT_DAY, Theme};

const UNKNOWN_DATA_VERSION: i64 = -1;

#[derive(Clone, Debug)]
enum Mode {
    Normal,
    Insert(Editor),
}

impl Mode {
    const fn label(&self) -> &'static str {
        match self {
            Self::Normal => " NORMAL ",
            Self::Insert(_) => " INSERT ",
        }
    }

    const fn editor(&self) -> Option<&Editor> {
        match self {
            Self::Normal => None,
            Self::Insert(editor) => Some(editor),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Focus {
    Branch(String),
    Todo(TodoId),
}

#[derive(Clone, Debug)]
enum EditorTarget {
    Create {
        branch_ref: String,
        after: Option<TodoId>,
        origin: Focus,
    },
    Update {
        id: TodoId,
    },
}

#[derive(Clone, Debug)]
struct Editor {
    target: EditorTarget,
    text: String,
    cursor: usize,
}

struct App {
    exit: bool,
    repository: RepositoryContext,
    store: TodoStore,
    persistence_available: bool,
    todos: Vec<Todo>,
    focus: Option<Focus>,
    mode: Mode,
    cut_buffer: Option<Todo>,
    pending_cut: bool,
    theme: Theme,
    data_version: i64,
    error: Option<String>,
    pointer_position: Option<Position>,
    frame_area: Rect,
}

impl Default for App {
    fn default() -> Self {
        Self::new(TOKYO_NIGHT_DAY)
    }
}

impl App {
    fn new(theme: Theme) -> Self {
        let mut repository = RepositoryContext::discover(".").unwrap_or_default();
        let (store, mut error, persistence_available) =
            if repository.common_git_dir.as_os_str().is_empty() {
                (
                    TodoStore::open_in_memory().expect("in-memory todo database must open"),
                    Some("not inside a Git repository; todos are unavailable".to_owned()),
                    false,
                )
            } else {
                let database_path = repository.common_git_dir.join("tuido").join("todos.db");
                match TodoStore::open(&database_path) {
                    Ok(store) => (store, None, true),
                    Err(open_error) => (
                        TodoStore::open_in_memory().expect("in-memory todo database must open"),
                        Some(open_error.to_string()),
                        false,
                    ),
                }
            };
        let mut data_version = match store.data_version() {
            Ok(version) => version,
            Err(version_error) => {
                error = Some(version_error.to_string());
                UNKNOWN_DATA_VERSION
            }
        };
        let todos = match store.load_all() {
            Ok(todos) => todos,
            Err(load_error) => {
                error = Some(load_error.to_string());
                data_version = UNKNOWN_DATA_VERSION;
                Vec::new()
            }
        };
        repository.add_stored_branches(todos.iter().map(|todo| todo.branch_ref.as_str()));

        Self {
            exit: false,
            repository,
            store,
            persistence_available,
            todos,
            focus: None,
            mode: Mode::Normal,
            cut_buffer: None,
            pending_cut: false,
            theme,
            data_version,
            error,
            pointer_position: None,
            frame_area: Rect::default(),
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        if let Err(error) = execute!(terminal.backend_mut(), EnableMouseCapture) {
            let _ = execute!(terminal.backend_mut(), DisableMouseCapture);
            return Err(error);
        }

        let result = (|| {
            while !self.exit {
                terminal.draw(|frame| self.draw(frame))?;
                self.handle_events()?;
            }
            Ok(())
        })();
        let disable_result = execute!(terminal.backend_mut(), DisableMouseCapture);
        result.and(disable_result)
    }

    fn draw(&mut self, frame: &mut Frame) {
        self.frame_area = frame.area();
        let hovered = self
            .pointer_position
            .and_then(|position| self.focus_at(position));
        let [status_area, content_area, footer_area] = app_areas(self.frame_area);

        render_status_bar(frame, status_area, &self.repository.head_label, &self.theme);
        let content_block = Block::bordered()
            .style(Style::default().bg(self.theme.background))
            .border_style(Style::default().fg(self.theme.mode_background));
        let todo_area = todo_viewport_area(self.frame_area);
        frame.render_widget(content_block, content_area);
        render_branch_sections(
            frame,
            todo_area,
            &self.repository.sections,
            &self.todos,
            self.focus.as_ref(),
            hovered.as_ref(),
            self.mode.editor(),
            &self.theme,
        );
        render_footer(
            frame,
            footer_area,
            &self.mode,
            self.error.as_deref(),
            &self.theme,
        );
    }

    fn handle_events(&mut self) -> io::Result<()> {
        if event::poll(Duration::from_millis(75))? {
            match event::read()? {
                Event::Key(key) => self.handle_key_event(key),
                Event::Mouse(mouse_event) => self.handle_mouse_event(mouse_event),
                _ => {}
            }
        }
        self.refresh_external();
        Ok(())
    }

    fn handle_mouse_event(&mut self, mouse_event: MouseEvent) {
        let position = Position::new(mouse_event.column, mouse_event.row);
        self.pointer_position = Some(position);
        if matches!(&self.mode, Mode::Normal)
            && mouse_event.kind == MouseEventKind::Down(MouseButton::Left)
        {
            self.focus = self.focus_at(position);
        }
    }

    fn focus_at(&self, position: Position) -> Option<Focus> {
        let area = todo_viewport_area(self.frame_area);
        if !area.contains(position) {
            return None;
        }
        let editor = self.mode.editor();
        let rows = layout_display_rows(
            display_rows(&self.repository.sections, &self.todos, editor),
            area.width,
        );
        let first = viewport_start(&rows, area.height, self.focus.as_ref(), editor);
        let target_y = usize::from(position.y - area.y);
        let mut rendered_y = 0;
        for layout in rows.iter().skip(first) {
            if rendered_y >= usize::from(area.height) {
                break;
            }
            let rendered_height = layout
                .visual_height()
                .min(usize::from(area.height) - rendered_y);
            if target_y < rendered_y + rendered_height {
                return match &layout.row {
                    DisplayRow::Header(section) => {
                        Some(Focus::Branch(section.full_ref_name.clone()))
                    }
                    DisplayRow::Todo(todo) => Some(Focus::Todo(todo.id)),
                    DisplayRow::Editor { .. } | DisplayRow::Empty => None,
                };
            }
            rendered_y += rendered_height;
        }
        None
    }

    fn refresh_external(&mut self) {
        let Ok(version) = self.store.data_version() else {
            return;
        };
        if version == self.data_version {
            return;
        }
        match self.store.load_all() {
            Ok(todos) => {
                self.todos = todos;
                self.repository
                    .add_stored_branches(self.todos.iter().map(|todo| todo.branch_ref.as_str()));
                self.data_version = version;
                self.repair_focus();
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn reload(&mut self) -> bool {
        let version = match self.store.data_version() {
            Ok(version) => version,
            Err(error) => {
                self.data_version = UNKNOWN_DATA_VERSION;
                self.error = Some(error.to_string());
                return false;
            }
        };
        match self.store.load_all() {
            Ok(todos) => {
                self.todos = todos;
                self.repository
                    .add_stored_branches(self.todos.iter().map(|todo| todo.branch_ref.as_str()));
                self.data_version = version;
                true
            }
            Err(error) => {
                self.data_version = UNKNOWN_DATA_VERSION;
                self.error = Some(error.to_string());
                false
            }
        }
    }

    fn integrate_todo(&mut self, todo: Todo) {
        for existing in &mut self.todos {
            if existing.branch_ref == todo.branch_ref && existing.sort_order >= todo.sort_order {
                existing.sort_order += 1;
            }
        }
        self.repository
            .add_stored_branches(std::iter::once(todo.branch_ref.as_str()));
        self.todos.push(todo);
        self.todos.sort_by(|left, right| {
            left.branch_ref
                .cmp(&right.branch_ref)
                .then_with(|| left.sort_order.cmp(&right.sort_order))
                .then_with(|| left.id.cmp(&right.id))
        });
    }

    fn repair_focus(&mut self) {
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

    fn flattened_focuses(&self) -> Vec<Focus> {
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

    fn move_focus(&mut self, delta: isize) {
        let rows = self.flattened_focuses();
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

    fn move_section_focus(&mut self, forward: bool) {
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

    fn open_create_editor(&mut self) {
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
        self.error = None;
    }

    fn open_update_editor(&mut self) {
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
        self.error = None;
    }

    fn discard_editor(&mut self) {
        if let Mode::Insert(Editor {
            target: EditorTarget::Create { origin, .. },
            ..
        }) = &self.mode
        {
            self.focus = Some(origin.clone());
        }
        self.mode = Mode::Normal;
        self.error = None;
    }

    fn commit_editor(&mut self) {
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
                }
                Err(error) => self.error = Some(error.to_string()),
            },
        }
    }

    fn toggle_focused_todo(&mut self) {
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

    fn cut_focused_todo(&mut self) {
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

    fn paste_cut_todo(&mut self, below: bool) {
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
            KeyCode::Char('i') => self.open_update_editor(),
            KeyCode::Char('x' | ' ') => self.toggle_focused_todo(),
            KeyCode::Char('p') => self.paste_cut_todo(true),
            KeyCode::Char('P') => self.paste_cut_todo(false),
            KeyCode::Esc => self.focus = None,
            KeyCode::Char('q') => self.exit = true,
            _ => {}
        }
    }

    fn handle_key_event(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        if matches!(&self.mode, Mode::Normal) {
            self.handle_normal_key(key.code);
            return;
        }
        match key.code {
            KeyCode::Enter => {
                self.commit_editor();
                return;
            }
            KeyCode::Esc => {
                self.discard_editor();
                return;
            }
            _ => {}
        }
        let Mode::Insert(editor) = &mut self.mode else {
            return;
        };
        match key.code {
            KeyCode::Char(character) => {
                editor.text.insert(editor.cursor, character);
                let insertion_end = editor.cursor + character.len_utf8();
                editor.cursor = boundary_at_or_after(&editor.text, insertion_end);
            }
            KeyCode::Backspace if editor.cursor > 0 => {
                let previous = previous_boundary(&editor.text, editor.cursor);
                editor.text.drain(previous..editor.cursor);
                editor.cursor = previous;
            }
            KeyCode::Delete if editor.cursor < editor.text.len() => {
                let next = next_boundary(&editor.text, editor.cursor);
                editor.text.drain(editor.cursor..next);
            }
            KeyCode::Left
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
            {
                editor.cursor = previous_word_boundary(&editor.text, editor.cursor);
            }
            KeyCode::Right
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
            {
                editor.cursor = next_word_boundary(&editor.text, editor.cursor);
            }
            KeyCode::Left => {
                editor.cursor = previous_boundary(&editor.text, editor.cursor);
            }
            KeyCode::Right => {
                editor.cursor = next_boundary(&editor.text, editor.cursor);
            }
            KeyCode::Home => editor.cursor = 0,
            KeyCode::End => editor.cursor = editor.text.len(),
            _ => {}
        }
    }
}

fn previous_boundary(text: &str, cursor: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(index, _)| index)
        .take_while(|index| *index < cursor)
        .last()
        .unwrap_or(0)
}

fn next_boundary(text: &str, cursor: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(index, _)| index)
        .find(|index| *index > cursor)
        .unwrap_or(text.len())
}

fn previous_word_boundary(text: &str, cursor: usize) -> usize {
    text[..cursor]
        .split_word_bound_indices()
        .filter(|(_, segment)| !segment.chars().all(char::is_whitespace))
        .map(|(index, _)| index)
        .last()
        .unwrap_or(0)
}

fn next_word_boundary(text: &str, cursor: usize) -> usize {
    text[cursor..]
        .split_word_bound_indices()
        .skip(1)
        .find(|(_, segment)| !segment.chars().all(char::is_whitespace))
        .map(|(index, _)| cursor + index)
        .unwrap_or(text.len())
}

fn boundary_at_or_after(text: &str, cursor: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(index, _)| index)
        .find(|index| *index >= cursor)
        .unwrap_or(text.len())
}

fn app_areas(area: Rect) -> [Rect; 3] {
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area)
}

fn todo_viewport_area(frame_area: Rect) -> Rect {
    let [_, content_area, _] = app_areas(frame_area);
    Block::bordered().inner(content_area)
}

fn render_status_bar(frame: &mut Frame, area: Rect, branch: &str, theme: &Theme) {
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.status_bar_background)),
        area,
    );
    let [brand_area, context_area] =
        Layout::horizontal([Constraint::Length(7), Constraint::Min(0)]).areas(area);
    frame.render_widget(
        Paragraph::new(" tuido")
            .style(
                Style::default()
                    .fg(theme.foreground)
                    .bg(theme.status_bar_background)
                    .add_modifier(Modifier::BOLD),
            )
            .left_aligned(),
        brand_area,
    );
    frame.render_widget(
        Paragraph::new(format!("git:{branch} "))
            .style(
                Style::default()
                    .fg(theme.foreground_muted)
                    .bg(theme.status_bar_background),
            )
            .right_aligned(),
        context_area,
    );
}

fn render_footer(frame: &mut Frame, area: Rect, mode: &Mode, error: Option<&str>, theme: &Theme) {
    let mode = Span::styled(
        mode.label(),
        Style::default()
            .fg(theme.mode_foreground)
            .bg(theme.mode_background)
            .add_modifier(Modifier::BOLD),
    );
    let error = error
        .map(|message| Span::styled(format!(" {message}"), Style::default().fg(theme.foreground)))
        .unwrap_or_default();
    frame.render_widget(
        Paragraph::new(Line::from(vec![mode, error])).style(
            Style::default()
                .fg(theme.foreground)
                .bg(theme.status_bar_background),
        ),
        area,
    );
}

enum DisplayRow<'a> {
    Header(&'a BranchSection),
    Todo(&'a Todo),
    Editor { editor: &'a Editor, completed: bool },
    Empty,
}

fn display_rows<'a>(
    sections: &'a [BranchSection],
    todos: &'a [Todo],
    editor: Option<&'a Editor>,
) -> Vec<DisplayRow<'a>> {
    let mut rows = Vec::new();
    for section in sections {
        rows.push(DisplayRow::Header(section));
        let branch_todos = todos
            .iter()
            .filter(|todo| todo.branch_ref == section.full_ref_name)
            .collect::<Vec<_>>();
        let create_editor = editor.filter(|editor| {
            matches!(
                &editor.target,
                EditorTarget::Create { branch_ref, .. }
                    if branch_ref == &section.full_ref_name
            )
        });
        if branch_todos.is_empty() && create_editor.is_none() {
            rows.push(DisplayRow::Empty);
            continue;
        }
        if create_editor.is_some_and(|editor| {
            matches!(&editor.target, EditorTarget::Create { after: None, .. })
        }) {
            rows.push(DisplayRow::Editor {
                editor: create_editor.expect("checked above"),
                completed: false,
            });
        }
        for todo in branch_todos {
            if editor.is_some_and(
                |editor| matches!(&editor.target, EditorTarget::Update { id } if *id == todo.id),
            ) {
                rows.push(DisplayRow::Editor {
                    editor: editor.expect("checked above"),
                    completed: todo.completed,
                });
            } else {
                rows.push(DisplayRow::Todo(todo));
            }
            if create_editor.is_some_and(|editor| {
                matches!(&editor.target, EditorTarget::Create { after, .. } if *after == Some(todo.id))
            }) {
                rows.push(DisplayRow::Editor {
                    editor: create_editor.expect("checked above"),
                    completed: false,
                });
            }
        }
    }
    rows
}

const TODO_PREFIX_WIDTH: u16 = 8;

struct DisplayRowLayout<'a> {
    row: DisplayRow<'a>,
    title_ranges: Vec<Range<usize>>,
    editor_cursor: Option<(usize, usize)>,
}

impl DisplayRowLayout<'_> {
    fn visual_height(&self) -> usize {
        self.title_ranges.len().max(1)
    }
}

fn wrap_title(title: &str, width: u16) -> Vec<Range<usize>> {
    let width = usize::from(width);
    let mut wrapped = Vec::new();
    let mut explicit_start = 0;

    for explicit_line in title.split('\n') {
        let explicit_end = explicit_start + explicit_line.len();
        if width == 0 || explicit_line.is_empty() {
            wrapped.push(explicit_start..explicit_start);
        } else {
            let mut remaining_start = explicit_start;
            while remaining_start < explicit_end {
                let remaining = &title[remaining_start..explicit_end];
                let mut used_width = 0usize;
                let mut fitted_end = 0;
                for (index, grapheme) in remaining.grapheme_indices(true) {
                    let grapheme_width = UnicodeWidthStr::width(grapheme);
                    let next_width = used_width.saturating_add(grapheme_width);
                    if next_width > width {
                        if fitted_end == 0 {
                            fitted_end = index + grapheme.len();
                        }
                        break;
                    }
                    fitted_end = index + grapheme.len();
                    used_width = next_width;
                }

                if fitted_end == remaining.len() {
                    wrapped.push(remaining_start..explicit_end);
                    break;
                }

                let word_break = remaining
                    .split_word_bound_indices()
                    .map(|(index, _)| index)
                    .take_while(|index| *index <= fitted_end)
                    .filter(|index| *index > 0)
                    .last();
                let line_end = word_break.unwrap_or(fitted_end);
                let display_end = remaining_start + remaining[..line_end].trim_end().len();
                wrapped.push(remaining_start..display_end);

                let next_start = remaining_start + line_end;
                let trimmed = title[next_start..explicit_end].trim_start();
                remaining_start = explicit_end - trimmed.len();
            }
        }

        explicit_start = explicit_end.saturating_add(1).min(title.len());
    }

    if wrapped.is_empty() {
        wrapped.push(0..0);
    }
    wrapped
}

fn wrapped_cursor(text: &str, lines: &[Range<usize>], cursor: usize) -> (usize, usize) {
    for (row, line) in lines.iter().enumerate() {
        let next_start = lines
            .get(row + 1)
            .map_or(text.len().saturating_add(1), |next| next.start);
        if cursor < next_start || row + 1 == lines.len() {
            let cursor = cursor.clamp(line.start, line.end);
            return (row, UnicodeWidthStr::width(&text[line.start..cursor]));
        }
    }
    (0, 0)
}

fn layout_display_rows<'a>(rows: Vec<DisplayRow<'a>>, width: u16) -> Vec<DisplayRowLayout<'a>> {
    let title_width = width.saturating_sub(TODO_PREFIX_WIDTH);
    rows.into_iter()
        .map(|row| {
            let title_ranges = match &row {
                DisplayRow::Todo(todo) => wrap_title(&todo.title, title_width),
                DisplayRow::Editor { editor, .. } => wrap_title(&editor.text, title_width),
                DisplayRow::Header(_) | DisplayRow::Empty => Vec::new(),
            };
            let editor_cursor = match &row {
                DisplayRow::Editor { editor, .. } => {
                    Some(wrapped_cursor(&editor.text, &title_ranges, editor.cursor))
                }
                DisplayRow::Header(_) | DisplayRow::Todo(_) | DisplayRow::Empty => None,
            };
            DisplayRowLayout {
                row,
                title_ranges,
                editor_cursor,
            }
        })
        .collect()
}

fn row_has_focus(
    layout: &DisplayRowLayout<'_>,
    focus: Option<&Focus>,
    editor: Option<&Editor>,
) -> bool {
    match &layout.row {
        DisplayRow::Header(section) => {
            editor.is_none()
                && matches!(focus, Some(Focus::Branch(branch_ref)) if branch_ref == &section.full_ref_name)
        }
        DisplayRow::Todo(todo) => {
            editor.is_none() && matches!(focus, Some(Focus::Todo(id)) if *id == todo.id)
        }
        DisplayRow::Editor { .. } => true,
        DisplayRow::Empty => false,
    }
}
fn viewport_start(
    rows: &[DisplayRowLayout<'_>],
    height: u16,
    focus: Option<&Focus>,
    editor: Option<&Editor>,
) -> usize {
    let Some(focused) = rows
        .iter()
        .position(|layout| row_has_focus(layout, focus, editor))
    else {
        return 0;
    };

    let height = usize::from(height);
    let mut start = focused;
    let mut occupied = rows[focused].visual_height();
    while start > 0 {
        let preceding_height = rows[start - 1].visual_height();
        if occupied.saturating_add(preceding_height) > height {
            break;
        }
        start -= 1;
        occupied = occupied.saturating_add(preceding_height);
    }
    start
}

fn render_branch_sections(
    frame: &mut Frame,
    area: Rect,
    sections: &[BranchSection],
    todos: &[Todo],
    focus: Option<&Focus>,
    hovered: Option<&Focus>,
    editor: Option<&Editor>,
    theme: &Theme,
) {
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.background)),
        area,
    );
    if sections.is_empty() {
        frame.render_widget(
            Paragraph::new(" No registered worktree branches").style(
                Style::default()
                    .fg(theme.foreground_muted)
                    .bg(theme.background),
            ),
            area,
        );
        return;
    }
    if area.height == 0 {
        return;
    }

    let rows = layout_display_rows(display_rows(sections, todos, editor), area.width);
    let first = viewport_start(&rows, area.height, focus, editor);
    let mut rendered_y = 0usize;

    for layout in rows.iter().skip(first) {
        if rendered_y >= usize::from(area.height) {
            break;
        }
        let rendered_height = layout
            .visual_height()
            .min(usize::from(area.height) - rendered_y);
        let row_area = Rect::new(
            area.x,
            area.y + rendered_y as u16,
            area.width,
            rendered_height as u16,
        );

        match &layout.row {
            DisplayRow::Header(section) => {
                let selected = editor.is_none()
                    && matches!(focus, Some(Focus::Branch(branch_ref)) if branch_ref == &section.full_ref_name);
                let hovered = matches!(
                    hovered,
                    Some(Focus::Branch(branch_ref)) if branch_ref == &section.full_ref_name
                );
                render_branch_header(frame, row_area, section, selected, hovered, theme);
            }
            DisplayRow::Todo(todo) => {
                let selected =
                    editor.is_none() && matches!(focus, Some(Focus::Todo(id)) if *id == todo.id);
                let hovered = matches!(hovered, Some(Focus::Todo(id)) if *id == todo.id);
                let background = if selected {
                    theme.selection_background
                } else if hovered {
                    theme.hover_background
                } else {
                    theme.background
                };
                let marker = if todo.completed { "[x]" } else { "[ ]" };
                let foreground = if todo.completed {
                    theme.foreground_muted
                } else {
                    theme.foreground
                };
                let style = Style::default().fg(foreground).bg(background);
                frame.render_widget(Block::default().style(style), row_area);

                let marker_area = Rect::new(
                    row_area.x,
                    row_area.y,
                    row_area.width.min(TODO_PREFIX_WIDTH),
                    1,
                );
                frame.render_widget(
                    Paragraph::new(format!("    {marker} ")).style(style),
                    marker_area,
                );

                if row_area.width > TODO_PREFIX_WIDTH {
                    let title_area = Rect::new(
                        row_area.x.saturating_add(TODO_PREFIX_WIDTH),
                        row_area.y,
                        row_area.width - TODO_PREFIX_WIDTH,
                        row_area.height,
                    );
                    let title_lines = layout
                        .title_ranges
                        .iter()
                        .map(|range| Line::from(&todo.title[range.clone()]))
                        .collect::<Vec<_>>();
                    frame.render_widget(Paragraph::new(title_lines).style(style), title_area);
                }
            }
            DisplayRow::Editor { editor, completed } => render_editor(
                frame,
                row_area,
                editor,
                *completed,
                &layout.title_ranges,
                layout
                    .editor_cursor
                    .expect("editor rows always have a cursor"),
                theme,
            ),
            DisplayRow::Empty => {
                frame.render_widget(
                    Paragraph::new("    No todos").style(
                        Style::default()
                            .fg(theme.foreground_muted)
                            .bg(theme.background),
                    ),
                    row_area,
                );
            }
        }
        rendered_y += rendered_height;
    }
}

fn render_branch_header(
    frame: &mut Frame,
    area: Rect,
    section: &BranchSection,
    selected: bool,
    hovered: bool,
    theme: &Theme,
) {
    let background = if selected {
        theme.selection_background
    } else if hovered {
        theme.hover_background
    } else {
        theme.background
    };
    frame.render_widget(
        Block::default().style(Style::default().bg(background)),
        area,
    );
    let tag = if section.is_stored_only {
        " BRANCH "
    } else {
        match (section.is_current, section.is_locked) {
            (true, true) => " CURRENT · LOCKED ",
            (true, false) => " CURRENT ",
            (false, true) => " WORKTREE · LOCKED ",
            (false, false) => " WORKTREE ",
        }
    };
    let tag_width = UnicodeWidthStr::width(tag);
    let branch_width = 2usize.saturating_add(UnicodeWidthStr::width(section.display_name.as_str()));
    let tag_width = if branch_width.saturating_add(tag_width) <= usize::from(area.width) {
        tag_width as u16
    } else {
        0
    };
    let [branch_area, tag_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(tag_width)]).areas(area);
    let style = Style::default()
        .fg(theme.foreground)
        .bg(background)
        .add_modifier(Modifier::BOLD);
    frame.render_widget(
        Paragraph::new(format!("▾ {}", section.display_name)).style(style),
        branch_area,
    );
    if tag_width > 0 {
        frame.render_widget(
            Paragraph::new(tag)
                .style(Style::default().fg(theme.foreground_muted).bg(background))
                .right_aligned(),
            tag_area,
        );
    }
}

fn render_editor(
    frame: &mut Frame,
    area: Rect,
    editor: &Editor,
    completed: bool,
    title_ranges: &[Range<usize>],
    cursor: (usize, usize),
    theme: &Theme,
) {
    let marker = if completed { "[x]" } else { "[ ]" };
    let background = theme.selection_background;
    let foreground = if completed {
        theme.foreground_muted
    } else {
        theme.foreground
    };
    let style = Style::default().fg(foreground).bg(background);
    frame.render_widget(Block::default().style(style), area);

    let first_line = cursor
        .0
        .saturating_add(1)
        .saturating_sub(usize::from(area.height));
    let marker_area = Rect::new(
        area.x,
        area.y,
        area.width.min(TODO_PREFIX_WIDTH),
        area.height.min(1),
    );
    frame.render_widget(
        Paragraph::new(format!("    {marker} ")).style(style),
        marker_area,
    );

    if area.width <= TODO_PREFIX_WIDTH {
        return;
    }
    let title_area = Rect::new(
        area.x.saturating_add(TODO_PREFIX_WIDTH),
        area.y,
        area.width - TODO_PREFIX_WIDTH,
        area.height,
    );
    let title_lines = title_ranges
        .iter()
        .skip(first_line)
        .take(usize::from(area.height))
        .map(|range| Line::from(&editor.text[range.clone()]))
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(title_lines).style(style), title_area);

    let cursor_y = area
        .y
        .saturating_add(cursor.0.saturating_sub(first_line) as u16)
        .min(area.bottom().saturating_sub(1));
    let cursor_x = title_area
        .x
        .saturating_add(cursor.1 as u16)
        .min(title_area.right().saturating_sub(1));
    frame.set_cursor_position(Position::new(cursor_x, cursor_y));
}

#[cfg(test)]
mod tests {
    use ratatui::{
        Terminal,
        backend::{Backend, TestBackend},
        crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
        style::Color,
    };

    use super::*;

    fn test_theme() -> Theme {
        Theme {
            background: Color::Red,
            foreground: Color::Green,
            foreground_muted: Color::Blue,
            hover_background: Color::LightCyan,
            selection_background: Color::LightYellow,
            status_bar_background: Color::Yellow,
            mode_background: Color::Magenta,
            mode_foreground: Color::Cyan,
        }
    }

    fn section(name: &str) -> BranchSection {
        BranchSection {
            full_ref_name: format!("refs/heads/{name}"),
            display_name: name.to_owned(),
            worktree_path: format!("/worktrees/{name}").into(),
            is_current: name == "main",
            is_locked: false,
            is_stored_only: false,
        }
    }

    fn app_with_sections(sections: Vec<BranchSection>) -> App {
        let store = TodoStore::open_in_memory().unwrap();
        let focus = sections
            .first()
            .map(|section| Focus::Branch(section.full_ref_name.clone()));
        App {
            exit: false,
            repository: RepositoryContext {
                head_label: "main".to_owned(),
                common_git_dir: Default::default(),
                sections,
            },
            store,
            persistence_available: true,
            todos: Vec::new(),
            focus,
            mode: Mode::Normal,
            cut_buffer: None,
            pending_cut: false,
            theme: test_theme(),
            data_version: 0,
            error: None,
            pointer_position: None,
            frame_area: Rect::default(),
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn modified_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn type_text(app: &mut App, text: &str) {
        for character in text.chars() {
            app.handle_key_event(key(KeyCode::Char(character)));
        }
    }

    fn editor(app: &App) -> &Editor {
        let Mode::Insert(editor) = &app.mode else {
            panic!("expected insert mode");
        };
        editor
    }

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn row_text(terminal: &Terminal<TestBackend>, row: u16) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.width)
            .map(|column| buffer[(column, row)].symbol())
            .collect()
    }

    fn branch_titles(app: &App, branch_ref: &str) -> Vec<(String, bool)> {
        app.todos
            .iter()
            .filter(|todo| todo.branch_ref == branch_ref)
            .map(|todo| (todo.title.clone(), todo.completed))
            .collect()
    }

    #[test]
    fn escape_clears_normal_mode_row_selection() {
        let mut app = app_with_sections(vec![section("main")]);
        let todo = app
            .store
            .insert_todo("refs/heads/main", "selected", None)
            .unwrap();
        app.reload();
        app.focus = Some(Focus::Todo(todo.id));

        app.handle_key_event(key(KeyCode::Char('d')));
        assert!(app.pending_cut);
        app.handle_key_event(key(KeyCode::Esc));

        assert_eq!(app.focus, None);
        assert!(!app.pending_cut);
        assert_eq!(app.store.load_all().unwrap(), app.todos);

        app.focus = Some(Focus::Branch("refs/heads/main".to_owned()));
        app.handle_key_event(key(KeyCode::Esc));
        assert_eq!(app.focus, None);
    }

    #[test]
    fn dd_requires_consecutive_keys_and_focuses_the_next_row_after_cut() {
        let mut app = app_with_sections(vec![section("main")]);
        let first = app
            .store
            .insert_todo("refs/heads/main", "first", None)
            .unwrap();
        let second = app
            .store
            .insert_todo("refs/heads/main", "second", Some(first.id))
            .unwrap();
        app.reload();
        app.focus = Some(Focus::Todo(first.id));

        app.handle_key_event(key(KeyCode::Char('d')));
        assert!(app.pending_cut);
        assert_eq!(app.store.load_all().unwrap().len(), 2);

        app.handle_key_event(key(KeyCode::Char('j')));
        assert!(!app.pending_cut);
        assert_eq!(app.focus, Some(Focus::Todo(second.id)));
        assert_eq!(app.store.load_all().unwrap().len(), 2);

        app.handle_key_event(key(KeyCode::Char('d')));
        app.handle_key_event(key(KeyCode::Char('k')));
        assert_eq!(app.focus, Some(Focus::Todo(first.id)));
        assert_eq!(app.store.load_all().unwrap().len(), 2);

        app.handle_key_event(key(KeyCode::Char('d')));
        app.handle_key_event(key(KeyCode::Char('d')));

        assert!(!app.pending_cut);
        assert_eq!(app.focus, Some(Focus::Todo(second.id)));
        assert_eq!(
            branch_titles(&app, "refs/heads/main"),
            vec![("second".to_owned(), false)]
        );
        assert_eq!(app.store.load_all().unwrap(), app.todos);
        assert_eq!(
            app.cut_buffer.as_ref().map(|todo| todo.title.as_str()),
            Some("first")
        );
    }

    #[test]
    fn paste_register_persists_and_preserves_completion() {
        let mut app = app_with_sections(vec![section("main")]);
        let completed = app
            .store
            .insert_todo("refs/heads/main", "repeat me", None)
            .unwrap();
        app.store.toggle_todo(completed.id).unwrap();
        app.reload();
        app.focus = Some(Focus::Todo(completed.id));

        app.handle_key_event(key(KeyCode::Char('d')));
        app.handle_key_event(key(KeyCode::Char('d')));
        let cut_id = app.cut_buffer.as_ref().unwrap().id;
        assert!(app.store.load_all().unwrap().is_empty());

        app.handle_key_event(key(KeyCode::Char('p')));
        let first_paste = match app.focus {
            Some(Focus::Todo(id)) => id,
            _ => panic!("pasted todo must receive focus"),
        };
        app.handle_key_event(key(KeyCode::Char('p')));
        let second_paste = match app.focus {
            Some(Focus::Todo(id)) => id,
            _ => panic!("pasted todo must receive focus"),
        };

        assert_ne!(first_paste, second_paste);
        assert_eq!(app.cut_buffer.as_ref().unwrap().id, cut_id);
        assert_eq!(
            branch_titles(&app, "refs/heads/main"),
            vec![
                ("repeat me".to_owned(), true),
                ("repeat me".to_owned(), true)
            ]
        );
        assert_eq!(app.store.load_all().unwrap(), app.todos);
    }

    #[test]
    fn todo_paste_positions_above_and_below_in_another_section() {
        let mut app = app_with_sections(vec![section("main"), section("topic")]);
        let first = app
            .store
            .insert_todo("refs/heads/main", "first", None)
            .unwrap();
        let second = app
            .store
            .insert_todo("refs/heads/main", "second", Some(first.id))
            .unwrap();
        let source = app
            .store
            .insert_todo("refs/heads/topic", "source", None)
            .unwrap();
        app.reload();
        app.focus = Some(Focus::Todo(source.id));
        app.handle_key_event(key(KeyCode::Char('d')));
        app.handle_key_event(key(KeyCode::Char('d')));

        app.focus = Some(Focus::Todo(second.id));
        app.handle_key_event(key(KeyCode::Char('P')));
        let above = app.focus.clone();
        assert_eq!(
            branch_titles(&app, "refs/heads/main"),
            vec![
                ("first".to_owned(), false),
                ("source".to_owned(), false),
                ("second".to_owned(), false)
            ]
        );

        app.handle_key_event(key(KeyCode::Char('p')));
        assert_ne!(app.focus, above);
        assert_eq!(
            branch_titles(&app, "refs/heads/main"),
            vec![
                ("first".to_owned(), false),
                ("source".to_owned(), false),
                ("source".to_owned(), false),
                ("second".to_owned(), false)
            ]
        );
        assert!(branch_titles(&app, "refs/heads/topic").is_empty());
        assert_eq!(app.store.load_all().unwrap(), app.todos);
    }

    #[test]
    fn header_paste_uses_section_top_and_bottom() {
        let mut app = app_with_sections(vec![section("main"), section("topic")]);
        let first = app
            .store
            .insert_todo("refs/heads/main", "first", None)
            .unwrap();
        app.store
            .insert_todo("refs/heads/main", "second", Some(first.id))
            .unwrap();
        let source = app
            .store
            .insert_todo("refs/heads/topic", "source", None)
            .unwrap();
        app.reload();
        app.focus = Some(Focus::Todo(source.id));
        app.handle_key_event(key(KeyCode::Char('d')));
        app.handle_key_event(key(KeyCode::Char('d')));

        app.focus = Some(Focus::Branch("refs/heads/main".to_owned()));
        app.handle_key_event(key(KeyCode::Char('P')));
        assert_eq!(
            branch_titles(&app, "refs/heads/main"),
            vec![
                ("source".to_owned(), false),
                ("first".to_owned(), false),
                ("second".to_owned(), false)
            ]
        );

        app.focus = Some(Focus::Branch("refs/heads/main".to_owned()));
        app.handle_key_event(key(KeyCode::Char('p')));
        assert_eq!(
            branch_titles(&app, "refs/heads/main"),
            vec![
                ("source".to_owned(), false),
                ("first".to_owned(), false),
                ("second".to_owned(), false),
                ("source".to_owned(), false)
            ]
        );
        assert!(matches!(app.focus, Some(Focus::Todo(_))));
        assert_eq!(app.store.load_all().unwrap(), app.todos);
    }

    #[test]
    fn cut_paste_no_ops_and_failures_preserve_state() {
        let mut app = app_with_sections(vec![section("main")]);
        app.handle_key_event(key(KeyCode::Char('p')));
        app.handle_key_event(key(KeyCode::Char('P')));
        app.handle_key_event(key(KeyCode::Char('d')));
        app.handle_key_event(key(KeyCode::Char('d')));
        assert!(app.todos.is_empty());
        assert!(app.cut_buffer.is_none());

        let retained = app
            .store
            .insert_todo("refs/heads/main", "retained", None)
            .unwrap();
        app.reload();
        app.cut_buffer = Some(retained.clone());
        app.focus = Some(Focus::Todo(i64::MAX));
        app.handle_key_event(key(KeyCode::Char('d')));
        app.handle_key_event(key(KeyCode::Char('d')));

        assert_eq!(app.cut_buffer, Some(retained.clone()));
        assert_eq!(app.store.load_all().unwrap(), app.todos);
        assert!(app.error.is_some());

        let mut invalid_register = retained;
        invalid_register.title = "   ".to_owned();
        app.cut_buffer = Some(invalid_register.clone());
        app.focus = Some(Focus::Branch("refs/heads/main".to_owned()));
        let todos_before_failed_paste = app.todos.clone();
        app.handle_key_event(key(KeyCode::Char('p')));

        assert_eq!(app.cut_buffer, Some(invalid_register));
        assert_eq!(app.todos, todos_before_failed_paste);
        assert_eq!(app.store.load_all().unwrap(), app.todos);
        assert!(app.error.is_some());
    }

    #[test]
    fn insert_mode_treats_cut_and_paste_keys_as_text() {
        let mut app = app_with_sections(vec![section("main")]);
        app.handle_key_event(key(KeyCode::Char('o')));

        app.handle_key_event(key(KeyCode::Char('d')));
        app.handle_key_event(key(KeyCode::Char('d')));
        app.handle_key_event(key(KeyCode::Char('p')));
        app.handle_key_event(key(KeyCode::Char('P')));

        assert_eq!(editor(&app).text, "ddpP");
        assert!(!app.pending_cut);
        assert!(app.cut_buffer.is_none());
        assert!(app.store.load_all().unwrap().is_empty());
    }

    #[test]
    fn header_creation_inserts_first_and_chains_entries() {
        let mut app = app_with_sections(vec![section("main")]);
        let existing_first = app
            .store
            .insert_todo("refs/heads/main", "existing first", None)
            .unwrap();
        app.store
            .insert_todo(
                "refs/heads/main",
                "existing second",
                Some(existing_first.id),
            )
            .unwrap();
        app.reload();
        app.focus = Some(Focus::Branch("refs/heads/main".to_owned()));

        app.handle_key_event(key(KeyCode::Char('o')));
        type_text(&mut app, "first");
        assert_eq!(
            app.store
                .load_all()
                .unwrap()
                .iter()
                .map(|todo| todo.title.as_str())
                .collect::<Vec<_>>(),
            ["existing first", "existing second"]
        );
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(row_text(&terminal, 3).contains("[ ] first"));
        assert!(row_text(&terminal, 4).contains("[ ] existing first"));

        app.handle_key_event(key(KeyCode::Enter));
        assert_eq!(
            app.todos
                .iter()
                .map(|todo| todo.title.as_str())
                .collect::<Vec<_>>(),
            ["first", "existing first", "existing second"]
        );
        assert!(matches!(&app.mode, Mode::Insert(_)));
        type_text(&mut app, "second");
        app.handle_key_event(key(KeyCode::Enter));
        assert_eq!(
            app.todos
                .iter()
                .map(|todo| todo.title.as_str())
                .collect::<Vec<_>>(),
            ["first", "second", "existing first", "existing second"]
        );
    }

    #[test]
    fn opening_on_todo_inserts_directly_below_it() {
        let mut app = app_with_sections(vec![section("main")]);
        let first = app
            .store
            .insert_todo("refs/heads/main", "first", None)
            .unwrap();
        app.store
            .insert_todo("refs/heads/main", "third", Some(first.id))
            .unwrap();
        app.reload();
        app.focus = Some(Focus::Todo(first.id));
        app.handle_key_event(key(KeyCode::Char('o')));
        type_text(&mut app, "second");
        app.handle_key_event(key(KeyCode::Enter));
        assert_eq!(
            app.todos
                .iter()
                .map(|todo| todo.title.as_str())
                .collect::<Vec<_>>(),
            ["first", "second", "third"]
        );
    }

    #[test]
    fn blank_enter_and_escape_discard_without_writing() {
        let mut app = app_with_sections(vec![section("main")]);
        app.handle_key_event(key(KeyCode::Char('o')));
        type_text(&mut app, "   ");
        app.handle_key_event(key(KeyCode::Enter));
        assert!(matches!(&app.mode, Mode::Normal));
        assert!(app.store.load_all().unwrap().is_empty());
        app.handle_key_event(key(KeyCode::Char('o')));
        type_text(&mut app, "discard me");
        app.handle_key_event(key(KeyCode::Esc));
        assert!(matches!(&app.mode, Mode::Normal));
        assert!(app.store.load_all().unwrap().is_empty());
    }

    #[test]
    fn unavailable_persistence_does_not_open_an_editor() {
        let mut app = app_with_sections(vec![section("main")]);
        app.persistence_available = false;
        app.error = Some("database unavailable".to_owned());

        app.handle_key_event(key(KeyCode::Char('o')));

        assert!(matches!(&app.mode, Mode::Normal));
        assert_eq!(app.error.as_deref(), Some("database unavailable"));
    }

    #[test]
    fn edit_save_updates_title_and_returns_to_normal_with_focus() {
        let mut app = app_with_sections(vec![section("main")]);
        let todo = app
            .store
            .insert_todo("refs/heads/main", "old", None)
            .unwrap();
        app.store.toggle_todo(todo.id).unwrap();
        app.reload();
        app.focus = Some(Focus::Todo(todo.id));

        app.handle_key_event(key(KeyCode::Char('i')));
        assert_eq!(editor(&app).text, "old");
        assert_eq!(editor(&app).cursor, "old".len());
        app.handle_key_event(key(KeyCode::Home));
        for _ in 0..3 {
            app.handle_key_event(key(KeyCode::Delete));
        }
        type_text(&mut app, "new title");
        app.handle_key_event(key(KeyCode::Enter));

        assert!(matches!(&app.mode, Mode::Normal));
        assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
        let persisted = app.store.load_all().unwrap();
        assert_eq!(persisted[0].title, "new title");
        assert!(persisted[0].completed);
        assert_eq!(app.todos[0].title, "new title");
        assert!(app.todos[0].completed);
    }

    #[test]
    fn edit_escape_cancels_without_changing_the_todo() {
        let mut app = app_with_sections(vec![section("main")]);
        let todo = app
            .store
            .insert_todo("refs/heads/main", "unchanged", None)
            .unwrap();
        app.reload();
        app.focus = Some(Focus::Todo(todo.id));

        app.handle_key_event(key(KeyCode::Char('i')));
        type_text(&mut app, " addition");
        app.handle_key_event(key(KeyCode::Esc));

        assert!(matches!(&app.mode, Mode::Normal));
        assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
        assert_eq!(app.store.load_all().unwrap()[0].title, "unchanged");
        assert_eq!(app.todos[0].title, "unchanged");
    }

    #[test]
    fn blank_edit_is_rejected_and_retains_the_editor_buffer() {
        let mut app = app_with_sections(vec![section("main")]);
        let todo = app
            .store
            .insert_todo("refs/heads/main", "keep", None)
            .unwrap();
        app.reload();
        app.focus = Some(Focus::Todo(todo.id));

        app.handle_key_event(key(KeyCode::Char('i')));
        app.handle_key_event(key(KeyCode::Home));
        for _ in 0..4 {
            app.handle_key_event(key(KeyCode::Delete));
        }
        app.handle_key_event(key(KeyCode::Enter));

        assert!(matches!(&app.mode, Mode::Insert(_)));
        assert!(editor(&app).text.is_empty());
        assert_eq!(editor(&app).cursor, 0);
        assert!(app.error.is_some());
        assert_eq!(app.store.load_all().unwrap()[0].title, "keep");
        assert_eq!(app.todos[0].title, "keep");
    }

    #[test]
    fn failed_edit_keeps_the_editor_buffer_and_original_todo() {
        let mut app = app_with_sections(vec![section("main")]);
        let todo = app
            .store
            .insert_todo("refs/heads/main", "original", None)
            .unwrap();
        app.reload();
        app.focus = Some(Focus::Todo(todo.id));
        app.handle_key_event(key(KeyCode::Char('i')));
        type_text(&mut app, " retained");
        let Mode::Insert(active_editor) = &mut app.mode else {
            panic!("expected insert mode");
        };
        active_editor.target = EditorTarget::Update { id: i64::MAX };

        app.handle_key_event(key(KeyCode::Enter));

        assert!(matches!(&app.mode, Mode::Insert(_)));
        assert_eq!(editor(&app).text, "original retained");
        assert!(app.error.is_some());
        assert_eq!(app.store.load_all().unwrap()[0].title, "original");
        assert_eq!(app.todos[0].title, "original");
    }

    #[test]
    fn branch_i_is_a_no_op() {
        let mut app = app_with_sections(vec![section("main")]);
        let focus = app.focus.clone();
        app.error = Some("existing error".to_owned());

        app.handle_key_event(key(KeyCode::Char('i')));

        assert!(matches!(&app.mode, Mode::Normal));
        assert_eq!(app.focus, focus);
        assert_eq!(app.error.as_deref(), Some("existing error"));
    }

    #[test]
    fn edit_renders_in_place_with_the_original_completion_marker() {
        let mut app = app_with_sections(vec![section("main")]);
        let first = app
            .store
            .insert_todo("refs/heads/main", "first", None)
            .unwrap();
        let second = app
            .store
            .insert_todo("refs/heads/main", "second", Some(first.id))
            .unwrap();
        app.store.toggle_todo(second.id).unwrap();
        app.reload();
        app.focus = Some(Focus::Todo(second.id));
        app.handle_key_event(key(KeyCode::Char('i')));
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| app.draw(frame)).unwrap();

        assert!(row_text(&terminal, 3).contains("[ ] first"));
        assert!(row_text(&terminal, 4).contains("[x] second"));
        assert!(!row_text(&terminal, 5).contains("["));
        assert_eq!(
            terminal.backend_mut().get_cursor_position().unwrap(),
            Position::new(15, 4)
        );
        assert_eq!(app.focus, Some(Focus::Todo(second.id)));
    }

    #[test]
    fn normal_mode_toggles_focused_todo_and_persists_each_state() {
        let mut app = app_with_sections(vec![section("main")]);
        let todo = app
            .store
            .insert_todo("refs/heads/main", "toggle me", None)
            .unwrap();
        app.reload();
        app.focus = Some(Focus::Todo(todo.id));
        app.error = Some("old error".to_owned());

        app.handle_key_event(key(KeyCode::Char('x')));

        assert!(
            app.todos
                .iter()
                .find(|item| item.id == todo.id)
                .unwrap()
                .completed
        );
        assert!(app.store.load_all().unwrap()[0].completed);
        assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
        assert!(app.error.is_none());
        let backend = TestBackend::new(40, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(row_text(&terminal, 3).contains("[x] toggle me"));
        assert_eq!(
            terminal.backend().buffer()[(5, 3)].fg,
            app.theme.foreground_muted
        );
        assert_eq!(
            terminal.backend().buffer()[(9, 3)].fg,
            app.theme.foreground_muted
        );

        app.handle_key_event(key(KeyCode::Char(' ')));

        assert!(
            !app.todos
                .iter()
                .find(|item| item.id == todo.id)
                .unwrap()
                .completed
        );
        assert!(!app.store.load_all().unwrap()[0].completed);
        assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(row_text(&terminal, 3).contains("[ ] toggle me"));
        assert_eq!(terminal.backend().buffer()[(5, 3)].fg, app.theme.foreground);
        assert_eq!(terminal.backend().buffer()[(9, 3)].fg, app.theme.foreground);
    }

    #[test]
    fn toggle_keys_are_no_ops_on_branch_headers_or_without_persistence() {
        let mut app = app_with_sections(vec![section("main")]);
        let todo = app
            .store
            .insert_todo("refs/heads/main", "unchanged", None)
            .unwrap();
        app.reload();
        app.error = Some("existing error".to_owned());

        app.handle_key_event(key(KeyCode::Char('x')));
        app.handle_key_event(key(KeyCode::Char(' ')));

        assert!(!app.store.load_all().unwrap()[0].completed);
        assert_eq!(app.focus, Some(Focus::Branch("refs/heads/main".to_owned())));
        assert_eq!(app.error.as_deref(), Some("existing error"));

        app.focus = Some(Focus::Todo(todo.id));
        app.persistence_available = false;
        app.handle_key_event(key(KeyCode::Char('x')));
        assert!(!app.store.load_all().unwrap()[0].completed);
        assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
        assert_eq!(app.error.as_deref(), Some("existing error"));
    }

    #[test]
    fn insert_mode_accepts_x_and_space_as_text() {
        let mut app = app_with_sections(vec![section("main")]);
        app.handle_key_event(key(KeyCode::Char('o')));

        app.handle_key_event(key(KeyCode::Char('x')));
        app.handle_key_event(key(KeyCode::Char(' ')));

        assert!(matches!(&app.mode, Mode::Insert(_)));
        assert_eq!(editor(&app).text, "x ");
        assert!(app.store.load_all().unwrap().is_empty());
    }

    #[test]
    fn failed_toggle_preserves_focus_and_in_memory_state() {
        let mut app = app_with_sections(vec![section("main")]);
        app.focus = Some(Focus::Todo(i64::MAX));

        app.handle_key_event(key(KeyCode::Char('x')));

        assert_eq!(app.focus, Some(Focus::Todo(i64::MAX)));
        assert!(app.todos.is_empty());
        assert!(app.error.is_some());
    }

    #[test]
    fn failed_commit_keeps_the_editor_and_insert_mode() {
        let mut app = app_with_sections(vec![section("main")]);
        app.handle_key_event(key(KeyCode::Char('o')));
        type_text(&mut app, "keep me");
        let Mode::Insert(active_editor) = &mut app.mode else {
            panic!("expected insert mode");
        };
        let EditorTarget::Create { after, .. } = &mut active_editor.target else {
            panic!("expected create editor");
        };
        *after = Some(i64::MAX);

        app.handle_key_event(key(KeyCode::Enter));

        assert!(matches!(&app.mode, Mode::Insert(_)));
        assert_eq!(editor(&app).text, "keep me");
        assert!(app.error.is_some());
        assert!(app.store.load_all().unwrap().is_empty());
    }

    #[test]
    fn unicode_editing_uses_grapheme_boundaries() {
        let mut app = app_with_sections(vec![section("main")]);
        app.handle_key_event(key(KeyCode::Char('o')));
        type_text(&mut app, "a界é");
        app.handle_key_event(key(KeyCode::Left));
        app.handle_key_event(key(KeyCode::Backspace));
        app.handle_key_event(key(KeyCode::Char('好')));
        app.handle_key_event(key(KeyCode::Home));
        app.handle_key_event(key(KeyCode::Right));
        app.handle_key_event(key(KeyCode::Left));
        app.handle_key_event(key(KeyCode::Delete));
        app.handle_key_event(key(KeyCode::End));
        assert_eq!(editor(&app).text, "好é");
        assert_eq!(editor(&app).cursor, "好é".len());
    }

    #[test]
    fn modified_arrows_move_between_unicode_word_boundaries() {
        let mut app = app_with_sections(vec![section("main")]);
        app.handle_key_event(key(KeyCode::Char('o')));
        type_text(&mut app, "alpha βeta gamma");

        app.handle_key_event(modified_key(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(editor(&app).cursor, "alpha βeta ".len());
        app.handle_key_event(modified_key(KeyCode::Left, KeyModifiers::SHIFT));
        assert_eq!(editor(&app).cursor, "alpha ".len());
        app.handle_key_event(modified_key(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(editor(&app).cursor, 0);
        app.handle_key_event(modified_key(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(editor(&app).cursor, 0);

        app.handle_key_event(modified_key(KeyCode::Right, KeyModifiers::SHIFT));
        assert_eq!(editor(&app).cursor, "alpha ".len());
        app.handle_key_event(modified_key(KeyCode::Right, KeyModifiers::CONTROL));
        assert_eq!(editor(&app).cursor, "alpha βeta ".len());
        app.handle_key_event(modified_key(KeyCode::Right, KeyModifiers::SHIFT));
        assert_eq!(editor(&app).cursor, "alpha βeta gamma".len());
        app.handle_key_event(modified_key(KeyCode::Right, KeyModifiers::CONTROL));
        assert_eq!(editor(&app).cursor, "alpha βeta gamma".len());
    }

    #[test]
    fn insertion_keeps_cursor_on_a_full_string_grapheme_boundary() {
        let mut app = app_with_sections(vec![section("main")]);
        app.handle_key_event(key(KeyCode::Char('o')));
        type_text(&mut app, "👩🔬");
        app.handle_key_event(key(KeyCode::Left));

        app.handle_key_event(key(KeyCode::Char('\u{200d}')));

        let input = editor(&app);
        assert_eq!(input.text, "👩‍🔬");
        assert_eq!(input.cursor, input.text.len());
        app.handle_key_event(key(KeyCode::Backspace));
        assert!(editor(&app).text.is_empty());
    }

    #[test]
    fn unselected_navigation_starts_at_directional_edge() {
        let build_app = || {
            let mut app = app_with_sections(vec![section("main"), section("topic")]);
            app.store
                .insert_todo("refs/heads/main", "top", None)
                .unwrap();
            let bottom = app
                .store
                .insert_todo("refs/heads/topic", "bottom", None)
                .unwrap();
            app.reload();
            app.focus = None;
            (app, bottom.id)
        };

        for code in [KeyCode::Char('j'), KeyCode::Down] {
            let (mut app, _) = build_app();
            app.handle_key_event(key(code));
            assert_eq!(app.focus, Some(Focus::Branch("refs/heads/main".to_owned())));
        }

        for code in [KeyCode::Char('k'), KeyCode::Up] {
            let (mut app, bottom_id) = build_app();
            app.handle_key_event(key(code));
            assert_eq!(app.focus, Some(Focus::Todo(bottom_id)));
        }
    }

    #[test]
    fn flattened_navigation_visits_headers_and_todos() {
        let mut app = app_with_sections(vec![section("main"), section("topic")]);
        let todo = app
            .store
            .insert_todo("refs/heads/main", "one", None)
            .unwrap();
        app.reload();
        app.handle_key_event(key(KeyCode::Char('j')));
        assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
        app.handle_key_event(key(KeyCode::Char('j')));
        assert_eq!(
            app.focus,
            Some(Focus::Branch("refs/heads/topic".to_owned()))
        );
        app.handle_key_event(key(KeyCode::Char('k')));
        assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
    }

    #[test]
    fn bracket_navigation_moves_between_section_headers() {
        let mut app = app_with_sections(vec![section("main"), section("topic"), section("third")]);

        app.handle_key_event(key(KeyCode::Char(']')));
        assert_eq!(
            app.focus,
            Some(Focus::Branch("refs/heads/topic".to_owned()))
        );
        app.handle_key_event(key(KeyCode::Char(']')));
        assert_eq!(
            app.focus,
            Some(Focus::Branch("refs/heads/third".to_owned()))
        );
        app.handle_key_event(key(KeyCode::Char(']')));
        assert_eq!(
            app.focus,
            Some(Focus::Branch("refs/heads/third".to_owned()))
        );

        app.handle_key_event(key(KeyCode::Char('[')));
        assert_eq!(
            app.focus,
            Some(Focus::Branch("refs/heads/topic".to_owned()))
        );
        app.handle_key_event(key(KeyCode::Char('[')));
        assert_eq!(app.focus, Some(Focus::Branch("refs/heads/main".to_owned())));
        app.handle_key_event(key(KeyCode::Char('[')));
        assert_eq!(app.focus, Some(Focus::Branch("refs/heads/main".to_owned())));
    }

    #[test]
    fn bracket_navigation_uses_nearest_header_and_directional_edge() {
        let mut app = app_with_sections(vec![section("main"), section("topic")]);
        let topic_todo = app
            .store
            .insert_todo("refs/heads/topic", "topic todo", None)
            .unwrap();
        app.reload();

        app.focus = Some(Focus::Todo(topic_todo.id));
        app.handle_key_event(key(KeyCode::Char('[')));
        assert_eq!(
            app.focus,
            Some(Focus::Branch("refs/heads/topic".to_owned()))
        );

        app.focus = None;
        app.handle_key_event(key(KeyCode::Char(']')));
        assert_eq!(app.focus, Some(Focus::Branch("refs/heads/main".to_owned())));
        app.focus = None;
        app.handle_key_event(key(KeyCode::Char('[')));
        assert_eq!(
            app.focus,
            Some(Focus::Branch("refs/heads/topic".to_owned()))
        );
    }

    #[test]
    fn arrow_navigation_visits_headers_and_todos() {
        let mut app = app_with_sections(vec![section("main"), section("topic")]);
        let todo = app
            .store
            .insert_todo("refs/heads/main", "one", None)
            .unwrap();
        app.reload();

        app.handle_key_event(key(KeyCode::Down));
        assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
        app.handle_key_event(key(KeyCode::Down));
        assert_eq!(
            app.focus,
            Some(Focus::Branch("refs/heads/topic".to_owned()))
        );
        app.handle_key_event(key(KeyCode::Up));
        assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
    }

    #[test]
    fn focused_todo_and_unicode_cursor_remain_visible() {
        let mut app = app_with_sections(vec![section("main"), section("topic")]);
        let todo = app
            .store
            .insert_todo("refs/heads/topic", "visible", None)
            .unwrap();
        app.reload();
        app.focus = Some(Focus::Todo(todo.id));
        let backend = TestBackend::new(18, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(row_text(&terminal, 2).contains("visible"));

        app.handle_key_event(key(KeyCode::Char('o')));
        type_text(&mut app, "ab界cd界ef");
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let cursor = terminal.backend_mut().get_cursor_position().unwrap();
        assert_eq!(cursor.y, 2);
        assert!(cursor.x < 17);
    }

    #[test]
    fn exact_width_editor_keeps_a_full_grapheme_visible() {
        let mut app = app_with_sections(vec![section("main")]);
        app.handle_key_event(key(KeyCode::Char('o')));
        type_text(&mut app, "👩‍🔬");
        let backend = TestBackend::new(12, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| app.draw(frame)).unwrap();

        assert_eq!(
            terminal.backend_mut().get_cursor_position().unwrap(),
            Position::new(10, 2)
        );
        assert_eq!(terminal.backend().buffer()[(9, 2)].symbol(), "👩‍🔬");
    }

    #[test]
    fn refresh_loads_other_process_commits_and_adds_their_branch() {
        let directory = std::env::temp_dir().join(format!(
            "tuido-main-refresh-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let database = directory.join("todos.db");
        let store = TodoStore::open(&database).unwrap();
        let mut app = app_with_sections(vec![section("main")]);
        app.store = store;
        app.data_version = app.store.data_version().unwrap();
        let mut other = TodoStore::open(&database).unwrap();

        other
            .insert_todo("refs/heads/stored-only", "from another process", None)
            .unwrap();
        app.refresh_external();

        assert_eq!(app.todos[0].title, "from another process");
        assert!(app.repository.sections.iter().any(|section| {
            section.full_ref_name == "refs/heads/stored-only" && section.is_stored_only
        }));
        drop(other);
        drop(app);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn refresh_retries_an_unknown_snapshot_without_a_new_commit() {
        let directory = std::env::temp_dir().join(format!(
            "tuido-main-unknown-version-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let database = directory.join("todos.db");
        let mut writer = TodoStore::open(&database).unwrap();
        writer
            .insert_todo("refs/heads/main", "already committed", None)
            .unwrap();
        let store = TodoStore::open(&database).unwrap();
        let mut app = app_with_sections(vec![section("main")]);
        app.store = store;
        app.data_version = UNKNOWN_DATA_VERSION;

        app.refresh_external();

        assert_eq!(app.todos[0].title, "already committed");
        drop(writer);
        drop(app);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn hover_background_applies_to_unselected_todo_and_branch_rows() {
        let mut app = app_with_sections(vec![section("main"), section("topic")]);
        app.store
            .insert_todo("refs/heads/main", "hover me", None)
            .unwrap();
        app.reload();
        let theme = app.theme;
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).unwrap();

        app.handle_mouse_event(mouse(MouseEventKind::Moved, 2, 3));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(row_text(&terminal, 3).contains("[ ] hover me"));
        assert_eq!(
            terminal.backend().buffer()[(2, 3)].bg,
            theme.hover_background
        );

        app.handle_mouse_event(mouse(MouseEventKind::Moved, 2, 4));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(row_text(&terminal, 4).contains("topic"));
        assert_eq!(
            terminal.backend().buffer()[(2, 4)].bg,
            theme.hover_background
        );
    }

    #[test]
    fn unfocused_rows_render_without_selection() {
        let mut app = app_with_sections(vec![section("main")]);
        app.focus = None;
        let theme = app.theme;
        let backend = TestBackend::new(40, 6);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| app.draw(frame)).unwrap();

        assert_eq!(terminal.backend().buffer()[(2, 2)].bg, theme.background);
    }

    #[test]
    fn focus_repair_preserves_explicit_deselection() {
        let mut app = app_with_sections(vec![section("main")]);
        app.focus = None;

        app.repair_focus();

        assert_eq!(app.focus, None);
    }

    #[test]
    fn selected_background_takes_precedence_over_hover() {
        let mut app = app_with_sections(vec![section("main")]);
        let theme = app.theme;
        let backend = TestBackend::new(40, 6);
        let mut terminal = Terminal::new(backend).unwrap();

        app.handle_mouse_event(mouse(MouseEventKind::Moved, 2, 2));
        terminal.draw(|frame| app.draw(frame)).unwrap();

        assert_eq!(
            terminal.backend().buffer()[(2, 2)].bg,
            theme.selection_background
        );
        assert_ne!(
            terminal.backend().buffer()[(2, 2)].bg,
            theme.hover_background
        );
    }

    #[test]
    fn normal_left_click_selects_the_rendered_row() {
        let mut app = app_with_sections(vec![section("main")]);
        let todo = app
            .store
            .insert_todo("refs/heads/main", "click me", None)
            .unwrap();
        app.reload();
        let backend = TestBackend::new(40, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();

        app.handle_mouse_event(mouse(MouseEventKind::Down(MouseButton::Left), 2, 3));

        assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
    }

    #[test]
    fn insert_mode_click_does_not_change_focus() {
        let mut app = app_with_sections(vec![section("main"), section("topic")]);
        app.handle_key_event(key(KeyCode::Char('o')));
        let original_focus = app.focus.clone();
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(row_text(&terminal, 4).contains("topic"));

        app.handle_mouse_event(mouse(MouseEventKind::Down(MouseButton::Left), 2, 4));

        assert!(matches!(&app.mode, Mode::Insert(_)));
        assert_eq!(app.focus, original_focus);
    }

    #[test]
    fn clicks_on_empty_rows_and_outside_the_viewport_deselect() {
        let mut app = app_with_sections(vec![section("main"), section("topic")]);
        let topic = Focus::Branch("refs/heads/topic".to_owned());
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(row_text(&terminal, 3).contains("No todos"));

        for (column, row) in [(2, 3), (0, 2), (39, 2), (2, 6)] {
            app.focus = Some(topic.clone());
            app.handle_mouse_event(mouse(MouseEventKind::Down(MouseButton::Left), column, row));
            assert_eq!(app.focus, None);
        }
    }

    #[test]
    fn click_hit_testing_uses_the_focus_driven_scroll_offset() {
        let mut app = app_with_sections(vec![section("main"), section("topic"), section("third")]);
        for branch in ["main", "topic", "third"] {
            app.store
                .insert_todo(
                    &format!("refs/heads/{branch}"),
                    &format!("{branch} todo"),
                    None,
                )
                .unwrap();
        }
        app.reload();
        app.focus = Some(Focus::Branch("refs/heads/third".to_owned()));
        let backend = TestBackend::new(40, 7);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(row_text(&terminal, 2).contains("topic"));
        assert!(row_text(&terminal, 4).contains("third"));

        app.handle_mouse_event(mouse(MouseEventKind::Down(MouseButton::Left), 2, 2));

        assert_eq!(
            app.focus,
            Some(Focus::Branch("refs/heads/topic".to_owned()))
        );
    }

    #[test]
    fn narrow_header_hides_tag_and_stored_scope_shows_branch_tag() {
        let theme = test_theme();
        let normal = section("very-long-branch");
        let stored = BranchSection {
            full_ref_name: "refs/heads/archive".to_owned(),
            display_name: "archive".to_owned(),
            worktree_path: Default::default(),
            is_current: false,
            is_locked: false,
            is_stored_only: true,
        };
        let mut app = app_with_sections(vec![normal, stored]);
        let backend = TestBackend::new(21, 9);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(row_text(&terminal, 1).starts_with('┌'));
        assert_eq!(
            terminal.backend().buffer()[(0, 1)].fg,
            theme.mode_background
        );
        assert!(!row_text(&terminal, 2).contains("WORKTREE"));
        assert!(row_text(&terminal, 3).contains("No todos"));
        assert!(row_text(&terminal, 4).contains("BRANCH"));
        assert!(row_text(&terminal, 7).starts_with('└'));
        assert_eq!(
            terminal.backend().buffer()[(1, 2)].bg,
            theme.selection_background
        );
    }

    #[test]
    fn long_titles_wrap_at_words_and_shift_following_rows() {
        let mut app = app_with_sections(vec![section("main")]);
        let first = app
            .store
            .insert_todo("refs/heads/main", "alpha beta gamma delta", None)
            .unwrap();
        app.store
            .insert_todo("refs/heads/main", "following", Some(first.id))
            .unwrap();
        app.reload();
        let backend = TestBackend::new(24, 9);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| app.draw(frame)).unwrap();

        assert!(row_text(&terminal, 3).starts_with("│    [ ] alpha beta"));
        assert!(row_text(&terminal, 4).starts_with("│        gamma delta"));
        assert!(row_text(&terminal, 5).starts_with("│    [ ] following"));
    }

    #[test]
    fn editor_wraps_as_text_is_typed_and_shifts_following_rows() {
        let mut app = app_with_sections(vec![section("main")]);
        app.store
            .insert_todo("refs/heads/main", "following", None)
            .unwrap();
        app.reload();
        app.focus = Some(Focus::Branch("refs/heads/main".to_owned()));
        app.handle_key_event(key(KeyCode::Char('o')));
        type_text(&mut app, "alpha beta gamma delta");
        let backend = TestBackend::new(24, 9);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| app.draw(frame)).unwrap();

        assert!(row_text(&terminal, 3).starts_with("│    [ ] alpha beta"));
        assert!(row_text(&terminal, 4).starts_with("│        gamma delta"));
        assert!(row_text(&terminal, 5).starts_with("│    [ ] following"));
        assert_eq!(
            terminal.backend_mut().get_cursor_position().unwrap(),
            Position::new(20, 4)
        );

        for expected in [
            Position::new(15, 4),
            Position::new(9, 4),
            Position::new(15, 3),
        ] {
            app.handle_key_event(modified_key(KeyCode::Left, KeyModifiers::CONTROL));
            terminal.draw(|frame| app.draw(frame)).unwrap();
            assert_eq!(
                terminal.backend_mut().get_cursor_position().unwrap(),
                expected
            );
        }
    }

    #[test]
    fn unicode_overlong_words_wrap_without_splitting_graphemes() {
        let mut app = app_with_sections(vec![section("main")]);
        app.store
            .insert_todo("refs/heads/main", "ab👩‍🔬cd界ef\nnext", None)
            .unwrap();
        app.reload();
        let backend = TestBackend::new(16, 9);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| app.draw(frame)).unwrap();

        assert!(row_text(&terminal, 3).starts_with("│    [ ] ab"));
        assert!(row_text(&terminal, 4).starts_with("│        "));
        assert!(row_text(&terminal, 5).starts_with("│        next"));
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(11, 3)].symbol(), "👩‍🔬");
        assert_eq!(buffer[(13, 3)].symbol(), "c");
        assert_eq!(buffer[(14, 3)].symbol(), "d");
        assert_eq!(buffer[(9, 4)].symbol(), "界");
        assert_eq!(buffer[(11, 4)].symbol(), "e");
        assert_eq!(buffer[(12, 4)].symbol(), "f");
    }

    #[test]
    fn selection_and_hover_cover_every_wrapped_line_and_continuations_hit_test() {
        let mut app = app_with_sections(vec![section("main")]);
        let todo = app
            .store
            .insert_todo("refs/heads/main", "alpha beta gamma delta", None)
            .unwrap();
        app.reload();
        let theme = app.theme;
        let backend = TestBackend::new(24, 8);
        let mut terminal = Terminal::new(backend).unwrap();

        app.focus = Some(Focus::Todo(todo.id));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        for row in [3, 4] {
            for column in 1..23 {
                assert_eq!(
                    terminal.backend().buffer()[(column, row)].bg,
                    theme.selection_background
                );
            }
        }

        app.focus = None;
        app.handle_mouse_event(mouse(MouseEventKind::Moved, 10, 4));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        for row in [3, 4] {
            for column in 1..23 {
                assert_eq!(
                    terminal.backend().buffer()[(column, row)].bg,
                    theme.hover_background
                );
            }
        }

        app.handle_mouse_event(mouse(MouseEventKind::Down(MouseButton::Left), 10, 4));
        assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
    }

    #[test]
    fn scrolling_keeps_a_wrapped_focus_visible_and_preserves_hit_testing() {
        let mut app = app_with_sections(vec![section("main")]);
        let leading = app
            .store
            .insert_todo("refs/heads/main", "leading", None)
            .unwrap();
        let wrapped = app
            .store
            .insert_todo("refs/heads/main", "one two three", Some(leading.id))
            .unwrap();
        app.reload();
        app.focus = Some(Focus::Todo(wrapped.id));
        let theme = app.theme;
        let backend = TestBackend::new(18, 7);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| app.draw(frame)).unwrap();

        assert!(row_text(&terminal, 2).starts_with("│    [ ] leading"));
        assert!(row_text(&terminal, 3).starts_with("│    [ ] one two"));
        assert!(row_text(&terminal, 4).starts_with("│        three"));
        for row in [3, 4] {
            assert_eq!(
                terminal.backend().buffer()[(1, row)].bg,
                theme.selection_background
            );
        }

        app.handle_mouse_event(mouse(MouseEventKind::Down(MouseButton::Left), 2, 2));
        assert_eq!(app.focus, Some(Focus::Todo(leading.id)));
    }

    #[test]
    fn very_narrow_terminals_render_wrapped_todos_without_panicking() {
        let mut app = app_with_sections(vec![section("main")]);
        app.store
            .insert_todo("refs/heads/main", "👩‍🔬overlong", None)
            .unwrap();
        app.reload();

        for width in 1..=9 {
            let backend = TestBackend::new(width, 5);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|frame| app.draw(frame)).unwrap();
        }
    }
}

fn main() -> std::io::Result<()> {
    ratatui::run(|terminal| App::default().run(terminal))
}

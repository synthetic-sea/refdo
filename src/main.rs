mod repository;
mod storage;
mod theme;

use std::{io, time::Duration};

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum InputMode {
    #[default]
    Normal,
    Insert,
}

impl InputMode {
    const fn label(self) -> &'static str {
        match self {
            Self::Normal => " NORMAL ",
            Self::Insert => " INSERT ",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Focus {
    Branch(String),
    Todo(TodoId),
}

#[derive(Clone, Debug)]
struct Draft {
    branch_ref: String,
    after: Option<TodoId>,
    text: String,
    cursor: usize,
    origin: Focus,
}

struct App {
    exit: bool,
    repository: RepositoryContext,
    store: TodoStore,
    persistence_available: bool,
    todos: Vec<Todo>,
    focus: Option<Focus>,
    draft: Option<Draft>,
    theme: Theme,
    input_mode: InputMode,
    data_version: i64,
    error: Option<String>,
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
        let focus = repository
            .sections
            .first()
            .map(|section| Focus::Branch(section.full_ref_name.clone()));

        Self {
            exit: false,
            repository,
            store,
            persistence_available,
            todos,
            focus,
            draft: None,
            theme,
            input_mode: InputMode::Normal,
            data_version,
            error,
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            self.handle_events()?;
        }
        Ok(())
    }

    fn draw(&self, frame: &mut Frame) {
        let [status_area, content_area, footer_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        render_status_bar(frame, status_area, &self.repository.head_label, &self.theme);
        let content_block = Block::bordered()
            .style(Style::default().bg(self.theme.background))
            .border_style(Style::default().fg(self.theme.foreground));
        let todo_area = content_block.inner(content_area);
        frame.render_widget(content_block, content_area);
        render_branch_sections(
            frame,
            todo_area,
            &self.repository.sections,
            &self.todos,
            self.focus.as_ref(),
            self.draft.as_ref(),
            &self.theme,
        );
        render_footer(
            frame,
            footer_area,
            self.input_mode,
            self.error.as_deref(),
            &self.theme,
        );
    }

    fn handle_events(&mut self) -> io::Result<()> {
        if event::poll(Duration::from_millis(75))?
            && let Event::Key(key) = event::read()?
        {
            self.handle_key_event(key);
        }
        self.refresh_external();
        Ok(())
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
        let valid = match self.focus.as_ref() {
            Some(Focus::Branch(branch_ref)) => self
                .repository
                .sections
                .iter()
                .any(|section| section.full_ref_name == *branch_ref),
            Some(Focus::Todo(id)) => self.todos.iter().any(|todo| todo.id == *id),
            None => false,
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
            self.focus = rows.into_iter().next();
            return;
        };
        let next = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current.saturating_add(delta as usize).min(rows.len() - 1)
        };
        self.focus = Some(rows[next].clone());
    }

    fn open_draft(&mut self) {
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
        self.draft = Some(Draft {
            branch_ref,
            after,
            text: String::new(),
            cursor: 0,
            origin,
        });
        self.input_mode = InputMode::Insert;
        self.error = None;
    }

    fn discard_draft(&mut self) {
        if let Some(draft) = self.draft.take() {
            self.focus = Some(draft.origin);
        }
        self.input_mode = InputMode::Normal;
        self.error = None;
    }

    fn commit_draft(&mut self) {
        let Some(draft) = self.draft.as_ref() else {
            return;
        };
        if draft.text.trim().is_empty() {
            self.discard_draft();
            return;
        }
        let result = self
            .store
            .insert_todo(&draft.branch_ref, &draft.text, draft.after);
        match result {
            Ok(todo) => {
                let committed = Focus::Todo(todo.id);
                let branch_ref = todo.branch_ref.clone();
                let todo_id = todo.id;
                self.integrate_todo(todo);
                self.focus = Some(committed.clone());
                self.draft = Some(Draft {
                    branch_ref,
                    after: Some(todo_id),
                    text: String::new(),
                    cursor: 0,
                    origin: committed,
                });
                if self.reload() {
                    self.error = None;
                }
            }
            Err(error) => self.error = Some(error.to_string()),
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

    fn handle_key_event(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        match self.input_mode {
            InputMode::Normal => match key.code {
                KeyCode::Char('j') => self.move_focus(1),
                KeyCode::Char('k') => self.move_focus(-1),
                KeyCode::Char('o') => self.open_draft(),
                KeyCode::Char('x' | ' ') => self.toggle_focused_todo(),
                KeyCode::Char('q') => self.exit = true,
                _ => {}
            },
            InputMode::Insert => match key.code {
                KeyCode::Char(character) => {
                    if let Some(draft) = self.draft.as_mut() {
                        draft.text.insert(draft.cursor, character);
                        let insertion_end = draft.cursor + character.len_utf8();
                        draft.cursor = boundary_at_or_after(&draft.text, insertion_end);
                    }
                }
                KeyCode::Backspace => {
                    if let Some(draft) = self.draft.as_mut()
                        && draft.cursor > 0
                    {
                        let previous = previous_boundary(&draft.text, draft.cursor);
                        draft.text.drain(previous..draft.cursor);
                        draft.cursor = previous;
                    }
                }
                KeyCode::Delete => {
                    if let Some(draft) = self.draft.as_mut()
                        && draft.cursor < draft.text.len()
                    {
                        let next = next_boundary(&draft.text, draft.cursor);
                        draft.text.drain(draft.cursor..next);
                    }
                }
                KeyCode::Left => {
                    if let Some(draft) = self.draft.as_mut() {
                        draft.cursor = previous_boundary(&draft.text, draft.cursor);
                    }
                }
                KeyCode::Right => {
                    if let Some(draft) = self.draft.as_mut() {
                        draft.cursor = next_boundary(&draft.text, draft.cursor);
                    }
                }
                KeyCode::Home => {
                    if let Some(draft) = self.draft.as_mut() {
                        draft.cursor = 0;
                    }
                }
                KeyCode::End => {
                    if let Some(draft) = self.draft.as_mut() {
                        draft.cursor = draft.text.len();
                    }
                }
                KeyCode::Enter => self.commit_draft(),
                KeyCode::Esc => self.discard_draft(),
                _ => {}
            },
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

fn boundary_at_or_after(text: &str, cursor: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(index, _)| index)
        .find(|index| *index >= cursor)
        .unwrap_or(text.len())
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

fn render_footer(
    frame: &mut Frame,
    area: Rect,
    input_mode: InputMode,
    error: Option<&str>,
    theme: &Theme,
) {
    let mode = Span::styled(
        input_mode.label(),
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
    Draft(&'a Draft),
    Empty,
    Gap,
}

fn display_rows<'a>(
    sections: &'a [BranchSection],
    todos: &'a [Todo],
    draft: Option<&'a Draft>,
) -> Vec<DisplayRow<'a>> {
    let mut rows = Vec::new();
    for (index, section) in sections.iter().enumerate() {
        if index > 0 {
            rows.push(DisplayRow::Gap);
        }
        rows.push(DisplayRow::Header(section));
        let branch_todos = todos
            .iter()
            .filter(|todo| todo.branch_ref == section.full_ref_name)
            .collect::<Vec<_>>();
        let branch_draft = draft.filter(|draft| draft.branch_ref == section.full_ref_name);
        if branch_todos.is_empty() && branch_draft.is_none() {
            rows.push(DisplayRow::Empty);
            continue;
        }
        for todo in branch_todos {
            rows.push(DisplayRow::Todo(todo));
            if branch_draft.is_some_and(|draft| draft.after == Some(todo.id)) {
                rows.push(DisplayRow::Draft(branch_draft.expect("checked above")));
            }
        }
        if branch_draft.is_some_and(|draft| draft.after.is_none()) {
            rows.push(DisplayRow::Draft(branch_draft.expect("checked above")));
        }
    }
    rows
}

fn render_branch_sections(
    frame: &mut Frame,
    area: Rect,
    sections: &[BranchSection],
    todos: &[Todo],
    focus: Option<&Focus>,
    draft: Option<&Draft>,
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

    let rows = display_rows(sections, todos, draft);
    let focused_row = rows.iter().position(|row| match row {
        DisplayRow::Header(section) => {
            draft.is_none()
                && matches!(focus, Some(Focus::Branch(branch_ref)) if branch_ref == &section.full_ref_name)
        }
        DisplayRow::Todo(todo) => {
            draft.is_none() && matches!(focus, Some(Focus::Todo(id)) if *id == todo.id)
        }
        DisplayRow::Draft(_) => true,
        DisplayRow::Empty | DisplayRow::Gap => false,
    });
    let first = focused_row
        .map(|row| {
            row.saturating_add(1)
                .saturating_sub(usize::from(area.height))
        })
        .unwrap_or(0);

    for (visible, row) in rows
        .iter()
        .skip(first)
        .take(usize::from(area.height))
        .enumerate()
    {
        let row_area = Rect::new(area.x, area.y + visible as u16, area.width, 1);
        match row {
            DisplayRow::Header(section) => {
                let selected = draft.is_none()
                    && matches!(focus, Some(Focus::Branch(branch_ref)) if branch_ref == &section.full_ref_name);
                render_branch_header(frame, row_area, section, selected, theme);
            }
            DisplayRow::Todo(todo) => {
                let selected =
                    draft.is_none() && matches!(focus, Some(Focus::Todo(id)) if *id == todo.id);
                let background = if selected {
                    theme.selection_background
                } else {
                    theme.background
                };
                let marker = if todo.completed { "[x]" } else { "[ ]" };
                let foreground = if todo.completed {
                    theme.foreground_muted
                } else {
                    theme.foreground
                };
                frame.render_widget(
                    Paragraph::new(format!("    {marker} {}", todo.title))
                        .style(Style::default().fg(foreground).bg(background)),
                    row_area,
                );
            }
            DisplayRow::Draft(draft) => render_draft(frame, row_area, draft, theme),
            DisplayRow::Gap => {}
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
    }
}

fn render_branch_header(
    frame: &mut Frame,
    area: Rect,
    section: &BranchSection,
    selected: bool,
    theme: &Theme,
) {
    let background = if selected {
        theme.selection_background
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

fn render_draft(frame: &mut Frame, area: Rect, draft: &Draft, theme: &Theme) {
    const PREFIX: &str = "    [ ] ";
    let background = theme.selection_background;
    frame.render_widget(
        Block::default().style(Style::default().bg(background)),
        area,
    );
    let available = usize::from(area.width).saturating_sub(UnicodeWidthStr::width(PREFIX));
    let before_cursor = &draft.text[..draft.cursor];
    let mut start = 0;
    let mut width = UnicodeWidthStr::width(before_cursor);
    while width >= available.max(1) && start < draft.cursor {
        let grapheme = before_cursor[start..]
            .graphemes(true)
            .next()
            .expect("valid grapheme boundary");
        start += grapheme.len();
        width = UnicodeWidthStr::width(&before_cursor[start..]);
    }
    let visible = &draft.text[start..];
    frame.render_widget(
        Paragraph::new(format!("{PREFIX}{visible}"))
            .style(Style::default().fg(theme.foreground).bg(background)),
        area,
    );
    if available > 0 {
        let cursor_x = area
            .x
            .saturating_add(UnicodeWidthStr::width(PREFIX) as u16)
            .saturating_add(width as u16)
            .min(area.right().saturating_sub(1));
        frame.set_cursor_position(Position::new(cursor_x, area.y));
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{
        Terminal,
        backend::{Backend, TestBackend},
        crossterm::event::KeyModifiers,
        style::Color,
    };

    use super::*;

    fn test_theme() -> Theme {
        Theme {
            background: Color::Red,
            foreground: Color::Green,
            foreground_muted: Color::Blue,
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
            draft: None,
            theme: test_theme(),
            input_mode: InputMode::Normal,
            data_version: 0,
            error: None,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_text(app: &mut App, text: &str) {
        for character in text.chars() {
            app.handle_key_event(key(KeyCode::Char(character)));
        }
    }

    fn row_text(terminal: &Terminal<TestBackend>, row: u16) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.width)
            .map(|column| buffer[(column, row)].symbol())
            .collect()
    }

    #[test]
    fn header_append_is_only_persisted_on_enter_and_chains_entries() {
        let mut app = app_with_sections(vec![section("main")]);
        app.handle_key_event(key(KeyCode::Char('o')));
        type_text(&mut app, "first");
        assert!(app.store.load_all().unwrap().is_empty());
        app.handle_key_event(key(KeyCode::Enter));
        assert_eq!(
            app.todos
                .iter()
                .map(|todo| todo.title.as_str())
                .collect::<Vec<_>>(),
            ["first"]
        );
        assert_eq!(app.input_mode, InputMode::Insert);
        type_text(&mut app, "second");
        app.handle_key_event(key(KeyCode::Enter));
        assert_eq!(
            app.todos
                .iter()
                .map(|todo| todo.title.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
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
        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.store.load_all().unwrap().is_empty());
        app.handle_key_event(key(KeyCode::Char('o')));
        type_text(&mut app, "discard me");
        app.handle_key_event(key(KeyCode::Esc));
        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.store.load_all().unwrap().is_empty());
    }

    #[test]
    fn unavailable_persistence_does_not_open_a_draft() {
        let mut app = app_with_sections(vec![section("main")]);
        app.persistence_available = false;
        app.error = Some("database unavailable".to_owned());

        app.handle_key_event(key(KeyCode::Char('o')));

        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.draft.is_none());
        assert_eq!(app.error.as_deref(), Some("database unavailable"));
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

        assert_eq!(app.input_mode, InputMode::Insert);
        assert_eq!(app.draft.as_ref().unwrap().text, "x ");
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
    fn failed_commit_keeps_the_draft_and_insert_mode() {
        let mut app = app_with_sections(vec![section("main")]);
        app.handle_key_event(key(KeyCode::Char('o')));
        type_text(&mut app, "keep me");
        app.draft.as_mut().unwrap().after = Some(i64::MAX);

        app.handle_key_event(key(KeyCode::Enter));

        assert_eq!(app.input_mode, InputMode::Insert);
        assert_eq!(app.draft.as_ref().unwrap().text, "keep me");
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
        assert_eq!(app.draft.as_ref().unwrap().text, "好é");
        assert_eq!(app.draft.as_ref().unwrap().cursor, "好é".len());
    }

    #[test]
    fn insertion_keeps_cursor_on_a_full_string_grapheme_boundary() {
        let mut app = app_with_sections(vec![section("main")]);
        app.handle_key_event(key(KeyCode::Char('o')));
        type_text(&mut app, "👩🔬");
        app.handle_key_event(key(KeyCode::Left));

        app.handle_key_event(key(KeyCode::Char('\u{200d}')));

        let draft = app.draft.as_ref().unwrap();
        assert_eq!(draft.text, "👩‍🔬");
        assert_eq!(draft.cursor, draft.text.len());
        app.handle_key_event(key(KeyCode::Backspace));
        assert!(app.draft.as_ref().unwrap().text.is_empty());
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
    fn draft_scrolling_does_not_split_emoji_graphemes() {
        let mut app = app_with_sections(vec![section("main")]);
        app.handle_key_event(key(KeyCode::Char('o')));
        type_text(&mut app, "👩‍🔬");
        let backend = TestBackend::new(12, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| app.draw(frame)).unwrap();

        let cursor = terminal.backend_mut().get_cursor_position().unwrap();
        assert_eq!(cursor, Position::new(9, 2));
        assert_eq!(row_text(&terminal, 2), "│    [ ]   │");
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
        let app = app_with_sections(vec![normal, stored]);
        let backend = TestBackend::new(21, 9);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(row_text(&terminal, 1).starts_with('┌'));
        assert_eq!(terminal.backend().buffer()[(0, 1)].fg, theme.foreground);
        assert!(!row_text(&terminal, 2).contains("WORKTREE"));
        assert_eq!(row_text(&terminal, 4), "│                   │");
        assert!(row_text(&terminal, 5).contains("BRANCH"));
        assert!(row_text(&terminal, 7).starts_with('└'));
        assert_eq!(
            terminal.backend().buffer()[(1, 2)].bg,
            theme.selection_background
        );
    }
}

fn main() -> std::io::Result<()> {
    ratatui::run(|terminal| App::default().run(terminal))
}

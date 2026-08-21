mod actions;
mod commands;
mod dispatch;
mod events;
mod external_editor;
mod navigation;
mod text_input;
mod ui;

use std::{
    collections::HashSet,
    io,
    path::PathBuf,
    time::{Duration, Instant},
};

use crossterm::clipboard::CopyToClipboard;

use ratatui::{
    DefaultTerminal, Frame, Terminal,
    backend::Backend,
    crossterm::{
        event::{DisableMouseCapture, EnableMouseCapture},
        execute,
        terminal::{EnterAlternateScreen, enable_raw_mode},
    },
    layout::{Position, Rect},
    style::Style,
    widgets::{Block, Padding},
};

use dispatch::DispatchController;

use crate::config::{DispatchConfigDigest, DispatchSettings, ThemeMode};
use crate::repository::RepositoryContext;
use crate::storage::{Todo, TodoId, TodoStore};
use crate::theme::{TOKYO_NIGHT_DAY, Theme};

const REPOSITORY_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const SYSTEM_THEME_REFRESH_INTERVAL: Duration = Duration::from_millis(500);
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(500);
const UNKNOWN_DATA_VERSION: i64 = -1;

#[derive(Clone, Debug)]
enum Mode {
    Normal,
    Select(SelectState),
    Preview(BodyPreview),
    Insert(Editor),
    Command(CommandLine),
    ConfirmClear(ClearConfirmation),
    ConfirmDispatchTrust(DispatchTrustConfirmation),
}

impl Mode {
    const fn label(&self) -> &'static str {
        match self {
            Self::Normal => " NORMAL ",
            Self::Select(_) => " SELECT ",
            Self::Preview(_) => " PREVIEW ",
            Self::Insert(_) => " INSERT ",
            Self::Command(_) => " COMMAND ",
            Self::ConfirmClear(_) | Self::ConfirmDispatchTrust(_) => " CONFIRM ",
        }
    }

    const fn editor(&self) -> Option<&Editor> {
        match self {
            Self::Insert(editor) => Some(editor),
            Self::Normal
            | Self::Select(_)
            | Self::Preview(_)
            | Self::Command(_)
            | Self::ConfirmClear(_)
            | Self::ConfirmDispatchTrust(_) => None,
        }
    }

    fn footer_message(&self) -> Option<&str> {
        match self {
            Self::ConfirmClear(confirmation) => Some(&confirmation.prompt),
            Self::ConfirmDispatchTrust(confirmation) => Some(&confirmation.prompt),
            Self::Normal
            | Self::Select(_)
            | Self::Preview(_)
            | Self::Insert(_)
            | Self::Command(_) => None,
        }
    }
}

#[derive(Clone, Debug)]
struct SelectState {
    branch_ref: String,
    selected_todo_ids: HashSet<TodoId>,
}

#[derive(Clone, Debug)]
struct BodyPreview {
    todo_id: TodoId,
    scroll: u16,
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

#[derive(Clone, Debug)]
struct CommandLine {
    target_branch: Option<String>,
    target_todos: Vec<TodoId>,
    text: String,
    cursor: usize,
}

#[derive(Clone, Debug)]
struct ClearConfirmation {
    target_branch: String,
    prompt: String,
}

#[derive(Clone, Debug)]
struct DispatchTrustConfirmation {
    digest: DispatchConfigDigest,
    display_name: String,
    worktree_path: PathBuf,
    prompt: String,
}

struct SystemThemeState {
    light_theme: Theme,
    dark_theme: Theme,
    last_checked: Instant,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingOperator {
    Cut,
    Yank,
}

#[derive(Clone, Copy, Debug)]
struct TodoClick {
    id: TodoId,
    at: Instant,
}

struct App {
    exit: bool,
    repository: RepositoryContext,
    store: TodoStore,
    persistence_available: bool,
    todos: Vec<Todo>,
    focus: Option<Focus>,
    mode: Mode,
    todo_register: Option<Todo>,
    pending_operator: Option<PendingOperator>,
    theme: Theme,
    system_theme: Option<SystemThemeState>,
    data_version: i64,
    dispatch: DispatchController,
    error: Option<String>,
    repository_error: Option<String>,
    last_repository_refresh: Instant,
    clipboard_request: Option<String>,
    external_edit_request: Option<TodoId>,
    pointer_position: Option<Position>,
    last_todo_click: Option<TodoClick>,
    frame_area: Rect,
    viewport_start: usize,
    reveal_focus: bool,
}

impl Default for App {
    fn default() -> Self {
        Self::new(TOKYO_NIGHT_DAY, DispatchController::default())
    }
}

impl App {
    fn new(theme: Theme, dispatch: DispatchController) -> Self {
        Self::new_with_repository_discovery(theme, dispatch, || {
            RepositoryContext::discover(".").map_err(|error| error.to_string())
        })
    }

    fn new_with_repository_discovery(
        theme: Theme,
        dispatch: DispatchController,
        discover: impl FnOnce() -> Result<RepositoryContext, String>,
    ) -> Self {
        let (mut repository, store, repository_error, mut error, persistence_available) =
            match discover() {
                Ok(repository) => {
                    if repository.common_git_dir.as_os_str().is_empty() {
                        (
                            repository,
                            TodoStore::open_in_memory().expect("in-memory todo database must open"),
                            None,
                            Some("not inside a Git repository; todos are unavailable".to_owned()),
                            false,
                        )
                    } else {
                        let database_path = repository.common_git_dir.join("refdo").join("data.db");
                        match TodoStore::open(&database_path) {
                            Ok(store) => (repository, store, None, None, true),
                            Err(open_error) => (
                                repository,
                                TodoStore::open_in_memory()
                                    .expect("in-memory todo database must open"),
                                None,
                                Some(open_error.to_string()),
                                false,
                            ),
                        }
                    }
                }
                Err(discovery_error) => (
                    RepositoryContext::default(),
                    TodoStore::open_in_memory().expect("in-memory todo database must open"),
                    Some(format!("repository: {discovery_error}")),
                    None,
                    false,
                ),
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
        repository.reconcile_stored_branches(todos.iter().map(|todo| todo.branch_ref.as_str()));

        Self {
            exit: false,
            repository,
            store,
            persistence_available,
            todos,
            focus: None,
            mode: Mode::Normal,
            todo_register: None,
            pending_operator: None,
            theme,
            system_theme: None,
            data_version,
            dispatch,
            error,
            repository_error,
            last_repository_refresh: Instant::now(),
            clipboard_request: None,
            external_edit_request: None,
            pointer_position: None,
            last_todo_click: None,
            frame_area: Rect::default(),
            viewport_start: 0,
            reveal_focus: true,
        }
    }

    fn new_system(light_theme: Theme, dark_theme: Theme, dispatch: DispatchController) -> Self {
        Self::new_system_with_detector(light_theme, dark_theme, dispatch, Instant::now(), || {
            dark_light::detect().ok()
        })
    }

    fn new_system_with_detector(
        light_theme: Theme,
        dark_theme: Theme,
        dispatch: DispatchController,
        now: Instant,
        detect: impl FnOnce() -> Option<dark_light::Mode>,
    ) -> Self {
        let theme = match detect() {
            Some(dark_light::Mode::Dark) => dark_theme,
            Some(dark_light::Mode::Light | dark_light::Mode::Unspecified) | None => light_theme,
        };
        let mut app = Self::new(theme, dispatch);
        app.system_theme = Some(SystemThemeState {
            light_theme,
            dark_theme,
            last_checked: now,
        });
        app
    }

    fn refresh_system_theme(&mut self) {
        self.refresh_system_theme_with(Instant::now(), || dark_light::detect().ok());
    }

    fn refresh_system_theme_with(
        &mut self,
        now: Instant,
        detect: impl FnOnce() -> Option<dark_light::Mode>,
    ) {
        let Some(system_theme) = &mut self.system_theme else {
            return;
        };
        if now
            .checked_duration_since(system_theme.last_checked)
            .is_none_or(|elapsed| elapsed < SYSTEM_THEME_REFRESH_INTERVAL)
        {
            return;
        }
        system_theme.last_checked = now;
        self.theme = match detect() {
            Some(dark_light::Mode::Light) => system_theme.light_theme,
            Some(dark_light::Mode::Dark) => system_theme.dark_theme,
            Some(dark_light::Mode::Unspecified) | None => self.theme,
        };
    }

    fn flush_clipboard_request(&mut self, writer: &mut impl io::Write) {
        let Some(text) = self.clipboard_request.take() else {
            return;
        };
        self.error = Some(
            match execute!(writer, CopyToClipboard::to_clipboard_from(text)) {
                Ok(()) => "Copied todo text".to_owned(),
                Err(error) => format!("copy: {error}"),
            },
        );
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        if let Err(error) = execute!(terminal.backend_mut(), EnableMouseCapture) {
            let _ = execute!(terminal.backend_mut(), DisableMouseCapture);
            return Err(error);
        }

        let result = (|| {
            while !self.exit {
                if let Some(id) = self.external_edit_request.take() {
                    self.edit_todo_externally(terminal, id)?;
                }
                terminal.draw(|frame| self.draw(frame))?;
                self.handle_events()?;
                self.flush_clipboard_request(terminal.backend_mut());
            }
            Ok(())
        })();
        let disable_result = execute!(terminal.backend_mut(), DisableMouseCapture);
        result.and(disable_result)
    }

    fn edit_todo_externally(
        &mut self,
        terminal: &mut DefaultTerminal,
        id: TodoId,
    ) -> io::Result<()> {
        let Some(todo) = self.todos.iter().find(|todo| todo.id == id) else {
            self.error = Some(format!("todo {id} was not found"));
            return Ok(());
        };
        let prepared = match external_editor::prepare(todo) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.error = Some(error);
                return Ok(());
            }
        };

        terminal.show_cursor()?;
        execute!(terminal.backend_mut(), DisableMouseCapture)?;
        ratatui::try_restore()?;

        let launch_result = prepared.launch();
        let resume_result = (|| {
            enable_raw_mode()?;
            execute!(
                terminal.backend_mut(),
                EnterAlternateScreen,
                EnableMouseCapture
            )?;
            terminal.hide_cursor()?;
            terminal.autoresize()?;
            clear_for_full_redraw(terminal)?;
            Ok::<(), io::Error>(())
        })();
        if let Err(error) = resume_result {
            let message = prepared.preserve(&format!("terminal restoration failed: {error}"));
            return Err(io::Error::other(message));
        }

        let status = match launch_result {
            Ok(status) => status,
            Err(error) => {
                self.error = Some(prepared.preserve(&format!("could not launch editor: {error}")));
                return Ok(());
            }
        };
        let edited = match prepared.finish(status) {
            Ok(edited) => edited,
            Err(error) => {
                self.error = Some(error);
                return Ok(());
            }
        };
        self.persist_external_edit(id, edited);
        Ok(())
    }

    fn persist_external_edit(&mut self, id: TodoId, edited: external_editor::EditedTodo) {
        match self.store.update_todo(id, &edited.title, &edited.body) {
            Ok(todo) => {
                if let Some(existing) = self.todos.iter_mut().find(|todo| todo.id == id) {
                    *existing = todo;
                }
                self.focus = Some(Focus::Todo(id));
                self.error = None;
                self.reveal_focus = true;
            }
            Err(error) => self.error = Some(edited.preserve(&error.to_string())),
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        self.frame_area = frame.area();
        let [status_area, content_area, footer_area] = ui::app_areas(self.frame_area);
        let todo_area = ui::todo_viewport_area(self.frame_area);
        let editor = self.mode.editor();
        let rows = ui::build_display_layout(
            &self.repository.sections,
            &self.todos,
            editor,
            todo_area.width,
        );
        self.viewport_start = ui::reconcile_viewport_start(
            &rows,
            todo_area.height,
            self.viewport_start,
            self.reveal_focus,
            self.focus.as_ref(),
            editor,
        );
        self.reveal_focus = false;
        let hovered = self.pointer_position.and_then(|position| {
            ui::hit_test_display_rows(&rows, todo_area, self.viewport_start, position)
        });

        ui::render_status_bar(frame, status_area, &self.repository.head_label, &self.theme);
        let content_block = Block::bordered()
            .style(Style::default().bg(self.theme.background))
            .border_style(Style::default().fg(self.theme.mode_background))
            .padding(Padding::horizontal(1));
        frame.render_widget(content_block, content_area);
        let select_state = match &self.mode {
            Mode::Select(state) => Some(state),
            _ => None,
        };
        ui::render_branch_sections(
            frame,
            todo_area,
            &rows,
            self.focus.as_ref(),
            hovered.as_ref(),
            self.mode.editor(),
            select_state,
            self.viewport_start,
            &self.theme,
        );
        ui::render_footer(
            frame,
            footer_area,
            &self.mode,
            self.mode
                .footer_message()
                .or(self.repository_error.as_deref())
                .or(self.error.as_deref()),
            &self.theme,
        );
        let (mode, todos) = (&mut self.mode, &self.todos);
        if let Mode::Preview(preview) = mode
            && let Some(todo) = todos.iter().find(|todo| todo.id == preview.todo_id)
        {
            ui::render_body_preview(frame, preview, todo, &self.theme);
        }
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
                self.repository.reconcile_stored_branches(
                    self.todos.iter().map(|todo| todo.branch_ref.as_str()),
                );
                self.data_version = version;
                self.repair_select_mode();
                self.repair_preview_mode();
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
                self.repository.reconcile_stored_branches(
                    self.todos.iter().map(|todo| todo.branch_ref.as_str()),
                );
                self.data_version = version;
                self.repair_preview_mode();
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
        self.todos.push(todo);
        self.todos.sort_by(|left, right| {
            left.branch_ref
                .cmp(&right.branch_ref)
                .then_with(|| left.sort_order.cmp(&right.sort_order))
                .then_with(|| left.id.cmp(&right.id))
        });
        self.repository
            .reconcile_stored_branches(self.todos.iter().map(|todo| todo.branch_ref.as_str()));
    }

    fn refresh_repository(&mut self) {
        self.refresh_repository_with(Instant::now(), || {
            RepositoryContext::discover(".").map_err(|error| error.to_string())
        });
    }

    fn refresh_repository_with(
        &mut self,
        now: Instant,
        discover: impl FnOnce() -> Result<RepositoryContext, String>,
    ) {
        if self.repository.common_git_dir.as_os_str().is_empty() {
            return;
        }
        if now
            .checked_duration_since(self.last_repository_refresh)
            .is_none_or(|elapsed| elapsed < REPOSITORY_REFRESH_INTERVAL)
        {
            return;
        }
        self.last_repository_refresh = now;
        match discover() {
            Ok(mut discovered) => {
                if discovered.common_git_dir != self.repository.common_git_dir {
                    self.repository_error = Some(
                        "repository: discovered repository has a different common Git directory"
                            .to_owned(),
                    );
                    return;
                }
                discovered.reconcile_stored_branches(
                    self.todos.iter().map(|todo| todo.branch_ref.as_str()),
                );
                self.repository = discovered;
                self.repository_error = None;
                self.repair_select_mode();
                self.repair_preview_mode();
                self.repair_focus();
            }
            Err(error) => {
                self.repository_error = Some(format!("repository: {error}"));
            }
        }
    }
}

// `Terminal::clear` snapshots the cursor first. The editor handoff cannot rely on a cursor-position
// response, so clear the display directly and reset Ratatui's comparison buffer for the next draw.
fn clear_for_full_redraw<B: Backend>(terminal: &mut Terminal<B>) -> Result<(), B::Error> {
    terminal.backend_mut().clear()?;
    terminal.swap_buffers();
    Ok(())
}

#[cfg(test)]
mod tests;

pub(crate) fn run(
    light_theme: Theme,
    dark_theme: Theme,
    mode: ThemeMode,
    dispatch_settings: DispatchSettings,
) -> std::io::Result<()> {
    ratatui::run(|terminal| {
        let dispatch = DispatchController::new(dispatch_settings);
        let mut app = match mode {
            ThemeMode::Light => App::new(light_theme, dispatch),
            ThemeMode::Dark => App::new(dark_theme, dispatch),
            ThemeMode::System => App::new_system(light_theme, dark_theme, dispatch),
        };
        app.run(terminal)
    })
}

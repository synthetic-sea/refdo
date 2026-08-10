mod actions;
mod commands;
mod events;
mod navigation;
mod text_input;
mod ui;

use std::{
    io,
    time::{Duration, Instant},
};

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::{
        event::{DisableMouseCapture, EnableMouseCapture},
        execute,
    },
    layout::{Position, Rect},
    style::Style,
    widgets::{Block, Padding},
};

use crate::config::ThemeMode;
use crate::repository::RepositoryContext;
use crate::storage::{Todo, TodoId, TodoStore};
use crate::theme::{TOKYO_NIGHT_DAY, Theme};

const SYSTEM_THEME_REFRESH_INTERVAL: Duration = Duration::from_millis(500);
const UNKNOWN_DATA_VERSION: i64 = -1;

#[derive(Clone, Debug)]
enum Mode {
    Normal,
    Insert(Editor),
    Command(CommandLine),
    ConfirmClear(ClearConfirmation),
}

impl Mode {
    const fn label(&self) -> &'static str {
        match self {
            Self::Normal => " NORMAL ",
            Self::Insert(_) => " INSERT ",
            Self::Command(_) => " COMMAND ",
            Self::ConfirmClear(_) => " CONFIRM ",
        }
    }

    const fn editor(&self) -> Option<&Editor> {
        match self {
            Self::Insert(editor) => Some(editor),
            Self::Normal | Self::Command(_) | Self::ConfirmClear(_) => None,
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

#[derive(Clone, Debug)]
struct CommandLine {
    target_branch: Option<String>,
    text: String,
    cursor: usize,
}

#[derive(Clone, Debug)]
struct ClearConfirmation {
    target_branch: String,
}

struct SystemThemeState {
    light_theme: Theme,
    dark_theme: Theme,
    last_checked: Instant,
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
    system_theme: Option<SystemThemeState>,
    data_version: i64,
    error: Option<String>,
    pointer_position: Option<Position>,
    frame_area: Rect,
    viewport_start: usize,
    reveal_focus: bool,
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
                let database_path = repository.common_git_dir.join("refdo").join("data.db");
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
            system_theme: None,
            data_version,
            error,
            pointer_position: None,
            frame_area: Rect::default(),
            viewport_start: 0,
            reveal_focus: true,
        }
    }

    fn new_system(light_theme: Theme, dark_theme: Theme) -> Self {
        Self::new_system_with_detector(light_theme, dark_theme, Instant::now(), || {
            dark_light::detect().ok()
        })
    }

    fn new_system_with_detector(
        light_theme: Theme,
        dark_theme: Theme,
        now: Instant,
        detect: impl FnOnce() -> Option<dark_light::Mode>,
    ) -> Self {
        let theme = match detect() {
            Some(dark_light::Mode::Dark) => dark_theme,
            Some(dark_light::Mode::Light | dark_light::Mode::Unspecified) | None => light_theme,
        };
        let mut app = Self::new(theme);
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

    fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
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
        ui::render_branch_sections(
            frame,
            todo_area,
            &rows,
            self.focus.as_ref(),
            hovered.as_ref(),
            self.mode.editor(),
            self.viewport_start,
            &self.theme,
        );
        ui::render_footer(
            frame,
            footer_area,
            &self.mode,
            self.error.as_deref(),
            &self.theme,
        );
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
}

#[cfg(test)]
mod tests;

pub(crate) fn run(light_theme: Theme, dark_theme: Theme, mode: ThemeMode) -> std::io::Result<()> {
    ratatui::run(|terminal| {
        let mut app = match mode {
            ThemeMode::Light => App::new(light_theme),
            ThemeMode::Dark => App::new(dark_theme),
            ThemeMode::System => App::new_system(light_theme, dark_theme),
        };
        app.run(terminal)
    })
}

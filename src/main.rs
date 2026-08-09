mod repository;
mod theme;

use std::io;

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};

use repository::{BranchSection, RepositoryContext};
use theme::{TOKYO_NIGHT_DAY, Theme};

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

#[derive(Debug)]
struct App {
    exit: bool,
    repository: RepositoryContext,
    selected_section: Option<usize>,
    theme: Theme,
    input_mode: InputMode,
}

impl Default for App {
    fn default() -> Self {
        Self::new(TOKYO_NIGHT_DAY)
    }
}

impl App {
    fn new(theme: Theme) -> Self {
        let repository = RepositoryContext::discover(".").unwrap_or_default();
        let selected_section = (!repository.sections.is_empty()).then_some(0);

        Self {
            exit: false,
            repository,
            selected_section,
            theme,
            input_mode: InputMode::Normal,
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
        render_branch_sections(
            frame,
            content_area,
            &self.repository.sections,
            self.selected_section,
            &self.theme,
        );
        render_footer(frame, footer_area, self.input_mode, &self.theme);
    }

    fn handle_events(&mut self) -> io::Result<()> {
        if let Event::Key(key) = event::read()? {
            self.handle_key_event(key);
        }

        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }

        match self.input_mode {
            InputMode::Normal => match key.code {
                KeyCode::Char('j') => {
                    if let (Some(selected), Some(last)) = (
                        self.selected_section,
                        self.repository.sections.len().checked_sub(1),
                    ) {
                        self.selected_section = Some((selected + 1).min(last));
                    }
                }
                KeyCode::Char('k') => {
                    if let Some(selected) = self.selected_section {
                        self.selected_section = Some(selected.saturating_sub(1));
                    }
                }
                KeyCode::Char('o') if self.selected_section.is_some() => {
                    self.input_mode = InputMode::Insert;
                }
                KeyCode::Char('q') => self.exit = true,
                _ => {}
            },
            InputMode::Insert => {
                if key.code == KeyCode::Esc {
                    self.input_mode = InputMode::Normal;
                }
            }
        }
    }
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

fn render_footer(frame: &mut Frame, area: Rect, input_mode: InputMode, theme: &Theme) {
    let mode = Span::styled(
        input_mode.label(),
        Style::default()
            .fg(theme.mode_foreground)
            .bg(theme.mode_background)
            .add_modifier(Modifier::BOLD),
    );

    frame.render_widget(
        Paragraph::new(Line::from(mode)).style(
            Style::default()
                .fg(theme.foreground)
                .bg(theme.status_bar_background),
        ),
        area,
    );
}

fn render_branch_sections(
    frame: &mut Frame,
    area: Rect,
    sections: &[BranchSection],
    selected_section: Option<usize>,
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

    let visible_sections = (usize::from(area.height) + 1) / 2;
    if visible_sections == 0 {
        return;
    }
    let first_visible_section = selected_section
        .filter(|&selected| selected < sections.len())
        .map(|selected| selected.saturating_add(1).saturating_sub(visible_sections))
        .unwrap_or(0);

    for (visible_index, (index, section)) in sections
        .iter()
        .enumerate()
        .skip(first_visible_section)
        .take(visible_sections)
        .enumerate()
    {
        let row_offset = u16::try_from(visible_index)
            .unwrap_or(u16::MAX)
            .saturating_mul(2);
        if row_offset >= area.height {
            break;
        }

        let header_area = Rect::new(area.x, area.y + row_offset, area.width, 1);
        let is_selected = selected_section == Some(index);
        let header_background = if is_selected {
            theme.selection_background
        } else {
            theme.background
        };
        frame.render_widget(
            Block::default().style(Style::default().bg(header_background)),
            header_area,
        );

        let tag = match (section.is_current, section.is_locked) {
            (true, true) => " CURRENT · LOCKED ",
            (true, false) => " CURRENT ",
            (false, true) => " WORKTREE · LOCKED ",
            (false, false) => " WORKTREE ",
        };
        let tag_width = tag.chars().count();
        let branch_width = 2usize.saturating_add(section.display_name.chars().count());
        let tag_width = if branch_width.saturating_add(tag_width) <= usize::from(header_area.width)
        {
            u16::try_from(tag_width).unwrap_or(0)
        } else {
            0
        };
        let [branch_area, tag_area] =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(tag_width)])
                .areas(header_area);
        let header_style = Style::default()
            .fg(theme.foreground)
            .bg(header_background)
            .add_modifier(Modifier::BOLD);

        frame.render_widget(
            Paragraph::new(format!("▾ {}", section.display_name)).style(header_style),
            branch_area,
        );
        if tag_width > 0 {
            frame.render_widget(
                Paragraph::new(tag)
                    .style(
                        Style::default()
                            .fg(theme.foreground_muted)
                            .bg(header_background),
                    )
                    .right_aligned(),
                tag_area,
            );
        }

        if row_offset + 1 < area.height {
            let empty_area = Rect::new(area.x, header_area.y + 1, area.width, 1);
            frame.render_widget(
                Paragraph::new("    No todos").style(
                    Style::default()
                        .fg(theme.foreground_muted)
                        .bg(theme.background),
                ),
                empty_area,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend, crossterm::event::KeyModifiers, style::Color};

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

    fn section(name: &str, is_current: bool, is_locked: bool) -> BranchSection {
        BranchSection {
            full_ref_name: format!("refs/heads/{name}"),
            display_name: name.to_owned(),
            worktree_path: std::path::PathBuf::from(format!("/worktrees/{name}")),
            is_current,
            is_locked,
        }
    }

    fn app_with_sections(theme: Theme, head_label: &str, sections: Vec<BranchSection>) -> App {
        let selected_section = (!sections.is_empty()).then_some(0);
        App {
            exit: false,
            repository: RepositoryContext {
                head_label: head_label.to_owned(),
                sections,
            },
            selected_section,
            theme,
            input_mode: InputMode::Normal,
        }
    }

    fn row_text(terminal: &Terminal<TestBackend>, row: u16) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.width)
            .map(|column| buffer[(column, row)].symbol())
            .collect()
    }

    #[test]
    fn application_renders_themed_branch_section_and_empty_state() {
        let theme = test_theme();
        let app = app_with_sections(
            theme,
            "feature/auth",
            vec![section("feature/auth", true, false)],
        );
        let backend = TestBackend::new(30, 4);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| app.draw(frame)).unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(row_text(&terminal, 0), " tuido       git:feature/auth ");
        assert!(row_text(&terminal, 1).contains("▾ feature/auth"));
        assert!(row_text(&terminal, 1).contains("CURRENT"));
        assert!(row_text(&terminal, 2).starts_with("    No todos"));
        assert_eq!(row_text(&terminal, 3), format!("{:<30}", " NORMAL "));
        assert!((0..30).all(|column| buffer[(column, 0)].bg == theme.status_bar_background));
        assert!((0..30).all(|column| buffer[(column, 1)].bg == theme.selection_background));
        assert!((0..30).all(|column| buffer[(column, 2)].bg == theme.background));
        assert!((8..30).all(|column| buffer[(column, 3)].bg == theme.status_bar_background));
        assert!((0..8).all(|column| buffer[(column, 3)].bg == theme.mode_background));
        assert_eq!(buffer[(1, 0)].fg, theme.foreground);
        assert_eq!(buffer[(13, 0)].fg, theme.foreground_muted);
        assert_eq!(buffer[(1, 3)].fg, theme.mode_foreground);
        assert!(buffer[(1, 0)].modifier.contains(Modifier::BOLD));
        assert!(buffer[(1, 3)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn normal_and_insert_modes_transition_without_quitting() {
        let mut app = app_with_sections(
            TOKYO_NIGHT_DAY,
            "feature/auth",
            vec![section("feature/auth", true, false)],
        );

        app.handle_key_event(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
        assert_eq!(app.input_mode, InputMode::Insert);

        app.handle_key_event(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(!app.exit);

        let backend = TestBackend::new(20, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert_eq!(row_text(&terminal, 3), format!("{:<20}", " INSERT "));

        app.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(!app.exit);

        app.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.exit);

        app.handle_key_event(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(app.exit);
    }

    #[test]
    fn normal_mode_moves_focus_between_branch_sections_without_wrapping() {
        let mut app = app_with_sections(
            TOKYO_NIGHT_DAY,
            "feature/auth",
            vec![
                section("feature/auth", true, false),
                section("main", false, false),
                section("release/1.0", false, true),
            ],
        );

        app.handle_key_event(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(app.selected_section, Some(1));
        app.handle_key_event(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        app.handle_key_event(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(app.selected_section, Some(2));
        app.handle_key_event(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
        assert_eq!(app.selected_section, Some(1));
    }

    #[test]
    fn selected_branch_remains_visible_in_a_short_narrow_viewport() {
        let theme = test_theme();
        let mut app = app_with_sections(
            theme,
            "feature/auth",
            vec![
                section("feature/auth", true, false),
                section("main", false, false),
                section("release/1.0", false, true),
            ],
        );
        app.selected_section = Some(2);
        let backend = TestBackend::new(19, 4);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| app.draw(frame)).unwrap();

        let branch_row = row_text(&terminal, 1);
        assert!(branch_row.contains("▾ release/1.0"));
        assert!(!branch_row.contains("WORKTREE"));
        assert!((0..19).all(
            |column| terminal.backend().buffer()[(column, 1)].bg == theme.selection_background
        ));
    }

    #[test]
    fn detached_context_without_branch_sections_stays_in_normal_mode() {
        let mut app = app_with_sections(TOKYO_NIGHT_DAY, "detached@abcdef0", Vec::new());
        let backend = TestBackend::new(40, 4);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(row_text(&terminal, 1).contains("No registered worktree branches"));

        app.handle_key_event(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
        assert_eq!(app.input_mode, InputMode::Normal);
    }
}

fn main() -> std::io::Result<()> {
    ratatui::run(|terminal| App::default().run(terminal))
}

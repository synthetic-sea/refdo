mod theme;

use std::io;

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};

use theme::{TOKYO_NIGHT_DAY, Theme};

#[derive(Debug)]
struct App {
    exit: bool,
    branch: String,
    theme: Theme,
}

impl Default for App {
    fn default() -> Self {
        Self::new(TOKYO_NIGHT_DAY)
    }
}

impl App {
    fn new(theme: Theme) -> Self {
        Self {
            exit: false,
            branch: current_branch(),
            theme,
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

        render_status_bar(frame, status_area, &self.branch, &self.theme);
        frame.render_widget(
            Block::default().style(Style::default().bg(self.theme.background)),
            content_area,
        );
        render_footer(frame, footer_area, &self.theme);
    }

    fn handle_events(&mut self) -> io::Result<()> {
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
        {
            self.exit = true;
        }

        Ok(())
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

fn render_footer(frame: &mut Frame, area: Rect, theme: &Theme) {
    let mode = Span::styled(
        " NORMAL ",
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

fn current_branch() -> String {
    let Ok(repository) = gix::discover(".") else {
        return "unknown".to_owned();
    };
    let Ok(head) = repository.head() else {
        return "unknown".to_owned();
    };

    if let Some(branch) = head.referent_name() {
        return branch.shorten().to_string();
    }

    head.id()
        .map(|id| {
            let mut commit = id.to_string();
            commit.truncate(7);
            format!("detached@{commit}")
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend, style::Color};

    use super::*;

    #[test]
    fn application_uses_the_supplied_theme_for_header_content_and_footer() {
        let theme = Theme {
            background: Color::Red,
            foreground: Color::Green,
            foreground_muted: Color::Blue,
            status_bar_background: Color::Yellow,
            mode_background: Color::Magenta,
            mode_foreground: Color::Cyan,
        };
        let app = App {
            exit: false,
            branch: "feature/auth".to_owned(),
            theme,
        };
        let backend = TestBackend::new(30, 3);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| app.draw(frame)).unwrap();

        let buffer = terminal.backend().buffer();
        let header = (0..30)
            .map(|column| buffer[(column, 0)].symbol())
            .collect::<String>();
        let footer = (0..30)
            .map(|column| buffer[(column, 2)].symbol())
            .collect::<String>();

        assert_eq!(header, " tuido       git:feature/auth ");
        assert_eq!(footer, format!("{:<30}", " NORMAL "));
        assert!((0..30).all(|column| buffer[(column, 0)].bg == theme.status_bar_background));
        assert!((0..30).all(|column| buffer[(column, 1)].bg == theme.background));
        assert!((8..30).all(|column| buffer[(column, 2)].bg == theme.status_bar_background));
        assert!((0..8).all(|column| buffer[(column, 2)].bg == theme.mode_background));
        assert_eq!(buffer[(1, 0)].fg, theme.foreground);
        assert_eq!(buffer[(13, 0)].fg, theme.foreground_muted);
        assert_eq!(buffer[(1, 2)].fg, theme.mode_foreground);
        assert!(buffer[(1, 0)].modifier.contains(Modifier::BOLD));
        assert!(buffer[(1, 2)].modifier.contains(Modifier::BOLD));
    }
}

fn main() -> std::io::Result<()> {
    ratatui::run(|terminal| App::default().run(terminal))
}

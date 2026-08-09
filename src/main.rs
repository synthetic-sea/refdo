mod theme;

use std::io;

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
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
        let [status_area, content_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(frame.area());

        render_status_bar(frame, status_area, &self.branch, &self.theme);
        frame.render_widget(
            Block::default().style(Style::default().bg(self.theme.background)),
            content_area,
        );
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
        Block::default().style(Style::default().bg(theme.background)),
        area,
    );

    let [brand_area, context_area] =
        Layout::horizontal([Constraint::Length(7), Constraint::Min(0)]).areas(area);

    frame.render_widget(
        Paragraph::new(" tuido")
            .style(
                Style::default()
                    .fg(theme.foreground)
                    .bg(theme.background)
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
                    .bg(theme.background),
            )
            .right_aligned(),
        context_area,
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
    fn application_uses_the_supplied_theme_for_every_rendered_cell() {
        let theme = Theme::new(Color::Red, Color::Green, Color::Blue);
        let app = App {
            exit: false,
            branch: "feature/auth".to_owned(),
            theme,
        };
        let backend = TestBackend::new(30, 2);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| app.draw(frame)).unwrap();

        let buffer = terminal.backend().buffer();
        let row = (0..30)
            .map(|column| buffer[(column, 0)].symbol())
            .collect::<String>();

        assert_eq!(row, " tuido       git:feature/auth ");
        assert!(
            (0..2).all(|row| (0..30).all(|column| buffer[(column, row)].bg == theme.background))
        );
        assert_eq!(buffer[(1, 0)].fg, theme.foreground);
        assert_eq!(buffer[(13, 0)].fg, theme.foreground_muted);
        assert!(buffer[(1, 0)].modifier.contains(Modifier::BOLD));
    }
}

fn main() -> std::io::Result<()> {
    ratatui::run(|terminal| App::default().run(terminal))
}

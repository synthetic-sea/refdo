use std::io;

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Paragraph},
};

const BACKGROUND: Color = Color::Rgb(26, 27, 38);
const FOREGROUND: Color = Color::Rgb(192, 202, 245);
const FOREGROUND_DIM: Color = Color::Rgb(86, 95, 137);

#[derive(Debug)]
struct App {
    exit: bool,
    branch: String,
}

impl Default for App {
    fn default() -> Self {
        Self {
            exit: false,
            branch: current_branch(),
        }
    }
}

impl App {
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

        render_status_bar(frame, status_area, &self.branch);
        frame.render_widget(
            Block::default().style(Style::default().bg(BACKGROUND)),
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

fn render_status_bar(frame: &mut Frame, area: Rect, branch: &str) {
    frame.render_widget(
        Block::default().style(Style::default().bg(BACKGROUND)),
        area,
    );

    let [brand_area, context_area] =
        Layout::horizontal([Constraint::Length(7), Constraint::Min(0)]).areas(area);

    frame.render_widget(
        Paragraph::new(" tuido")
            .style(
                Style::default()
                    .fg(FOREGROUND)
                    .bg(BACKGROUND)
                    .add_modifier(Modifier::BOLD),
            )
            .left_aligned(),
        brand_area,
    );
    frame.render_widget(
        Paragraph::new(format!("git:{branch} "))
            .style(Style::default().fg(FOREGROUND_DIM).bg(BACKGROUND))
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
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    #[test]
    fn status_bar_spans_the_row_and_keeps_context_right_aligned() {
        let backend = TestBackend::new(30, 2);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                render_status_bar(frame, Rect::new(0, 0, 30, 1), "feature/auth");
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let row = (0..30)
            .map(|column| buffer[(column, 0)].symbol())
            .collect::<String>();

        assert_eq!(row, " tuido       git:feature/auth ");
        assert!((0..30).all(|column| buffer[(column, 0)].bg == BACKGROUND));
        assert!(buffer[(1, 0)].modifier.contains(Modifier::BOLD));
    }
}

fn main() -> std::io::Result<()> {
    ratatui::run(|terminal| App::default().run(terminal))
}

mod actions;
mod commands;
mod navigation;
mod refresh;
mod support;
mod theme;
mod ui;

use super::{
    App, BodyPreview, ClearConfirmation, CommandLine, DispatchController, Editor, EditorTarget,
    Focus, Mode, PendingOperator, UNKNOWN_DATA_VERSION, clear_for_full_redraw,
};
use crate::config::{DispatchConfigDigest, DispatchSettings, load_repository_dispatch_config};
use crate::repository::{BranchSection, RepositoryContext};
use crate::storage::{TodoId, TodoStore};
use crate::theme::Theme;
use ratatui::{
    Terminal,
    backend::{Backend, ClearType, TestBackend, WindowSize},
    buffer::Cell,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    layout::{Position, Rect, Size},
    style::Color,
};
use std::{convert::Infallible, io};

struct CursorQueryFailingBackend {
    inner: TestBackend,
    cursor_queries: usize,
    clears: usize,
}

impl CursorQueryFailingBackend {
    fn new(width: u16, height: u16) -> Self {
        Self {
            inner: TestBackend::new(width, height),
            cursor_queries: 0,
            clears: 0,
        }
    }
}

fn infallible<T>(result: Result<T, Infallible>) -> io::Result<T> {
    result.map_err(|error| match error {})
}

impl Backend for CursorQueryFailingBackend {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        infallible(self.inner.draw(content))
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        infallible(self.inner.hide_cursor())
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        infallible(self.inner.show_cursor())
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        self.cursor_queries += 1;
        Err(io::Error::other("cursor position unavailable"))
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        infallible(self.inner.set_cursor_position(position))
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.clears += 1;
        infallible(self.inner.clear())
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        infallible(self.inner.clear_region(clear_type))
    }

    fn size(&self) -> Result<Size, Self::Error> {
        infallible(self.inner.size())
    }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        infallible(self.inner.window_size())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        infallible(self.inner.flush())
    }
}

#[test]
fn restoration_clear_avoids_cursor_query_and_forces_full_redraw() {
    let backend = CursorQueryFailingBackend::new(12, 2);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| frame.render_widget("restored", frame.area()))
        .unwrap();

    clear_for_full_redraw(&mut terminal).unwrap();

    let backend = terminal.backend();
    assert_eq!(backend.cursor_queries, 0);
    assert_eq!(backend.clears, 1);
    assert!(
        backend
            .inner
            .buffer()
            .content
            .iter()
            .all(|cell| cell.symbol() == " ")
    );
    terminal
        .draw(|frame| frame.render_widget("restored", frame.area()))
        .unwrap();
    let backend = terminal.backend();
    let first_row = (0..backend.inner.buffer().area.width)
        .map(|x| backend.inner.buffer()[(x, 0)].symbol())
        .collect::<String>();
    assert!(first_row.starts_with("restored"));
}

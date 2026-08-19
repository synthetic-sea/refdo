use std::ops::Range;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use super::super::{Editor, Focus, Mode};
use super::layout::{DisplayRow, DisplayRowLayout, maximum_viewport_start};
use crate::repository::BranchSection;
use crate::theme::Theme;

const INCOMPLETE_TODO_MARKER: &str = "󰄱";
const COMPLETE_TODO_MARKER: &str = "󰄲";
const TODO_PREFIX_WIDTH: u16 = 5;
pub(in crate::app) fn render_status_bar(
    frame: &mut Frame,
    area: Rect,
    branch: &str,
    theme: &Theme,
) {
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.status_bar_background)),
        area,
    );
    let [brand_area, context_area] =
        Layout::horizontal([Constraint::Length(7), Constraint::Min(0)]).areas(area);
    frame.render_widget(
        Paragraph::new(" refdo")
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

pub(in crate::app) fn render_footer(
    frame: &mut Frame,
    area: Rect,
    mode: &Mode,
    error: Option<&str>,
    theme: &Theme,
) {
    let footer_style = Style::default()
        .fg(theme.foreground)
        .bg(theme.status_bar_background);
    if let Mode::Command(command) = mode {
        frame.render_widget(
            Paragraph::new(format!(":{}", command.text)).style(footer_style),
            area,
        );
        if area.width > 0 && area.height > 0 {
            let cursor_offset = 1 + command.text[..command.cursor].width();
            let cursor_x = area
                .x
                .saturating_add(u16::try_from(cursor_offset).unwrap_or(u16::MAX))
                .min(area.right().saturating_sub(1));
            frame.set_cursor_position(Position::new(cursor_x, area.y));
        }
        return;
    }

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
        Paragraph::new(Line::from(vec![mode, error])).style(footer_style),
        area,
    );
}
#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn render_branch_sections(
    frame: &mut Frame,
    area: Rect,
    rows: &[DisplayRowLayout<'_>],
    focus: Option<&Focus>,
    hovered: Option<&Focus>,
    editor: Option<&Editor>,
    viewport_start: usize,
    theme: &Theme,
) {
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.background)),
        area,
    );
    if rows.is_empty() {
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

    let first = viewport_start.min(maximum_viewport_start(rows, area.height));
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
                let marker = if todo.completed {
                    COMPLETE_TODO_MARKER
                } else {
                    INCOMPLETE_TODO_MARKER
                };
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
                    Paragraph::new(format!("   {marker} ")).style(style),
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
                    Paragraph::new("   No todos").style(
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
            (true, true) => " CURRENT · LOCKED",
            (true, false) => " CURRENT",
            (false, true) => " WORKTREE · LOCKED",
            (false, false) => " WORKTREE",
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
        Paragraph::new(format!("󰍝 {}", section.display_name)).style(style),
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
    let marker = if completed {
        COMPLETE_TODO_MARKER
    } else {
        INCOMPLETE_TODO_MARKER
    };
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
        Paragraph::new(format!("   {marker} ")).style(style),
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

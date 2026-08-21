use std::ops::Range;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::super::{BodyPreview, Editor, Focus, Mode, SelectState};
use super::layout::{DisplayRow, DisplayRowLayout, maximum_viewport_start};
use crate::repository::BranchSection;
use crate::storage::Todo;
use crate::theme::Theme;

const INCOMPLETE_TODO_MARKER: &str = "󰄱";
const COMPLETE_TODO_MARKER: &str = "󰄲";
const SELECTED_TODO_MARKER: &str = "●";
const UNSELECTED_TODO_MARKER: &str = "○";
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
    message: Option<&str>,
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

    let mode_span = Span::styled(
        mode.label(),
        Style::default()
            .fg(theme.mode_foreground)
            .bg(theme.mode_background)
            .add_modifier(Modifier::BOLD),
    );
    let mut spans = vec![mode_span];
    if let Mode::Select(select_state) = mode {
        let count = select_state.selected_todo_ids.len();
        spans.push(Span::styled(
            format!("· {count} selected"),
            Style::default().fg(theme.foreground),
        ));
    }
    if let Some(message) = message {
        spans.push(Span::styled(
            format!(" {message}"),
            Style::default().fg(theme.foreground),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(footer_style), area);
}

pub(in crate::app) fn render_body_preview(
    frame: &mut Frame,
    preview: &mut BodyPreview,
    todo: &Todo,
    theme: &Theme,
) {
    let frame_area = frame.area();
    if frame_area.width == 0 || frame_area.height == 0 {
        return;
    }
    let width = frame_area.width.saturating_mul(80) / 100;
    let height = frame_area.height.saturating_mul(80) / 100;
    if width < 3 || height < 3 {
        return;
    }
    let area = Rect::new(
        frame_area.x + (frame_area.width - width) / 2,
        frame_area.y + (frame_area.height - height) / 2,
        width,
        height,
    );
    let block = Block::bordered()
        .title(" Todo details ")
        .style(Style::default().fg(theme.foreground).bg(theme.background))
        .border_style(Style::default().fg(theme.mode_background));
    let inner = block.inner(area);
    let mut lines = wrapped_slices(&todo.title, inner.width)
        .into_iter()
        .map(|line| {
            Line::from(Span::styled(
                line,
                Style::default().add_modifier(Modifier::BOLD),
            ))
        })
        .collect::<Vec<_>>();
    lines.push(Line::default());
    lines.extend(
        wrapped_slices(&todo.body, inner.width)
            .into_iter()
            .map(Line::raw),
    );
    let maximum_scroll = lines.len().saturating_sub(usize::from(inner.height));
    let maximum_scroll = u16::try_from(maximum_scroll).unwrap_or(u16::MAX);
    preview.scroll = preview.scroll.min(maximum_scroll);
    let paragraph =
        Paragraph::new(lines).style(Style::default().fg(theme.foreground).bg(theme.background));

    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(paragraph.scroll((preview.scroll, 0)), inner);
}

fn wrapped_slices(text: &str, width: u16) -> Vec<&str> {
    let width = usize::from(width);
    let mut lines = Vec::new();
    for explicit_line in text.split('\n') {
        if explicit_line.is_empty() {
            lines.push(explicit_line);
            continue;
        }

        let mut start = 0;
        let mut used_width = 0usize;
        for (index, grapheme) in explicit_line.grapheme_indices(true) {
            let grapheme_width = UnicodeWidthStr::width(grapheme);
            if used_width > 0 && used_width.saturating_add(grapheme_width) > width {
                lines.push(&explicit_line[start..index]);
                start = index;
                used_width = 0;
            }
            if used_width == 0 && grapheme_width > width {
                lines.push(grapheme);
                start = index + grapheme.len();
            } else {
                used_width = used_width.saturating_add(grapheme_width);
            }
        }
        if start < explicit_line.len() {
            lines.push(&explicit_line[start..]);
        }
    }
    lines
}
#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn render_branch_sections(
    frame: &mut Frame,
    area: Rect,
    rows: &[DisplayRowLayout<'_>],
    focus: Option<&Focus>,
    hovered: Option<&Focus>,
    editor: Option<&Editor>,
    select_state: Option<&SelectState>,
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
                let marker = if let Some(select_state) = select_state {
                    if select_state.branch_ref == todo.branch_ref {
                        if select_state.selected_todo_ids.contains(&todo.id) {
                            SELECTED_TODO_MARKER
                        } else {
                            UNSELECTED_TODO_MARKER
                        }
                    } else if todo.completed {
                        COMPLETE_TODO_MARKER
                    } else {
                        INCOMPLETE_TODO_MARKER
                    }
                } else if todo.completed {
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
                    Paragraph::new(format!(
                        " {} {marker} ",
                        if todo.body.is_empty() { " " } else { "≡" }
                    ))
                    .style(style),
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

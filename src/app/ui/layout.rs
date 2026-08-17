use std::ops::Range;

use ratatui::{
    layout::{Constraint, Layout, Position, Rect},
    widgets::{Block, Padding},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::super::{Editor, EditorTarget, Focus};
use crate::repository::BranchSection;
use crate::storage::{Todo, TodoId};
pub(in crate::app) fn app_areas(area: Rect) -> [Rect; 3] {
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area)
}

pub(in crate::app) fn todo_viewport_area(frame_area: Rect) -> Rect {
    let [_, content_area, _] = app_areas(frame_area);
    Block::bordered()
        .padding(Padding::horizontal(1))
        .inner(content_area)
}
pub(super) enum DisplayRow<'a> {
    Header(&'a BranchSection),
    Todo(&'a Todo),
    Editor { editor: &'a Editor, completed: bool },
    Empty,
}

fn display_rows<'a>(
    sections: &'a [BranchSection],
    todos: &'a [Todo],
    editor: Option<&'a Editor>,
) -> Vec<DisplayRow<'a>> {
    let mut rows = Vec::new();
    for section in sections {
        rows.push(DisplayRow::Header(section));
        let branch_todos = todos
            .iter()
            .filter(|todo| todo.branch_ref == section.full_ref_name)
            .collect::<Vec<_>>();
        let create_editor = editor.filter(|editor| {
            matches!(
                &editor.target,
                EditorTarget::Create { branch_ref, .. }
                    if branch_ref == &section.full_ref_name
            )
        });
        if branch_todos.is_empty() && create_editor.is_none() {
            rows.push(DisplayRow::Empty);
            continue;
        }
        if create_editor.is_some_and(|editor| {
            matches!(&editor.target, EditorTarget::Create { after: None, .. })
        }) {
            rows.push(DisplayRow::Editor {
                editor: create_editor.expect("checked above"),
                completed: false,
            });
        }
        for todo in branch_todos {
            if editor.is_some_and(
                |editor| matches!(&editor.target, EditorTarget::Update { id } if *id == todo.id),
            ) {
                rows.push(DisplayRow::Editor {
                    editor: editor.expect("checked above"),
                    completed: todo.completed,
                });
            } else {
                rows.push(DisplayRow::Todo(todo));
            }
            if create_editor.is_some_and(|editor| {
                matches!(&editor.target, EditorTarget::Create { after, .. } if *after == Some(todo.id))
            }) {
                rows.push(DisplayRow::Editor {
                    editor: create_editor.expect("checked above"),
                    completed: false,
                });
            }
        }
    }
    rows
}
const TODO_PREFIX_WIDTH: u16 = 5;

pub(in crate::app) struct DisplayRowLayout<'a> {
    pub(super) row: DisplayRow<'a>,
    pub(super) title_ranges: Vec<Range<usize>>,
    pub(super) editor_cursor: Option<(usize, usize)>,
}

impl DisplayRowLayout<'_> {
    pub(super) fn visual_height(&self) -> usize {
        self.title_ranges.len().max(1)
    }
}

fn wrap_title(title: &str, width: u16) -> Vec<Range<usize>> {
    let width = usize::from(width);
    let mut wrapped = Vec::new();
    let mut explicit_start = 0;

    for explicit_line in title.split('\n') {
        let explicit_end = explicit_start + explicit_line.len();
        if width == 0 || explicit_line.is_empty() {
            wrapped.push(explicit_start..explicit_start);
        } else {
            let mut remaining_start = explicit_start;
            while remaining_start < explicit_end {
                let remaining = &title[remaining_start..explicit_end];
                let mut used_width = 0usize;
                let mut fitted_end = 0;
                for (index, grapheme) in remaining.grapheme_indices(true) {
                    let grapheme_width = UnicodeWidthStr::width(grapheme);
                    let next_width = used_width.saturating_add(grapheme_width);
                    if next_width > width {
                        if fitted_end == 0 {
                            fitted_end = index + grapheme.len();
                        }
                        break;
                    }
                    fitted_end = index + grapheme.len();
                    used_width = next_width;
                }

                if fitted_end == remaining.len() {
                    wrapped.push(remaining_start..explicit_end);
                    break;
                }

                let word_break = remaining
                    .split_word_bound_indices()
                    .map(|(index, _)| index)
                    .take_while(|index| *index <= fitted_end)
                    .filter(|index| *index > 0)
                    .last();
                let line_end = word_break.unwrap_or(fitted_end);
                let display_end = remaining_start + remaining[..line_end].trim_end().len();
                wrapped.push(remaining_start..display_end);

                let next_start = remaining_start + line_end;
                let trimmed = title[next_start..explicit_end].trim_start();
                remaining_start = explicit_end - trimmed.len();
            }
        }

        explicit_start = explicit_end.saturating_add(1).min(title.len());
    }

    if wrapped.is_empty() {
        wrapped.push(0..0);
    }
    wrapped
}

fn wrapped_cursor(text: &str, lines: &[Range<usize>], cursor: usize) -> (usize, usize) {
    for (row, line) in lines.iter().enumerate() {
        let next_start = lines
            .get(row + 1)
            .map_or(text.len().saturating_add(1), |next| next.start);
        if cursor < next_start || row + 1 == lines.len() {
            let cursor = cursor.clamp(line.start, line.end);
            return (row, UnicodeWidthStr::width(&text[line.start..cursor]));
        }
    }
    (0, 0)
}

fn layout_display_rows<'a>(rows: Vec<DisplayRow<'a>>, width: u16) -> Vec<DisplayRowLayout<'a>> {
    let title_width = width.saturating_sub(TODO_PREFIX_WIDTH);
    rows.into_iter()
        .map(|row| {
            let title_ranges = match &row {
                DisplayRow::Todo(todo) => wrap_title(&todo.title, title_width),
                DisplayRow::Editor { editor, .. } => wrap_title(&editor.text, title_width),
                DisplayRow::Header(_) | DisplayRow::Empty => Vec::new(),
            };
            let editor_cursor = match &row {
                DisplayRow::Editor { editor, .. } => {
                    Some(wrapped_cursor(&editor.text, &title_ranges, editor.cursor))
                }
                DisplayRow::Header(_) | DisplayRow::Todo(_) | DisplayRow::Empty => None,
            };
            DisplayRowLayout {
                row,
                title_ranges,
                editor_cursor,
            }
        })
        .collect()
}
pub(in crate::app) fn build_display_layout<'a>(
    sections: &'a [BranchSection],
    todos: &'a [Todo],
    editor: Option<&'a Editor>,
    width: u16,
) -> Vec<DisplayRowLayout<'a>> {
    layout_display_rows(display_rows(sections, todos, editor), width)
}

fn row_has_focus(
    layout: &DisplayRowLayout<'_>,
    focus: Option<&Focus>,
    editor: Option<&Editor>,
) -> bool {
    match &layout.row {
        DisplayRow::Header(section) => {
            editor.is_none()
                && matches!(focus, Some(Focus::Branch(branch_ref)) if branch_ref == &section.full_ref_name)
        }
        DisplayRow::Todo(todo) => {
            editor.is_none() && matches!(focus, Some(Focus::Todo(id)) if *id == todo.id)
        }
        DisplayRow::Editor { .. } => true,
        DisplayRow::Empty => false,
    }
}
pub(in crate::app) fn maximum_viewport_start(rows: &[DisplayRowLayout<'_>], height: u16) -> usize {
    if rows.is_empty() || height == 0 {
        return 0;
    }

    let height = usize::from(height);
    let mut start = rows.len() - 1;
    let mut occupied = rows[start].visual_height();
    while start > 0 {
        let preceding_height = rows[start - 1].visual_height();
        if occupied.saturating_add(preceding_height) > height {
            break;
        }
        start -= 1;
        occupied = occupied.saturating_add(preceding_height);
    }
    start
}
fn hit_test_display_row<'layout, 'row>(
    rows: &'row [DisplayRowLayout<'layout>],
    area: Rect,
    viewport_start: usize,
    position: Position,
) -> Option<(&'row DisplayRowLayout<'layout>, usize)> {
    if !area.contains(position) {
        return None;
    }
    let first = viewport_start.min(maximum_viewport_start(rows, area.height));
    let target_y = usize::from(position.y - area.y);
    let mut rendered_y = 0;
    for layout in rows.iter().skip(first) {
        if rendered_y >= usize::from(area.height) {
            break;
        }
        let rendered_height = layout
            .visual_height()
            .min(usize::from(area.height) - rendered_y);
        if target_y < rendered_y + rendered_height {
            return Some((layout, target_y - rendered_y));
        }
        rendered_y += rendered_height;
    }
    None
}

fn title_cursor_at_column(title: &str, range: &Range<usize>, column: usize) -> Option<usize> {
    let line = &title[range.clone()];
    if column > UnicodeWidthStr::width(line) {
        return None;
    }

    let mut used_width = 0;
    for (offset, grapheme) in line.grapheme_indices(true) {
        let next_width = used_width + UnicodeWidthStr::width(grapheme);
        if column < next_width {
            return Some(range.start + offset);
        }
        used_width = next_width;
    }
    Some(range.end)
}

pub(in crate::app) fn hit_test_display_rows(
    rows: &[DisplayRowLayout<'_>],
    area: Rect,
    viewport_start: usize,
    position: Position,
) -> Option<Focus> {
    let (layout, _) = hit_test_display_row(rows, area, viewport_start, position)?;
    match &layout.row {
        DisplayRow::Header(section) => Some(Focus::Branch(section.full_ref_name.clone())),
        DisplayRow::Todo(todo) => Some(Focus::Todo(todo.id)),
        DisplayRow::Editor { .. } | DisplayRow::Empty => None,
    }
}

pub(in crate::app) fn hit_test_todo_text(
    rows: &[DisplayRowLayout<'_>],
    area: Rect,
    viewport_start: usize,
    position: Position,
) -> Option<(TodoId, usize)> {
    let title_x = area.x.saturating_add(TODO_PREFIX_WIDTH);
    if position.x < title_x {
        return None;
    }
    let (layout, visual_line) = hit_test_display_row(rows, area, viewport_start, position)?;
    let DisplayRow::Todo(todo) = &layout.row else {
        return None;
    };
    let range = layout.title_ranges.get(visual_line)?;
    let column = usize::from(position.x - title_x);
    let cursor = title_cursor_at_column(&todo.title, range, column)?;
    Some((todo.id, cursor))
}

pub(in crate::app) fn reconcile_viewport_start(
    rows: &[DisplayRowLayout<'_>],
    height: u16,
    viewport_start: usize,
    reveal_focus: bool,
    focus: Option<&Focus>,
    editor: Option<&Editor>,
) -> usize {
    let maximum = maximum_viewport_start(rows, height);
    let mut start = viewport_start.min(maximum);
    if !reveal_focus || height == 0 {
        return start;
    }
    let Some(focused) = rows
        .iter()
        .position(|layout| row_has_focus(layout, focus, editor))
    else {
        return start;
    };

    let height = usize::from(height);
    if focused < start || rows[focused].visual_height() > height {
        return focused.min(maximum);
    }

    let mut occupied = rows[start..=focused].iter().fold(0usize, |total, layout| {
        total.saturating_add(layout.visual_height())
    });
    while occupied > height && start < focused {
        occupied = occupied.saturating_sub(rows[start].visual_height());
        start += 1;
    }
    start.min(maximum)
}

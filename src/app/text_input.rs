use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

pub(in crate::app) fn edit_line(text: &mut String, cursor: &mut usize, key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Char(character) => {
            text.insert(*cursor, character);
            let insertion_end = *cursor + character.len_utf8();
            *cursor = boundary_at_or_after(text, insertion_end);
            true
        }
        KeyCode::Backspace if *cursor > 0 => {
            let previous = previous_boundary(text, *cursor);
            text.drain(previous..*cursor);
            *cursor = previous;
            true
        }
        KeyCode::Delete if *cursor < text.len() => {
            let next = next_boundary(text, *cursor);
            text.drain(*cursor..next);
            true
        }
        KeyCode::Left
            if key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
        {
            *cursor = previous_word_boundary(text, *cursor);
            false
        }
        KeyCode::Right
            if key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
        {
            *cursor = next_word_boundary(text, *cursor);
            false
        }
        KeyCode::Left => {
            *cursor = previous_boundary(text, *cursor);
            false
        }
        KeyCode::Right => {
            *cursor = next_boundary(text, *cursor);
            false
        }
        KeyCode::Home => {
            *cursor = 0;
            false
        }
        KeyCode::End => {
            *cursor = text.len();
            false
        }
        _ => false,
    }
}

fn previous_boundary(text: &str, cursor: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(index, _)| index)
        .take_while(|index| *index < cursor)
        .last()
        .unwrap_or(0)
}

fn next_boundary(text: &str, cursor: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(index, _)| index)
        .find(|index| *index > cursor)
        .unwrap_or(text.len())
}

fn previous_word_boundary(text: &str, cursor: usize) -> usize {
    text[..cursor]
        .split_word_bound_indices()
        .filter(|(_, segment)| !segment.chars().all(char::is_whitespace))
        .map(|(index, _)| index)
        .last()
        .unwrap_or(0)
}

fn next_word_boundary(text: &str, cursor: usize) -> usize {
    text[cursor..]
        .split_word_bound_indices()
        .skip(1)
        .find(|(_, segment)| !segment.chars().all(char::is_whitespace))
        .map(|(index, _)| cursor + index)
        .unwrap_or(text.len())
}

fn boundary_at_or_after(text: &str, cursor: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(index, _)| index)
        .find(|index| *index >= cursor)
        .unwrap_or(text.len())
}

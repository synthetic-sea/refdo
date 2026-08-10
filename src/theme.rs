use std::{error::Error, fmt};

use ratatui::style::Color;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Theme {
    pub background: Color,
    pub foreground: Color,
    pub foreground_muted: Color,
    pub selection_background: Color,
    pub hover_background: Color,
    pub status_bar_background: Color,
    pub mode_background: Color,
    pub mode_foreground: Color,
}

pub const TOKYO_NIGHT_DAY: Theme = Theme {
    background: Color::Rgb(225, 226, 231),
    foreground: Color::Rgb(55, 96, 191),
    foreground_muted: Color::Rgb(137, 144, 179),
    selection_background: Color::Rgb(183, 193, 227),
    hover_background: Color::Rgb(207, 211, 226),
    status_bar_background: Color::Rgb(196, 200, 218),
    mode_background: Color::Rgb(46, 125, 233),
    mode_foreground: Color::Rgb(208, 213, 227),
};

pub const TOKYO_NIGHT: Theme = Theme {
    background: Color::Rgb(26, 27, 38),
    foreground: Color::Rgb(122, 162, 247),
    foreground_muted: Color::Rgb(86, 95, 137),
    selection_background: Color::Rgb(40, 52, 87),
    hover_background: Color::Rgb(36, 40, 59),
    status_bar_background: Color::Rgb(22, 22, 30),
    mode_background: Color::Rgb(122, 162, 247),
    mode_foreground: Color::Rgb(26, 27, 38),
};

#[derive(Debug, Eq, PartialEq)]
pub struct ThemeError {
    name: String,
}

impl fmt::Display for ThemeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown theme name: {:?}", self.name)
    }
}

impl Error for ThemeError {}

pub fn by_name(name: &str) -> Result<Theme, ThemeError> {
    match name {
        "tokyo-night-day" => Ok(TOKYO_NIGHT_DAY),
        "tokyo-night" => Ok(TOKYO_NIGHT),
        _ => Err(ThemeError {
            name: name.to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_supported_theme_names() {
        assert_eq!(by_name("tokyo-night-day"), Ok(TOKYO_NIGHT_DAY));
        assert_eq!(by_name("tokyo-night"), Ok(TOKYO_NIGHT));
    }

    #[test]
    fn rejects_unknown_theme_names() {
        let error = by_name("Tokyo Night").unwrap_err();

        assert_eq!(error.to_string(), "unknown theme name: \"Tokyo Night\"");
    }
}

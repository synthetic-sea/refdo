use ratatui::style::Color;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Theme {
    pub background: Color,
    pub foreground: Color,
    pub foreground_muted: Color,
}

impl Theme {
    pub const fn new(background: Color, foreground: Color, foreground_muted: Color) -> Self {
        Self {
            background,
            foreground,
            foreground_muted,
        }
    }
}

pub const TOKYO_NIGHT_DAY: Theme = Theme::new(
    Color::Rgb(225, 226, 231),
    Color::Rgb(55, 96, 191),
    Color::Rgb(137, 144, 179),
);

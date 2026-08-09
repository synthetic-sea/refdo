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

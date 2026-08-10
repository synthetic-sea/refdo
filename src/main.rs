mod app;
mod config;
mod repository;
mod storage;
mod theme;

use std::error::Error;

use config::{Config, ThemeMode};

fn main() -> Result<(), Box<dyn Error>> {
    let config = Config::load()?;
    let light_theme = theme::by_name(&config.theme.light)?;
    let dark_theme = theme::by_name(&config.theme.dark)?;
    let selected_theme = match config.theme.mode {
        ThemeMode::Light => light_theme,
        ThemeMode::Dark => dark_theme,
        ThemeMode::System => match dark_light::detect() {
            Ok(dark_light::Mode::Dark) => dark_theme,
            Ok(dark_light::Mode::Light | dark_light::Mode::Unspecified) | Err(_) => light_theme,
        },
    };

    app::run(selected_theme)?;
    Ok(())
}

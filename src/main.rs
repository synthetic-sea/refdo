mod app;
mod config;
mod repository;
mod storage;
mod theme;

use std::error::Error;

use config::Config;

fn main() -> Result<(), Box<dyn Error>> {
    let config = Config::load()?;
    let light_theme = theme::by_name(&config.theme.light)?;
    let dark_theme = theme::by_name(&config.theme.dark)?;
    app::run(
        light_theme,
        dark_theme,
        config.theme.mode,
        config.dispatch,
        config.dispatches,
    )?;
    Ok(())
}

mod app;
mod repository;
mod storage;
mod theme;

fn main() -> std::io::Result<()> {
    app::run()
}

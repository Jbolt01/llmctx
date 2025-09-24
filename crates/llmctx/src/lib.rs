pub mod app;
pub mod domain;
pub mod infra;
pub mod ui;

pub fn init() {
    tracing_subscriber::fmt::init();
}

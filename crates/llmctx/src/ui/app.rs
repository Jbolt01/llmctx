//! Application loop for the TUI.

#[derive(Default)]
pub struct UiApp;

impl UiApp {
    pub fn new() -> Self {
        Self
    }

    pub fn run(&mut self) -> anyhow::Result<()> {
        // TODO: implement TUI loop
        Ok(())
    }
}

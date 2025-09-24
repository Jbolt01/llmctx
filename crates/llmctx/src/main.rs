fn main() -> anyhow::Result<()> {
    llmctx::init();

    let mut app = llmctx::ui::app::UiApp;
    app.run()
}

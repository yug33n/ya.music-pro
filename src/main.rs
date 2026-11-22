pub mod core;

slint::include_modules!();

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    log::info!("Starting ya.music-pro core...");

    let _channels = core::CoreChannels::new(64, 128);
    let _state = core::AppState::default();

    let main_window = MainWindow::new()?;
    main_window.run()?;

    Ok(())
}

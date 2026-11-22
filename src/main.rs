slint::include_modules!();

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    log::info!("Starting ya.music-pro core...");

    let main_window = MainWindow::new()?;
    main_window.run()?;

    Ok(())
}

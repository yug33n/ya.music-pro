pub mod api;
pub mod audio;
pub mod core;
pub mod db;

slint::include_modules!();

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    log::info!("Starting ya.music-pro core...");

    let _channels = core::CoreChannels::new(64, 128);
    let _state = core::AppState::default();
    let _db = db::DatabaseRepository::in_memory()?;
    let _api = api::YandexApiClient::new();
    let _device_mgr = audio::DeviceManager::new();

    let main_window = MainWindow::new()?;
    main_window.run()?;

    Ok(())
}

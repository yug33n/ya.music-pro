pub mod api;
pub mod audio;
pub mod core;
pub mod db;
pub mod dsp;
pub mod lyrics;
pub mod platform;
pub mod ui;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

slint::include_modules!();

pub fn format_duration(ms: u64) -> String {
    let total_seconds = ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{}:{:02}", minutes, seconds)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    log::info!("Starting ya.music-pro: Native Yandex Music Client");

    let channels = core::CoreChannels::new(64, 128);
    let mut state = core::AppState::default();
    let db = std::sync::Arc::new(db::DatabaseRepository::in_memory()?);
    let _blacklist_engine = core::BlacklistEngine::new(Some(db.clone()));
    let api_client = api::YandexApiClient::new();
    let device_mgr = audio::DeviceManager::new();

    // Check if user has saved token
    if let Ok(true) = api_client.load_token_from_db(&db).await {
        state.is_authorized = true;
        if let Ok(account) = api_client.get_account_status().await {
            if let Some(info) = account.account {
                state.user_name = info.display_name.or(info.login);
            }
        }
    }

    let main_window = MainWindow::new()?;

    // Sync initial state to UI
    main_window.set_is_authorized(state.is_authorized);
    if let Some(ref name) = state.user_name {
        main_window.set_user_name(SharedString::from(name.as_str()));
    }
    main_window.set_volume(state.volume);
    main_window.set_is_muted(state.is_muted);
    main_window.set_vibe_character(SharedString::from(state.active_vibe.character.as_str()));
    main_window.set_vibe_mood(SharedString::from(state.active_vibe.mood.as_str()));
    main_window.set_vibe_language(SharedString::from(state.active_vibe.language.as_str()));

    // Create sample collection tracks for preview
    let sample_tracks: Vec<TrackData> = vec![
        TrackData {
            id: SharedString::from("101"),
            title: SharedString::from("Комета"),
            artists: SharedString::from("JONY"),
            album: SharedString::from("Список твоих мыслей"),
            duration: SharedString::from("2:41"),
            is_liked: true,
            explicit: false,
            is_playing: false,
        },
        TrackData {
            id: SharedString::from("102"),
            title: SharedString::from("Где прошла ты"),
            artists: SharedString::from("Кравц, Гио Пика"),
            album: SharedString::from("17 мгновений осени"),
            duration: SharedString::from("2:55"),
            is_liked: true,
            explicit: true,
            is_playing: false,
        },
        TrackData {
            id: SharedString::from("103"),
            title: SharedString::from("Сияй"),
            artists: SharedString::from("Ramil'"),
            album: SharedString::from("Сияй"),
            duration: SharedString::from("3:04"),
            is_liked: true,
            explicit: false,
            is_playing: false,
        },
    ];

    main_window.set_collection_count(sample_tracks.len() as i32);
    main_window.set_collection_tracks(ModelRc::new(VecModel::from(sample_tracks)));

    // Sample LRC lyrics for preview
    let sample_lyrics = vec![
        LyricRow {
            timestamp_ms: 0,
            text: SharedString::from("Давай с тобой уедем туда, где нас не найдут"),
        },
        LyricRow {
            timestamp_ms: 5_000,
            text: SharedString::from("Где волны бьются о скалы и чайки поют"),
        },
        LyricRow {
            timestamp_ms: 12_000,
            text: SharedString::from("И пусть весь мир подождет, пока мы одни"),
        },
        LyricRow {
            timestamp_ms: 18_000,
            text: SharedString::from("Сгорают в небе огни, сгорают огни..."),
        },
    ];
    main_window.set_current_lyrics(ModelRc::new(VecModel::from(sample_lyrics)));
    main_window.set_active_lyric_index(0);

    // Connect callbacks
    let cmd_tx = channels.command_sender.clone();
    main_window.on_play_pause({
        let tx = cmd_tx.clone();
        move || {
            let _ = tx.try_send(core::CoreCommand::Play);
        }
    });

    main_window.on_next_track({
        let tx = cmd_tx.clone();
        move || {
            let _ = tx.try_send(core::CoreCommand::Next);
        }
    });

    main_window.on_prev_track({
        let tx = cmd_tx.clone();
        move || {
            let _ = tx.try_send(core::CoreCommand::Previous);
        }
    });

    main_window.on_shuffle_toggle({
        let tx = cmd_tx.clone();
        move || {
            let _ = tx.try_send(core::CoreCommand::SetShuffleMode(core::ShuffleMode::On));
        }
    });

    main_window.on_repeat_toggle({
        let tx = cmd_tx.clone();
        move || {
            let _ = tx.try_send(core::CoreCommand::SetRepeatMode(core::RepeatMode::All));
        }
    });

    main_window.on_seek_to({
        let tx = cmd_tx.clone();
        move |progress| {
            let target_ms = (progress * 180_000.0) as u64;
            let _ = tx.try_send(core::CoreCommand::Seek(target_ms));
        }
    });

    main_window.on_set_volume({
        let tx = cmd_tx.clone();
        move |vol| {
            let _ = tx.try_send(core::CoreCommand::SetVolume(vol));
        }
    });

    main_window.on_toggle_mute({
        let tx = cmd_tx.clone();
        move || {
            let _ = tx.try_send(core::CoreCommand::ToggleMute);
        }
    });

    main_window.on_toggle_like({
        let tx = cmd_tx.clone();
        move |id| {
            let _ = tx.try_send(core::CoreCommand::ToggleLike(id.to_string()));
        }
    });

    main_window.on_dislike_track({
        let tx = cmd_tx.clone();
        move || {
            let _ = tx.try_send(core::CoreCommand::DislikeTrack(String::new()));
        }
    });

    main_window.on_play_wave({
        let tx = cmd_tx.clone();
        move || {
            log::info!("Starting My Wave playback station...");
            let _ = tx.try_send(core::CoreCommand::Play);
        }
    });

    main_window.on_set_vibe({
        let tx = cmd_tx.clone();
        move |char, mood, lang| {
            let _ = tx.try_send(core::CoreCommand::SetVibe(core::VibeSettings {
                character: char.to_string(),
                mood: mood.to_string(),
                language: lang.to_string(),
            }));
        }
    });

    main_window.on_tab_selected(|tab| {
        log::info!("Switched to tab index: {}", tab);
    });

    let window_weak = main_window.as_weak();
    main_window.on_search_query_changed({
        let api = api_client.clone();
        let weak = window_weak.clone();
        move |query| {
            let api = api.clone();
            let weak = weak.clone();
            let q_str = query.to_string();

            tokio::spawn(async move {
                if q_str.trim().is_empty() {
                    return;
                }
                if let Ok(search_res) = api.search_catalog(&q_str).await {
                    if let Some(tracks) = search_res.tracks {
                        let slint_tracks: Vec<TrackData> = tracks
                            .items
                            .into_iter()
                            .map(|t| {
                                let core = t.to_core_track(false);
                                TrackData {
                                    id: SharedString::from(core.id.as_str()),
                                    title: SharedString::from(core.title.as_str()),
                                    artists: SharedString::from(core.artists_display().as_str()),
                                    album: SharedString::from(core.album.as_deref().unwrap_or("")),
                                    duration: SharedString::from(format_duration(core.duration_ms).as_str()),
                                    is_liked: false,
                                    explicit: core.explicit,
                                    is_playing: false,
                                }
                            })
                            .collect();

                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(w) = weak.upgrade() {
                                w.set_search_results(ModelRc::new(VecModel::from(slint_tracks)));
                            }
                        });
                    }
                }
            });
        }
    });

    main_window.on_request_auth({
        let api = api_client.clone();
        move || {
            let api = api.clone();
            tokio::spawn(async move {
                log::info!("Starting OAuth Device Code authorization...");
                if let Ok(code_resp) = api.request_device_code().await {
                    log::info!(
                        "Go to {} and enter code: {}",
                        code_resp.verification_url,
                        code_resp.user_code
                    );
                }
            });
        }
    });

    main_window.on_toggle_lyrics(|| {
        log::info!("Toggle fullscreen lyrics requested");
    });

    main_window.on_lyric_line_clicked({
        let tx = cmd_tx.clone();
        move |timestamp_ms| {
            log::info!("Seeking to lyric line timestamp: {} ms", timestamp_ms);
            let _ = tx.try_send(core::CoreCommand::Seek(timestamp_ms as u64));
        }
    });

    let engine = audio::AudioEngine::new(&device_mgr).ok();
    let _ducking_handle = engine.as_ref().map(|eng| {
        platform::start_voice_activity_monitor(
            eng.controller.clone(),
            std::time::Duration::from_millis(50),
        )
    });

    // System integrations: Discord RPC, Global Hotkeys, Ultra-light Headless Tray
    let mut discord_rpc = platform::DiscordRpcService::new(None);
    let _ = discord_rpc.connect();

    let hotkeys = platform::GlobalHotkeysEngine::new().ok();
    let tray = platform::HeadlessTrayMode::new().ok();

    let timer = slint::Timer::default();
    let cmd_timer_tx = cmd_tx.clone();
    let window_timer_weak = window_weak.clone();

    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(100),
        move || {
            // 1. Poll global hotkeys
            if let Some(ref hk) = hotkeys {
                if let Some(action) = hk.poll_event() {
                    match action {
                        platform::GlobalHotkeyAction::PlayPause => {
                            let _ = cmd_timer_tx.try_send(core::CoreCommand::Play);
                        }
                        platform::GlobalHotkeyAction::NextTrack => {
                            let _ = cmd_timer_tx.try_send(core::CoreCommand::Next);
                        }
                        platform::GlobalHotkeyAction::PreviousTrack => {
                            let _ = cmd_timer_tx.try_send(core::CoreCommand::Previous);
                        }
                        platform::GlobalHotkeyAction::SeekForward10s => {
                            log::info!("Hotkey: Seek +10s");
                        }
                        platform::GlobalHotkeyAction::SeekBackward10s => {
                            log::info!("Hotkey: Seek -10s");
                        }
                        platform::GlobalHotkeyAction::ToggleLike => {
                            log::info!("Hotkey: Toggle Like");
                        }
                        platform::GlobalHotkeyAction::Dislike => {
                            let _ = cmd_timer_tx.try_send(core::CoreCommand::DislikeTrack(String::new()));
                        }
                        platform::GlobalHotkeyAction::NextEqPreset => {
                            log::info!("Hotkey: Cycle EQ Preset");
                        }
                    }
                }
            }

            // 2. Poll tray icon events
            if let Some(ref t) = tray {
                if let Some(action) = t.poll_tray_events() {
                    match action {
                        platform::TrayEventAction::RestoreWindow => {
                            if let Some(w) = window_timer_weak.upgrade() {
                                let _ = w.show();
                            }
                        }
                        platform::TrayEventAction::MinimizeToTray => {
                            if let Some(w) = window_timer_weak.upgrade() {
                                let _ = w.hide();
                            }
                        }
                        platform::TrayEventAction::PlayPause => {
                            let _ = cmd_timer_tx.try_send(core::CoreCommand::Play);
                        }
                        platform::TrayEventAction::NextTrack => {
                            let _ = cmd_timer_tx.try_send(core::CoreCommand::Next);
                        }
                        platform::TrayEventAction::Quit => {
                            std::process::exit(0);
                        }
                        _ => {}
                    }
                }
            }
        },
    );

    main_window.run()?;
    Ok(())
}

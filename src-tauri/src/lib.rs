pub mod audio;
pub mod ble;
pub mod config;
pub mod db;
pub mod download;
pub mod tap;
pub mod hid;
#[cfg(feature = "inject")]
pub mod inject;
pub mod interception;
pub mod keymap;
pub mod keyslots;
pub mod llm;
pub mod model_caps;
pub mod overlay;
pub mod process;
pub mod protocol;
pub mod runtime;
pub mod stats;
pub mod stt;
pub mod tray;
pub mod voice;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_logging();
    let args: Vec<String> = std::env::args().collect();
    // Elevated helper entry: inject/eject the hook DLL inside WUDFHost. Must run
    // before the Tauri builder so single-instance does not swallow the launch.
    if args.iter().any(|a| a == "--inject-tap" || a == "--eject-tap") {
        std::process::exit(crate::tap::run_elevated_action(&args));
    }
    if args.iter().any(|arg| arg == "--generate-caps") {
        let code = match crate::model_caps::generate_assets() {
            Ok(status) => {
                println!(
                    "已生成能力表：{} 个模型（{} 个会思考）→ assets/model-caps.json",
                    status.models, status.reasoning
                );
                0
            }
            Err(error) => {
                eprintln!("生成能力表失败：{error}");
                1
            }
        };
        std::process::exit(code);
    }

    tauri::Builder::default()
        // Must be registered first: a second launch focuses the running
        // instance instead of starting another one.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            crate::tray::show_main(app);
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(runtime::AppState::new())
        .setup(|app| {
            use tauri::Manager;
            log::info!("clay-mic starting");
            runtime::init(app.handle().clone());
            crate::tray::init(app)?;
            crate::overlay::window::apply_saved_size(app.handle());
            // The boot service runs its own copy from %LOCALAPPDATA%; refresh
            // it so an app update reaches the next boot without re-registering.
            std::thread::spawn(crate::keyslots::refresh_staged_helper);
            if let Some(state) = crate::runtime::state() {
                let (width, height) = {
                    let config = state.config.lock().unwrap();
                    (config.window.width, config.window.height)
                };
                if let Some(main) = app.get_webview_window("main") {
                    let w = if width >= 400 { width } else { 1040 };
                    let h = if height >= 300 { height } else { 760 };
                    let _ = main.set_size(tauri::PhysicalSize::new(w, h));
                    let _ = main.show();
                }
            }
            if let Some(state) = crate::runtime::state() {
                let hotkey = state.config.lock().unwrap().overlay.hotkey.clone();
                crate::overlay::window::register_hotkey(app.handle(), &hotkey);
            }
            // If suppression was on when we exited, restore the SAME state on
            // launch (hook + suppression). Done on a background thread after
            // the window shows, never blocking setup.
            if crate::keymap::snapshot().suppress {
                std::thread::spawn(move || {
                    // Short delay lets the window paint first; the pipe server
                    // is brought up immediately by get_tap_status if the UI
                    // is already open (see refresh_status_async).
                    std::thread::sleep(std::time::Duration::from_millis(800));
                    if let Err(error) = apply_suppression(true) {
                        log::warn!("restore suppression on launch failed: {}", error);
                    }
                });
            } else if crate::tap::status().injected {
                // Resident hook DLL from last session: accept reconnects even
                // without suppression so status does not stick at a false
                // "not connected" after restart.
                std::thread::spawn(crate::tap::ensure_server_public);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::connect_device,
            commands::list_paired_devices,
            commands::get_text_list,
            commands::get_stats,
            commands::inject_text,
            commands::get_config,
            commands::update_config,
            commands::get_keymap,
            commands::save_keymap,
            commands::pick_script,
            commands::set_suppression_enabled,
            commands::stt_status,
            commands::warmup_stt,
            commands::download_stt_model,
            commands::download_stt_binary,
            commands::get_interception_status,
            commands::get_filter_chain_status,
            commands::download_interception,
            commands::install_driver_and_service,
            commands::repair_driver_pair,
            commands::repair_patch,
            commands::get_keyslot_status,
            commands::uninstall_driver_and_service,
            commands::hide_overlay,
            commands::retry_item,
            commands::delete_item,
            commands::clear_text_list,
            commands::list_models,
            commands::open_bluetooth_settings,
            commands::get_caps_status,
            commands::refresh_model_caps,
            commands::get_thinking_options,
            commands::get_caps_providers,
            commands::get_tap_status,
            commands::inject_tap,
            commands::remove_tap,
        ])
        .on_window_event(|window, event| {
            match window.label() {
                "main" => match event {
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        if let Ok(size) = window.inner_size() {
                            persist_main_size(size);
                        }
                        let close_to_tray = crate::runtime::state()
                            .map(|state| state.config.lock().unwrap().window.close_to_tray)
                            .unwrap_or(true);
                        if close_to_tray {
                            // Keep running in the tray; quit from the tray menu.
                            api.prevent_close();
                            let _ = window.hide();
                        } else if let Some(app) = crate::runtime::handle() {
                            app.exit(0);
                        }
                    }
                    _ => {}
                },
                "overlay" => {
                    if let tauri::WindowEvent::Resized(size) = event {
                        crate::overlay::window::remember_size(size.width, size.height);
                    }
                }
                _ => {}
            }
        })
        .build(tauri::generate_context!())
        .expect("error while running clay-mic")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                use tauri::Manager;
                if let Some(window) = app.get_webview_window("main") {
                    if let Ok(size) = window.inner_size() {
                        persist_main_size(size);
                    }
                }
                crate::stt::shutdown_server();
            }
        });
}

/// Remember the main window size across runs. Saved when the window closes or
/// the app exits rather than on every resize, so startup resizing stays quiet.
fn persist_main_size(size: tauri::PhysicalSize<u32>) {
    if size.width < 400 || size.height < 300 {
        return;
    }
    let Some(state) = crate::runtime::state() else {
        return;
    };
    let mut config = state.config.lock().unwrap();
    if config.window.width != size.width || config.window.height != size.height {
        config.window.width = size.width;
        config.window.height = size.height;
        let _ = config.save();
    }
}

pub(crate) fn init_logging() {
    use std::io::Write;

    let dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clay-mic");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("clay-mic.log");

    let target = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(file) => {
            struct Tee {
                file: std::fs::File,
            }
            impl Write for Tee {
                fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                    let _ = std::io::stderr().write_all(buf);
                    self.file.write(buf)
                }
                fn flush(&mut self) -> std::io::Result<()> {
                    let _ = std::io::stderr().flush();
                    self.file.flush()
                }
            }
            env_logger::Target::Pipe(Box::new(Tee { file }))
        }
        Err(_) => env_logger::Target::Stderr,
    };

    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .target(target)
        .format_timestamp_millis()
        .try_init();
}

/// Activate suppression: enable HID capture + trigger tap inject.
/// Called when the toggle is ON and/or a device connects.
fn activate_suppression() {
    if !crate::keymap::snapshot().suppress {
        return;
    }
    let _ = crate::hid::start_capture_if_needed();
    let _ = crate::hid::set_suppression(true, None);
    std::thread::spawn(crate::tap::try_inject_if_ready);
}

/// Deactivate suppression: stop HID capture. Tap DLL stays loaded but events
/// are ignored (config.suppress is checked in on_tap_event).
fn deactivate_suppression() {
    let _ = crate::hid::set_suppression(false, None);
    let _ = crate::hid::stop_capture();
}

/// Persist the suppression preference and activate/deactivate accordingly.
fn apply_suppression(enabled: bool) -> Result<(), String> {
    let mut config = crate::keymap::KeymapConfig::load();
    config.suppress = enabled;
    crate::keymap::set(config.clone());
    log::info!("apply_suppression enabled={}", enabled);
    let _ = config.save();
    if enabled {
        activate_suppression();
    } else {
        deactivate_suppression();
    }
    Ok(())
}

mod commands {
    use crate::config::AppConfig;
    use crate::overlay::TextItem;
    use crate::runtime::AppState;
    use tauri::State;

    #[tauri::command]
    pub fn get_status() -> String {
        "ready".to_string()
    }

    #[derive(serde::Serialize)]
    pub struct ConnectedDevice {
        pub name: String,
        pub vendor_id: Option<u16>,
        pub product_id: Option<u16>,
        pub model: Option<String>,
    }

    /// Connect to a BLE device by Bluetooth address (MAC) and keep the
    /// session alive so voice capture can use it later. Returns the identity
    /// the caller persists and the capture layer later matches on.
    #[tauri::command]
    pub async fn connect_device(address: String) -> Result<ConnectedDevice, String> {
        log::info!("Connecting to device: {}", address);
        let device = crate::ble::scanner::connect_and_store(&address)?;
        super::activate_suppression();
        Ok(ConnectedDevice {
            name: device.name,
            vendor_id: device.vendor_id,
            product_id: device.product_id,
            model: device.model,
        })
    }

    /// List BLE devices already paired with the OS (for the in-app picker).
    #[tauri::command]
    pub async fn list_paired_devices() -> Result<Vec<crate::ble::scanner::BleDeviceInfo>, String> {
        crate::ble::scanner::list_paired_devices()
    }

    /// Current key mapping (from keymap.json).
    #[tauri::command]
    pub fn get_keymap() -> crate::keymap::KeymapConfig {
        crate::keymap::snapshot()
    }

    /// Persist the key mapping. This ONLY writes the config file.
    #[tauri::command]
    pub fn save_keymap(config: crate::keymap::KeymapConfig) -> Result<(), String> {
        config.save()?;
        crate::keymap::set(config);
        Ok(())
    }

    /// Read the saved keymap and enable/disable suppression from it. This is
    /// the single place that applies suppression.
    #[tauri::command]
    pub fn set_suppression_enabled(enabled: bool) -> Result<(), String> {
        crate::apply_suppression(enabled)
    }

    /// Native file picker for an `exec` binding's script path.
    #[tauri::command]
    pub fn pick_script() -> Result<Option<String>, String> {
        crate::keymap::pick_script()
    }

    /// Readiness of the local whisper.cpp backend.
    #[tauri::command]
    pub fn stt_status(state: State<'_, AppState>) -> crate::stt::SttStatus {
        let config = state.config.lock().unwrap().clone();
        crate::stt::status(&config.stt)
    }

    /// Preload the configured STT model into the persistent whisper server.
    #[tauri::command]
    pub async fn warmup_stt(state: State<'_, AppState>) -> Result<(), String> {
        let config = state.config.lock().unwrap().clone();
        tauri::async_runtime::spawn_blocking(move || crate::stt::warm_up(&config.stt))
            .await
            .map_err(|error| error.to_string())?
    }

    /// Download a GGML model (e.g. `base`) into the app data directory.
    #[tauri::command]
    pub async fn download_stt_model(model: String) -> Result<String, String> {
        let path = crate::stt::download_model(&model).await?;
        Ok(path.to_string_lossy().into_owned())
    }

    /// Download and extract the prebuilt whisper.cpp binaries.
    #[tauri::command]
    pub async fn download_stt_binary(runtime: String) -> Result<String, String> {
        let path = crate::stt::download_binary(&runtime).await?;
        Ok(path.to_string_lossy().into_owned())
    }

    /// Interception driver status: DLL presence and whether the driver runs.
    #[tauri::command]
    pub fn get_interception_status() -> crate::interception::DriverStatus {
        crate::interception::status()
    }

    /// Which filter drivers the keyboard and mouse classes currently chain.
    #[tauri::command]
    pub fn get_filter_chain_status() -> crate::interception::FilterChainStatus {
        crate::interception::filter_chain_status()
    }

    /// Download the official Interception package into the app data directory.
    #[tauri::command]
    pub async fn download_interception() -> Result<String, String> {
        let path = crate::interception::download().await?;
        Ok(path.to_string_lossy().into_owned())
    }

    /// Shared by the install button and the repair path: one elevated script
    /// (one UAC) where the official installer runs first and the service is
    /// only registered — and started, with failures checked — after it succeeds.
    fn install_pair() -> Result<(), String> {
        let installer = crate::interception::locate_installer()
            .ok_or_else(|| "未找到安装程序，请先下载驱动".to_string())?;
        let helper = crate::keyslots::stage_helper()?;
        let installer = installer.display().to_string().replace('\'', "''");
        let script = format!(
            "$ErrorActionPreference = 'Stop'\n& '{installer}' /install\nif ($LASTEXITCODE -ne 0) {{ throw \"安装程序失败（exit $($LASTEXITCODE)）\" }}\n{}",
            crate::keyslots::install_script(&helper)
        );
        crate::keyslots::run_elevated(&script, "install-driver-service")
    }

    /// Install the Interception driver and the patch in one elevated pass:
    /// a single UAC prompt; the driver part still needs a reboot.
    #[tauri::command]
    pub async fn install_driver_and_service() -> Result<(), String> {
        tauri::async_runtime::spawn_blocking(install_pair)
            .await
            .map_err(|error| error.to_string())?
    }

    /// Diagnose the driver/patch pair and repair whichever half is out of sync.
    #[tauri::command]
    pub async fn repair_driver_pair() -> Result<String, String> {
        tauri::async_runtime::spawn_blocking(|| {
            let chained = crate::interception::filter_chain_status().interception_installed;
            let registered = crate::keyslots::service_installed();
            let status = crate::keyslots::status();
            let applied_ok = status.applied_unix.is_some() && status.ok.unwrap_or(false);
            match (chained, registered, applied_ok) {
                (true, true, true) => Ok("已就绪，无需修复".into()),
                (true, false, _) => {
                    install_pair()?;
                    Ok("已修复：驱动与补丁已重新安装，重启后驱动生效".into())
                }
                (true, true, false) => {
                    let script = format!(
                        "$ErrorActionPreference = 'Stop'\nStart-Service -Name '{}'",
                        crate::keyslots::SERVICE_NAME
                    );
                    crate::keyslots::run_elevated(&script, "repair-reapply")?;
                    Ok("已修复：补丁已重新生效".into())
                }
                (false, true, _) => {
                    crate::keyslots::run_elevated(
                        &crate::keyslots::uninstall_script(),
                        "repair-orphan",
                    )?;
                    Ok("已修复：已清理无驱动的残留补丁".into())
                }
                (false, false, _) => Ok("已干净，无需修复".into()),
            }
        })
        .await
        .map_err(|error| error.to_string())?
    }

    /// Reinstall the keyslots patch service (install_script already handles
    /// stopping/deleting an existing service first).
    #[tauri::command]
    pub async fn repair_patch() -> Result<String, String> {
        tauri::async_runtime::spawn_blocking(|| {
            let helper = crate::keyslots::stage_helper()?;
            crate::keyslots::run_elevated(&crate::keyslots::install_script(&helper), "repair-patch")?;
            Ok("补丁已重新安装".into())
        })
        .await
        .map_err(|error| error.to_string())?
    }

    /// Whether the keyboard-slot service is registered, and what its last run did.
    #[tauri::command]
    pub fn get_keyslot_status() -> crate::keyslots::KeySlotStatus {
        crate::keyslots::status()
    }

    /// Uninstall the driver and the patch in one elevated pass, mirroring the
    /// install: the official installer's `/uninstall` runs first, and a failure
    /// aborts before the patch is touched. Reboot required.
    #[tauri::command]
    pub async fn uninstall_driver_and_service() -> Result<(), String> {
        tauri::async_runtime::spawn_blocking(|| {
            let installer = crate::interception::locate_installer()
                .ok_or_else(|| "未找到安装程序".to_string())?;
            let installer = installer.display().to_string().replace('\'', "''");
            let script = format!(
                "$ErrorActionPreference = 'Stop'\n& '{installer}' /uninstall\nif ($LASTEXITCODE -ne 0) {{ exit $LASTEXITCODE }}\n{}",
                crate::keyslots::uninstall_script()
            );
            crate::keyslots::run_elevated(&script, "uninstall-driver-service")
        })
        .await
        .map_err(|error| error.to_string())?
    }

    /// Loaded model-capability table status (disk cache or embedded).
    #[tauri::command]
    pub fn get_caps_status() -> crate::model_caps::CapsStatus {
        crate::model_caps::status()
    }

    /// Re-download models.dev into the disk cache; manual only, no auto-refresh.
    #[tauri::command]
    pub async fn refresh_model_caps() -> Result<crate::model_caps::CapsStatus, String> {
        crate::model_caps::refresh().await
    }

    /// Thinking-level options for a model; `null` disables the control.
    #[tauri::command]
    pub fn get_thinking_options(
        model: String,
        provider: String,
    ) -> Option<Vec<crate::model_caps::ThinkingOption>> {
        crate::model_caps::thinking_options(&provider, &model)
    }

    /// Providers listed in the capability table, for the provider picker.
    #[tauri::command]
    pub fn get_caps_providers() -> Vec<crate::model_caps::ProviderInfo> {
        crate::model_caps::provider_list()
    }

    /// HID Tap component + injection state for DriverSect. Returns the cached
    /// snapshot immediately; a background thread recomputes and pushes
    /// `tap://status` if anything changed.
    #[tauri::command]
    pub fn get_tap_status() -> crate::tap::TapStatus {
        let cached = crate::tap::status_cached();
        std::thread::spawn(crate::tap::refresh_status_async);
        cached
    }

    /// Explicit inject (may prompt UAC). Does not download.
    #[tauri::command]
    pub async fn inject_tap() -> Result<(), String> {
        tauri::async_runtime::spawn_blocking(crate::tap::inject_command)
            .await
            .map_err(|error| error.to_string())?
    }

    /// Stop the listener and unload the hook DLL (may prompt UAC).
    #[tauri::command]
    pub async fn remove_tap() -> Result<(), String> {
        tauri::async_runtime::spawn_blocking(crate::tap::remove_command)
            .await
            .map_err(|error| error.to_string())?
    }

    /// One page of history, newest first; `page` is 1-based and clamped.
    #[derive(serde::Serialize)]
    pub struct TextListPage {
        pub items: Vec<TextItem>,
        pub total: u64,
        pub page: u32,
        pub size: u32,
    }

    #[tauri::command]
    pub fn get_text_list(
        state: State<'_, AppState>,
        page: Option<u32>,
        size: Option<u32>,
    ) -> TextListPage {
        let size = size.unwrap_or(30).clamp(10, 100);
        let (items, total, page) = state.items_page(page.unwrap_or(1), size);
        TextListPage {
            items,
            total,
            page,
            size,
        }
    }

    #[tauri::command]
    pub fn get_stats(state: State<'_, AppState>) -> crate::stats::UsageStats {
        state.stats.lock().unwrap().clone()
    }

    #[tauri::command]
    pub async fn inject_text(state: State<'_, AppState>, text: String) -> Result<(), String> {
        #[cfg(feature = "inject")]
        {
            let method = state.config.lock().unwrap().inject.method.clone();
            crate::inject::inject(&text, &method)
        }
        #[cfg(not(feature = "inject"))]
        {
            let _ = (&state, &text);
            Err("Text injection not compiled".into())
        }
    }

    #[tauri::command]
    pub fn get_config(state: State<'_, AppState>) -> AppConfig {
        state.config.lock().unwrap().clone()
    }

    #[tauri::command]
    pub fn update_config(
        app: tauri::AppHandle,
        state: State<'_, AppState>,
        config: AppConfig,
    ) -> Result<(), String> {
        let previous_hotkey = state.config.lock().unwrap().overlay.hotkey.clone();
        config.save()?;
        if previous_hotkey != config.overlay.hotkey {
            crate::overlay::window::reregister_hotkey(&app, &config.overlay.hotkey);
        }
        *state.config.lock().unwrap() = config;
        Ok(())
    }

    #[tauri::command]
    pub fn hide_overlay(app: tauri::AppHandle) {
        crate::overlay::window::hide_overlay(&app);
    }

    #[tauri::command]
    pub fn retry_item(state: State<'_, AppState>, id: String) -> Result<(), String> {
        let config = state.config.lock().unwrap().clone();
        let has_raw = state
            .item(&id)
            .is_some_and(|item| !item.raw_text.trim().is_empty());
        if !has_raw {
            return Err("没有可重试的原始文本".into());
        }
        std::thread::spawn(move || crate::voice::retry_format(config, id));
        Ok(())
    }

    #[tauri::command]
    pub fn delete_item(state: State<'_, AppState>, id: String) {
        state.remove_item(&id);
        crate::runtime::emit("text://removed", id);
    }

    #[tauri::command]
    pub fn clear_text_list(state: State<'_, AppState>) {
        state.clear_items();
        crate::runtime::emit("text://cleared", "".to_string());
    }

    #[tauri::command]
    pub async fn list_models(
        base_url: String,
        api_key: String,
        provider: String,
    ) -> Result<Vec<String>, String> {
        crate::llm::provider::list_models(&base_url, &api_key, &provider).await
    }

    #[tauri::command]
    pub async fn open_bluetooth_settings() -> Result<(), String> {
        #[cfg(target_os = "windows")]
        {
            let mut command = std::process::Command::new("cmd");
            command.args(["/c", "start", "ms-settings:bluetooth"]);
            crate::process::hide_console(&mut command);
            command.spawn().map_err(|e| e.to_string())?;
        }
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open")
                .arg("x-apple.systempreferences:com.apple.BluetoothSettings")
                .spawn()
                .map_err(|e| e.to_string())?;
        }
        #[cfg(target_os = "linux")]
        {
            std::process::Command::new("gnome-control-center")
                .arg("bluetooth")
                .spawn()
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

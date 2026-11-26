//! Tauri application entry point: startup Docker bootstrap plus the Phase 5
//! command bridge used by the three-step UI.

use std::sync::Arc;

use tauri::Manager;

mod clean;
mod commands;
mod docker;
pub mod build;
pub mod settings;
pub mod source;

use commands::{BuildState, DockerState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let launch_result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            app.manage(DockerState::default());
            // Managed as Arc so commands can clone it and drop the Tauri State
            // guard before awaiting long-running Docker/git work.
            app.manage(Arc::new(BuildState::default()));

            let handle = app.handle().clone();

            // Image inspection/build can take minutes, so keep it off the UI thread.
            // The shared helper also publishes status events and serializes with
            // build-time re-checks after Full Clean.
            tauri::async_runtime::spawn(async move {
                let _ = commands::ensure_docker_image(&handle).await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::docker_status,
            commands::host_platform,
            commands::choose_data_rsdk,
            commands::choose_output_folder,
            commands::load_settings,
            commands::save_settings,
            commands::validate_asset,
            commands::prepare_source,
            commands::start_build,
            commands::open_output_folder,
            commands::clean_preview,
            commands::full_clean,
        ])
        .run(tauri::generate_context!());

    if let Err(error) = launch_result {
        eprintln!("ChaosLauncher failed to start: {error}");
    }
}

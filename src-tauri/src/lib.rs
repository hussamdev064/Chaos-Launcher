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

use commands::{publish_docker_status, BuildState, DockerState};

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
            tauri::async_runtime::spawn(async move {
                let publish = |stage: &'static str, message: &str| {
                    publish_docker_status(&handle, stage, message.to_owned());
                };

                match docker::ensure_builder_image(|stage| {
                    let message = match stage {
                        "checking" => "Checking Docker daemon and builder image...",
                        "building" => "Building sonic-mania-builder:latest (first run only)...",
                        other => other,
                    };
                    publish(stage, message);
                })
                .await
                {
                    Ok(docker::ImageStatus::AlreadyPresent) => {
                        publish("ready", "Builder image is ready.");
                    }
                    Ok(docker::ImageStatus::Built) => {
                        publish("ready", "Builder image built and ready.");
                    }
                    Err(error) => publish("failed", &error.to_string()),
                }
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

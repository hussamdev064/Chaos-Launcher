//! Scoped Full Clean for launcher-created runtime state.
//!
//! This module deliberately knows only about paths and Docker names created by
//! ChaosLauncher. It never touches the user's Data.rsdk, system Docker sockets,
//! the source checkout of the launcher itself, or an entire custom output root.

use std::fs;
use std::path::{Path, PathBuf};

use bollard::container::{ListContainersOptions, RemoveContainerOptions};
use bollard::errors::Error as DockerApiError;
use bollard::image::RemoveImageOptions;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::docker::{connect_docker, BUILDER_IMAGE};
use crate::settings::{clear_manifest, is_safe_output_child, tracked_outputs, OutputEntry};

/// Containers are named in `build::container::run_container`.
const CONTAINER_PREFIX: &str = "chaoslauncher-build-";
const SOURCE_DIRECTORY: &str = "Sonic-Mania-Decompilation";
const OUTPUT_DIRECTORY: &str = "outputs";
const GRADLE_DIRECTORY: &str = "gradle-cache";
const MACOS_BUILD_DIRECTORY: &str = "chaoslauncher-macos-build";
/// Legacy/managed Android staging cache; generated output staging lives inside
/// timestamped output folders and is removed with its parent.
const ANDROID_STAGING_DIRECTORY: &str = "android-rsdk";
const ANDROID_EXTERNAL_RSDK: &str = "/storage/emulated/0/RSDK";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanPlan {
    /// Launcher-owned filesystem roots that will be removed.
    pub filesystem_paths: Vec<String>,
    /// Timestamped build directories under a user-chosen output root.
    pub custom_output_paths: Vec<String>,
    pub containers: Vec<String>,
    pub images: Vec<String>,
    pub warnings: Vec<String>,
    /// References Full Clean explicitly never removes.
    pub protected_paths: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanRequest {
    /// If false, generated builds inside a custom output root are kept.
    pub include_custom_outputs: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanReport {
    pub removed_paths: Vec<String>,
    pub kept_paths: Vec<String>,
    pub removed_containers: Vec<String>,
    pub removed_images: Vec<String>,
    pub warnings: Vec<String>,
    pub protected_paths: Vec<String>,
}

pub async fn clean_preview(app: &AppHandle) -> CleanPlan {
    let mut warnings = Vec::new();
    let filesystem_paths = managed_filesystem_paths(app, &mut warnings)
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    let managed_outputs = managed_output_root(app);
    let custom_output_paths = tracked_outputs(app)
        .unwrap_or_else(|error| {
            warnings.push(format!("Could not read the output manifest: {error}"));
            Vec::new()
        })
        .into_iter()
        // Managed-cache children are already covered by the `outputs` root.
        .filter(|entry| managed_outputs.as_ref() != Some(&entry.root))
        .map(|entry| entry.path.to_string_lossy().into_owned())
        .collect();
    let (containers, images, docker_warnings) = docker_targets().await;
    warnings.extend(docker_warnings);

    let mut protected_paths = vec![
        "The original Data.rsdk selected in the launcher".to_owned(),
        "Custom output folder roots recorded in the manifest".to_owned(),
    ];
    protected_paths.extend(
        protected_reference_paths()
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned()),
    );

    CleanPlan {
        filesystem_paths,
        custom_output_paths,
        containers,
        images,
        warnings,
        protected_paths,
    }
}

pub async fn full_clean(
    app: &AppHandle,
    request: CleanRequest,
    on_event: &(dyn Fn(crate::build::BuildEvent) + Send + Sync),
) -> CleanReport {
    let plan = clean_preview(app).await;
    let mut report = CleanReport {
        removed_paths: Vec::new(),
        kept_paths: Vec::new(),
        removed_containers: Vec::new(),
        removed_images: Vec::new(),
        warnings: plan.warnings.clone(),
        protected_paths: plan.protected_paths.clone(),
    };

    on_event(crate::build::BuildEvent::phase(
        "cleaning",
        "Removing ChaosLauncher containers...",
    ));
    let docker = match connect_docker() {
        Ok(docker) => Some(docker),
        Err(error) => {
            push_unique(
                &mut report.warnings,
                format!("Docker was unavailable, so no containers/image were removed: {error}"),
            );
            None
        }
    };
    if let Some(docker) = &docker {
        for name in &plan.containers {
            match docker
                .remove_container(
                    name,
                    Some(RemoveContainerOptions {
                        force: true,
                        v: true,
                        ..Default::default()
                    }),
                )
                .await
            {
                Ok(()) => push_unique(&mut report.removed_containers, name.clone()),
                Err(error) => push_unique(
                    &mut report.warnings,
                    format!("Could not remove container {name}: {error}"),
                ),
            }
        }
        for image in &plan.images {
            match docker
                .remove_image(
                    image,
                    Some(RemoveImageOptions {
                        force: true,
                        ..Default::default()
                    }),
                    None,
                )
                .await
            {
                Ok(_) => push_unique(&mut report.removed_images, image.clone()),
                Err(error) => push_unique(
                    &mut report.warnings,
                    format!("Could not remove image {image}: {error}"),
                ),
            }
        }
    }

    on_event(crate::build::BuildEvent::phase(
        "cleaning",
        "Removing generated Sonic Mania outputs...",
    ));
    let managed_outputs = managed_output_root(app);
    let outputs = tracked_outputs(app).unwrap_or_default();
    for OutputEntry { path, root } in outputs {
        // Managed-cache children disappear with the `outputs` root; listing them
        // as "kept" would be misleading when custom outputs are unchecked.
        if managed_outputs.as_ref() == Some(&root) {
            continue;
        }
        if is_protected_reference(&path) || !is_safe_output_child(&path, &root) || !path.is_dir() {
            continue;
        }
        let rendered = path.to_string_lossy().into_owned();
        if request.include_custom_outputs {
            match remove_path(&path) {
                Ok(()) => push_unique(&mut report.removed_paths, rendered),
                Err(error) => push_unique(
                    &mut report.warnings,
                    format!("Could not remove {rendered}: {error}"),
                ),
            }
        } else {
            push_unique(&mut report.kept_paths, rendered);
        }
    }

    on_event(crate::build::BuildEvent::phase(
        "cleaning",
        "Removing launcher caches and source checkout...",
    ));
    let mut path_warnings = Vec::new();
    for path in managed_filesystem_paths(app, &mut path_warnings) {
        let rendered = path.to_string_lossy().into_owned();
        if is_protected_reference(&path) {
            push_unique(
                &mut report.warnings,
                format!("Skipped protected reference: {rendered}"),
            );
            continue;
        }
        match remove_path(&path) {
            Ok(()) => push_unique(&mut report.removed_paths, rendered),
            Err(error) => push_unique(
                &mut report.warnings,
                format!("Could not remove {rendered}: {error}"),
            ),
        }
    }
    for warning in path_warnings {
        push_unique(&mut report.warnings, warning);
    }
    if let Err(error) = clear_manifest(app) {
        push_unique(
            &mut report.warnings,
            format!("Could not reset the output manifest: {error}"),
        );
    }

    on_event(crate::build::BuildEvent::phase(
        "complete",
        "Full Clean finished.",
    ));
    report
}

fn managed_filesystem_paths(app: &AppHandle, warnings: &mut Vec<String>) -> Vec<PathBuf> {
    let cache = match app.path().app_cache_dir() {
        Ok(path) => path,
        Err(error) => {
            warnings.push(format!("Could not resolve app cache directory: {error}"));
            return Vec::new();
        }
    };
    let mut paths = vec![
        cache.join(SOURCE_DIRECTORY),
        cache.join(OUTPUT_DIRECTORY),
        cache.join(GRADLE_DIRECTORY),
        cache.join(ANDROID_STAGING_DIRECTORY),
        std::env::temp_dir().join(MACOS_BUILD_DIRECTORY),
    ];
    match crate::settings::manifest_path(app) {
        Ok(path) => paths.push(path),
        Err(error) => warnings.push(format!("Could not resolve output manifest: {error}")),
    }
    paths
        .into_iter()
        .filter(|path| !is_protected_reference(path))
        .filter(|path| path.exists())
        .collect()
}

fn managed_output_root(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_cache_dir()
        .ok()
        .map(|cache| cache.join(OUTPUT_DIRECTORY))
}

/// Paths outside ChaosLauncher's cache that Full Clean must never touch. The
/// reference tree under Downloads is user data; the Android root is device-side
/// external storage and is never mounted into a build container.
fn protected_reference_paths() -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::from(ANDROID_EXTERNAL_RSDK)];
    if let Some(home) = home_directory() {
        let downloads = home.join("Downloads");
        paths.push(downloads.join("Sonic"));
        paths.push(downloads.join("sonic"));
    }
    paths
}

fn is_protected_reference(path: &Path) -> bool {
    protected_reference_paths()
        .iter()
        .any(|protected| path == protected || path.starts_with(protected))
}

fn home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

async fn docker_targets() -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut containers = Vec::new();
    let mut images = Vec::new();
    let mut warnings = Vec::new();

    let docker = match connect_docker() {
        Ok(docker) => docker,
        Err(error) => {
            warnings.push(format!("Docker status unavailable: {error}"));
            return (containers, images, warnings);
        }
    };

    match docker
        .list_containers(Some(ListContainersOptions::<&str> {
            all: true,
            ..Default::default()
        }))
        .await
    {
        Ok(summary) => {
            for container in summary {
                let names = container.names.unwrap_or_default();
                for name in names {
                    let name = name.trim_start_matches('/');
                    if name.starts_with(CONTAINER_PREFIX) {
                        push_unique(&mut containers, name.to_owned());
                    }
                }
            }
        }
        Err(error) => warnings.push(format!("Could not list containers: {error}")),
    }

    match docker.inspect_image(BUILDER_IMAGE).await {
        Ok(_) => images.push(BUILDER_IMAGE.to_owned()),
        Err(DockerApiError::DockerResponseServerError {
            status_code: 404, ..
        }) => {}
        Err(error) => warnings.push(format!("Could not inspect {BUILDER_IMAGE}: {error}")),
    }

    (containers, images, warnings)
}

fn remove_path(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(path).map_err(|error| error.to_string())
    } else if metadata.is_dir() {
        fs::remove_dir_all(path).map_err(|error| error.to_string())
    } else {
        Err("unsupported file type".to_owned())
    }
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_prefix_is_scoped() {
        assert!("chaoslauncher-build-123-456".starts_with(CONTAINER_PREFIX));
        assert!(!"other-container".starts_with(CONTAINER_PREFIX));
    }

    #[test]
    fn protected_references_cover_downloads_sonic_and_android_external_storage() {
        let home = home_directory().expect("test host should expose a home directory");
        assert!(is_protected_reference(&home.join("Downloads/Sonic/RSDK/v5/Data.rsdk")));
        assert!(is_protected_reference(&home.join("Downloads/sonic")));
        assert!(is_protected_reference(Path::new(
            "/storage/emulated/0/RSDK/v5/mods/modconfig.ini"
        )));
        assert!(!is_protected_reference(&home.join(".cache/com.chaoslauncher.app/outputs")));
        assert!(!is_protected_reference(&std::env::temp_dir().join("chaoslauncher-macos-build")));
    }
}

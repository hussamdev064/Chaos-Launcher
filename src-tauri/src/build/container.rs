//! Docker container lifecycle for a build: create with bind mounts, attach,
//! stream logs, wait for the exit code, and always remove the container.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bollard::container::{
    AttachContainerOptions, Config, CreateContainerOptions, RemoveContainerOptions,
    StartContainerOptions, WaitContainerOptions,
};
use bollard::models::HostConfig;
use bollard::Docker;
use futures_util::StreamExt;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use super::script::BuildPaths;
use super::{BuildError, BuildEvent, EventSink};
use crate::docker::BUILDER_IMAGE;

const CONTAINER_SOURCE: &str = "/work/source";
const CONTAINER_DATA: &str = "/work/Data.rsdk";
const CONTAINER_BUILD: &str = "/work/build";
const CONTAINER_OUTPUT: &str = "/work/out";
const CONTAINER_GRADLE_CACHE: &str = "/root/.gradle";

/// Host paths mounted into the container. Source and asset are read-only so a
/// build can never modify the user's checkout or their legally obtained asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerMounts {
    pub source_host: PathBuf,
    pub data_host: PathBuf,
    pub output_host: PathBuf,
}

pub fn container_paths() -> BuildPaths {
    BuildPaths {
        source: PathBuf::from(CONTAINER_SOURCE),
        data: PathBuf::from(CONTAINER_DATA),
        build: PathBuf::from(CONTAINER_BUILD),
        output: PathBuf::from(CONTAINER_OUTPUT),
    }
}

/// Gradle downloads live next to the other app cache data so Android builds do
/// not repeat large wrapper/Maven downloads in every ephemeral container.
fn gradle_cache_host(output_host: &Path) -> PathBuf {
    output_host
        .parent()
        .and_then(Path::parent)
        .unwrap_or(output_host)
        .join("gradle-cache")
}

pub async fn run_container(
    script: &str,
    mounts: &ContainerMounts,
    on_event: &EventSink,
) -> Result<(), BuildError> {
    let docker = Docker::connect_with_local_defaults()
        .map_err(|error| BuildError::Docker(error.to_string()))?
        // Builds can legitimately run far longer than bollard's 2-minute default.
        .with_timeout(Duration::from_secs(6 * 60 * 60));

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let name = format!("chaoslauncher-build-{}-{timestamp}", std::process::id());

    let options = CreateContainerOptions {
        name: name.clone(),
        platform: None,
    };

    let gradle_cache = gradle_cache_host(&mounts.output_host);
    tokio::fs::create_dir_all(&gradle_cache)
        .await
        .map_err(|error| BuildError::Io(error.to_string()))?;

    let binds = vec![
        format!("{}:{CONTAINER_SOURCE}:ro", mounts.source_host.display()),
        format!("{}:{CONTAINER_DATA}:ro", mounts.data_host.display()),
        format!("{}:{CONTAINER_OUTPUT}:rw", mounts.output_host.display()),
        format!("{}:{CONTAINER_GRADLE_CACHE}:rw", gradle_cache.display()),
    ];

    let config = Config::<&str> {
        image: Some(BUILDER_IMAGE),
        cmd: Some(vec!["/bin/bash", "-lc", script]),
        host_config: Some(HostConfig {
            binds: Some(binds),
            auto_remove: Some(false),
            ..Default::default()
        }),
        ..Default::default()
    };

    let created = docker
        .create_container(Some(options), config)
        .await
        .map_err(|error| BuildError::Docker(error.to_string()))?;
    let container_id = created.id;

    // Capture attach/stream failures so cleanup still runs (no early return).
    let outcome = async {
        // Attach before starting so no build output is missed.
        let attach_options = AttachContainerOptions::<String> {
            stdin: Some(false),
            stdout: Some(true),
            stderr: Some(true),
            stream: Some(true),
            logs: Some(true),
            detach_keys: Some("ctrl-c".to_owned()),
        };
        let attached = docker
            .attach_container(&container_id, Some(attach_options))
            .await
            .map_err(|error| BuildError::Docker(error.to_string()))?;

        stream_build(&docker, &container_id, attached.output, on_event).await
    }
    .await;

    let removal = docker
        .remove_container(
            &container_id,
            Some(RemoveContainerOptions {
                force: true,
                v: true,
                ..Default::default()
            }),
        )
        .await;

    outcome?;
    removal.map_err(|error| BuildError::Docker(error.to_string()))
}

async fn stream_build(
    docker: &Docker,
    container_id: &str,
    output: std::pin::Pin<
        Box<
            dyn futures_util::stream::Stream<
                    Item = Result<bollard::container::LogOutput, bollard::errors::Error>,
                > + Send,
        >,
    >,
    on_event: &EventSink,
) -> Result<(), BuildError> {
    docker
        .start_container(container_id, None::<StartContainerOptions<String>>)
        .await
        .map_err(|error| BuildError::Docker(error.to_string()))?;

    let mut output = output;
    let mut tail: Vec<String> = Vec::new();
    while let Some(chunk) = output.next().await {
        let chunk = chunk.map_err(|error| BuildError::Docker(error.to_string()))?;
        for line in String::from_utf8_lossy(chunk.as_ref()).lines() {
            handle_log_line(line, on_event, &mut tail);
        }
    }

    // The attach stream closes when the process exits; wait_container returns its code.
    // Box::pin keeps the response stream usable with StreamExt::next().
    let mut statuses = Box::pin(docker.wait_container(container_id, None::<WaitContainerOptions<String>>));
    let status = match statuses.next().await {
        Some(Ok(response)) => response.status_code,
        Some(Err(error)) => return Err(BuildError::Docker(error.to_string())),
        None => return Err(BuildError::Docker("no exit status from container".to_owned())),
    };

    if status != 0 {
        return Err(BuildError::BuildFailed { status, tail });
    }
    Ok(())
}

/// macOS builds run on the host because Apple SDK rules forbid reliable Linux
/// cross-compilation. The generated script is identical; only paths differ.
pub async fn run_native(script: &str, on_event: &EventSink) -> Result<(), BuildError> {
    let mut child = Command::new("/bin/sh")
        .args(["-c", script])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| BuildError::Io(format!("cannot start native build: {error}")))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| BuildError::Io("native build has no stdout pipe".to_owned()))?;
    let (sender, mut receiver) = mpsc::channel::<String>(64);

    let reader = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|error| BuildError::Io(error.to_string()))?
        {
            if sender.send(line).await.is_err() {
                break;
            }
        }
        Ok::<(), BuildError>(())
    });

    let mut tail: Vec<String> = Vec::new();
    while let Some(line) = receiver.recv().await {
        handle_log_line(&line, on_event, &mut tail);
    }

    let status = child
        .wait()
        .await
        .map_err(|error| BuildError::Io(error.to_string()))?;
    reader
        .await
        .map_err(|error| BuildError::Io(error.to_string()))??;

    let code = status.code().unwrap_or(-1);
    if code != 0 {
        return Err(BuildError::BuildFailed { status: i64::from(code), tail });
    }
    Ok(())
}

/// Extract `::progress::NN` / `::stage::text` markers and forward everything
/// else as terminal log lines. Keep a short tail for error messages.
fn handle_log_line(line: &str, on_event: &EventSink, tail: &mut Vec<String>) {
    let line = line.trim_end();
    if line.is_empty() {
        return;
    }

    if let Some(percent) = line.strip_prefix("::progress::") {
        if let Ok(value) = percent.parse::<u8>() {
            on_event(BuildEvent {
                percent: Some(value),
                message: String::new(),
            });
            return;
        }
    }
    if let Some(stage) = line.strip_prefix("::stage::") {
        on_event(BuildEvent {
            percent: None,
            message: stage.to_owned(),
        });
        return;
    }

    tail.push(line.to_owned());
    if tail.len() > 12 {
        tail.remove(0);
    }
    on_event(BuildEvent {
        percent: None,
        message: line.to_owned(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradle_cache_is_stored_beside_outputs() {
        let output = PathBuf::from("/tmp/app-cache/outputs/android-arm64-apk-1");
        assert_eq!(
            gradle_cache_host(&output),
            PathBuf::from("/tmp/app-cache/gradle-cache")
        );
    }
}

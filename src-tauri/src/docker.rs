//! Docker bootstrap for the builder image.
//!
//! The Dockerfile and MinGW toolchain file are embedded at compile time, so a
//! packaged app can rebuild the image without needing the source checkout.
//! The build context is assembled in memory as a tar archive and handed to the
//! Docker Engine API through `bollard`; no `docker` CLI is required.

use std::fmt;

use bollard::errors::Error as DockerApiError;
use bollard::image::BuildImageOptions;
use bollard::Docker;
use futures_util::StreamExt;

/// Tag inspected and built on startup.
pub const BUILDER_IMAGE: &str = "sonic-mania-builder:latest";

const DOCKERFILE: &str = include_str!("../../Dockerfile");
const MINGW_TOOLCHAIN: &str = include_str!("../../docker/mingw-x86_64.cmake");
const LIBTHEORA_SOURCE: &[u8] = include_bytes!("../../vendor/libtheora-1.1.1.tar.gz");
const LIBOGG_SOURCE: &[u8] = include_bytes!("../../vendor/libogg-1.3.5.tar.gz");

/// Result of the startup check, used by the UI to choose its status message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageStatus {
    AlreadyPresent,
    Built,
}

/// All failures are values; the bootstrap never panics on a missing daemon or a failed build.
#[derive(Debug)]
pub enum BootstrapError {
    Daemon(String),
    Context(String),
    Build(String),
}

impl fmt::Display for BootstrapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Daemon(message) => write!(formatter, "Docker daemon error: {message}"),
            Self::Context(message) => write!(formatter, "Docker build context error: {message}"),
            Self::Build(message) => write!(formatter, "Builder image build failed: {message}"),
        }
    }
}

impl std::error::Error for BootstrapError {}

/// Check the image, building it from the embedded Dockerfile when absent.
/// `on_stage` lets the caller emit UI events ("checking" then "building").
pub async fn ensure_builder_image(
    on_stage: impl Fn(&'static str),
) -> Result<ImageStatus, BootstrapError> {
    on_stage("checking");
    if image_exists().await? {
        return Ok(ImageStatus::AlreadyPresent);
    }

    on_stage("building");
    build_image().await?;
    Ok(ImageStatus::Built)
}

/// A 404 from `inspect_image` means the tag is absent and should be built.
pub async fn image_exists() -> Result<bool, BootstrapError> {
    let docker = connect()?;
    docker
        .ping()
        .await
        .map_err(|error| BootstrapError::Daemon(error.to_string()))?;

    match docker.inspect_image(BUILDER_IMAGE).await {
        Ok(_) => Ok(true),
        Err(DockerApiError::DockerResponseServerError { status_code: 404, .. }) => Ok(false),
        Err(error) => Err(BootstrapError::Daemon(error.to_string())),
    }
}

/// Stream the build response to completion; Docker reports failures inside the stream,
/// so a successful HTTP stream is not enough to conclude the image was built.
pub async fn build_image() -> Result<(), BootstrapError> {
    let docker = connect()?;
    let context = build_context()?;

    let options: BuildImageOptions<&str> = BuildImageOptions {
        dockerfile: "Dockerfile",
        t: BUILDER_IMAGE,
        rm: true,
        ..Default::default()
    };

    let mut updates = docker.build_image(options, None, Some(context.into()));
    while let Some(update) = updates.next().await {
        let update = update.map_err(|error| BootstrapError::Daemon(error.to_string()))?;
        if let Some(message) = update.error {
            return Err(BootstrapError::Build(message));
        }
        if let Some(detail) = update.error_detail {
            return Err(BootstrapError::Build(
                detail
                    .message
                    .unwrap_or_else(|| "Docker image build failed.".to_owned()),
            ));
        }
    }

    Ok(())
}

fn connect() -> Result<Docker, BootstrapError> {
    Docker::connect_with_local_defaults().map_err(|error| BootstrapError::Daemon(error.to_string()))
}

/// Docker's build endpoint accepts a tar archive as its body. Keep the files in
/// memory so no temporary directory is needed and no user data is touched.
fn build_context() -> Result<Vec<u8>, BootstrapError> {
    let mut archive = tar::Builder::new(Vec::new());
    append_context_file(&mut archive, "Dockerfile", DOCKERFILE.as_bytes())?;
    append_context_file(
        &mut archive,
        "docker/mingw-x86_64.cmake",
        MINGW_TOOLCHAIN.as_bytes(),
    )?;
    // The embedded Dockerfile stages these sources for Android builds. A
    // packaged app has no checkout, so they must travel inside the tar context.
    append_context_file(
        &mut archive,
        "vendor/libtheora-1.1.1.tar.gz",
        LIBTHEORA_SOURCE,
    )?;
    append_context_file(&mut archive, "vendor/libogg-1.3.5.tar.gz", LIBOGG_SOURCE)?;
    archive
        .into_inner()
        .map_err(|error| BootstrapError::Context(error.to_string()))
}

fn append_context_file(
    archive: &mut tar::Builder<Vec<u8>>,
    path: &str,
    contents: &[u8],
) -> Result<(), BootstrapError> {
    let mut header = tar::Header::new_gnu();
    header.set_size(contents.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(0);
    header.set_cksum();
    archive
        .append_data(&mut header, path, contents)
        .map_err(|error| BootstrapError::Context(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_context_contains_every_embedded_dependency() {
        let bytes = build_context().expect("embedded files should form a valid tar archive");
        let mut archive = tar::Archive::new(bytes.as_slice());
        let names: Vec<String> = archive
            .entries()
            .expect("tar entries should be readable")
            .map(|entry| {
                entry
                    .expect("tar entry should be readable")
                    .path()
                    .expect("entry path should be valid")
                    .display()
                    .to_string()
            })
            .collect();

        for required in [
            "Dockerfile",
            "docker/mingw-x86_64.cmake",
            "vendor/libtheora-1.1.1.tar.gz",
            "vendor/libogg-1.3.5.tar.gz",
        ] {
            assert!(names.iter().any(|name| name == required), "missing {required}");
        }

        // Guard against future Dockerfile COPYs being omitted from the context.
        for line in DOCKERFILE.lines() {
            if let Some(rest) = line.trim().strip_prefix("COPY vendor/") {
                let source = rest
                    .split_whitespace()
                    .next()
                    .map(|name| format!("vendor/{name}"))
                    .expect("COPY line should name a source");
                assert!(names.iter().any(|name| name == &source), "missing {source}");
            }
        }
    }
}

//! Clone and maintain the upstream Sonic Mania source in the app cache.
//!
//! Upstream must be cloned recursively: the RSDKv5 engine lives in a submodule,
//! so a GitHub ZIP archive would not contain everything needed to build.

use std::cell::Cell;
use std::fmt;
use std::path::{Path, PathBuf};

pub const SOURCE_URL: &str = "https://github.com/RSDKModding/Sonic-Mania-Decompilation.git";
pub const CHECKOUT_DIRECTORY: &str = "Sonic-Mania-Decompilation";

/// Required after a recursive clone; Phase 4 builds from these paths.
const REQUIRED_ROOT_FILE: &str = "CMakeLists.txt";
const REQUIRED_SUBMODULE_DIR: &str = "dependencies/RSDKv5";

#[derive(Debug)]
pub enum SourceError {
    Io(String),
    Git(String),
    Layout(String),
    Task(String),
}

impl fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(formatter, "Filesystem error: {message}"),
            Self::Git(message) => write!(formatter, "Git error: {message}"),
            Self::Layout(message) => write!(formatter, "Unexpected source layout: {message}"),
            Self::Task(message) => write!(formatter, "Background task failed: {message}"),
        }
    }
}

impl std::error::Error for SourceError {}

/// Cache path for the checkout, e.g. `<app cache>/Sonic-Mania-Decompilation`.
pub fn source_path(cache_directory: &Path) -> PathBuf {
    cache_directory.join(CHECKOUT_DIRECTORY)
}

/// Clone on first use; later runs only refresh submodules so a user's local
/// source tweaks are never silently overwritten.
///
/// `git2` is blocking, so the whole operation runs on a Tokio blocking thread.
pub async fn prepare_source(
    cache_directory: &Path,
    on_progress: impl Fn(&str) + Send + 'static,
) -> Result<PathBuf, SourceError> {
    let cache_directory = cache_directory.to_path_buf();

    tokio::task::spawn_blocking(move || {
        let checkout = source_path(&cache_directory);

        if checkout.join(".git").is_dir() {
            on_progress("Existing source checkout found; updating submodules...");
            update_submodules(&checkout)?;
        } else {
            on_progress("Cloning Sonic-Mania-Decompilation...");
            std::fs::create_dir_all(&cache_directory)
                .map_err(|error| SourceError::Io(error.to_string()))?;
            clone_repository(&checkout, &on_progress)?;
        }

        validate_layout(&checkout)?;
        on_progress("Source is ready.");
        Ok(checkout)
    })
    .await
    .map_err(|error| SourceError::Task(error.to_string()))?
}

fn clone_repository(
    checkout: &Path,
    on_progress: &(dyn Fn(&str) + Send),
) -> Result<(), SourceError> {
    let mut callbacks = git2::RemoteCallbacks::new();
    // git2 invokes this for every pack update; emit only changed counts so the
    // UI receives useful progress instead of tens of thousands of duplicates.
    let last_received = Cell::new(0);
    callbacks.transfer_progress(move |progress| {
        let received = progress.received_objects();
        if received > 0 && received != last_received.get() {
            last_received.set(received);
            on_progress(&format!(
                "Received {}/{} objects",
                received,
                progress.total_objects()
            ));
        }
        true
    });

    let mut fetch_options = git2::FetchOptions::new();
    fetch_options.remote_callbacks(callbacks);

    let mut builder = git2::build::RepoBuilder::new();
    builder.fetch_options(fetch_options);
    builder.clone(SOURCE_URL, checkout).map_err(|error| {
        SourceError::Git(format!("failed to clone {SOURCE_URL}: {error}"))
    })?;

    update_submodules(checkout)
}

/// `submodule.update(true, ...)` both initializes missing submodules and updates
/// existing ones. Recurse manually because `git2` does not do nested updates.
fn update_submodules(checkout: &Path) -> Result<(), SourceError> {
    update_submodules_at(checkout)
}

fn update_submodules_at(repository_path: &Path) -> Result<(), SourceError> {
    let repository = git2::Repository::open(repository_path).map_err(|error| {
        SourceError::Git(format!("cannot open {}: {error}", repository_path.display()))
    })?;

    for mut submodule in repository
        .submodules()
        .map_err(|error| SourceError::Git(error.to_string()))?
    {
        let name = submodule.name().unwrap_or("?").to_owned();

        // `.gitmodules` can outlive a gitlink. Skip stale entries that are not
        // tracked by HEAD or the index; otherwise update() creates an empty
        // directory and recursion later fails trying to open it as a repo.
        if submodule.head_id().is_none() && submodule.index_id().is_none() {
            continue;
        }

        let fetch_options = git2::FetchOptions::new();
        let mut update_options = git2::SubmoduleUpdateOptions::new();
        update_options.fetch(fetch_options);

        submodule
            .update(true, Some(&mut update_options))
            .map_err(|error| SourceError::Git(format!("submodule '{name}': {error}")))?;

        if let Some(relative_path) = submodule.path().to_str() {
            let nested = repository_path.join(relative_path);
            // Only recurse into an initialized repository; some nested gitlinks
            // may be intentionally absent for the target revision.
            if git2::Repository::open(&nested).is_ok() {
                update_submodules_at(&nested)?;
            }
        }
    }

    Ok(())
}

fn validate_layout(checkout: &Path) -> Result<(), SourceError> {
    if !checkout.join(REQUIRED_ROOT_FILE).is_file() {
        return Err(SourceError::Layout(format!(
            "missing {REQUIRED_ROOT_FILE} in {}",
            checkout.display()
        )));
    }
    if !checkout.join(REQUIRED_SUBMODULE_DIR).is_dir() {
        return Err(SourceError::Layout(format!(
            "missing {REQUIRED_SUBMODULE_DIR}; the clone must be recursive"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkout_path_is_inside_cache() {
        let cache = Path::new("/tmp/chaos-cache");
        assert_eq!(
            source_path(cache),
            PathBuf::from("/tmp/chaos-cache/Sonic-Mania-Decompilation")
        );
    }
}

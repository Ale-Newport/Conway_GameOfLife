//! Locating the asset folder.
//!
//! Order: `$EMERGENCE_LAB_ASSETS`, then `<executable dir>/assets` (packaged
//! builds), then the workspace's `assets/` (`cargo run -p emergence-lab`
//! from anywhere, or the binary started from `target/`). The result is
//! absolute and handed to Bevy's `AssetPlugin::file_path`, which uses
//! absolute paths as given.

use std::path::{Path, PathBuf};

use thiserror::Error;

/// Environment variable naming the asset folder.
pub const ASSETS_ENV: &str = "EMERGENCE_LAB_ASSETS";

/// Where the asset folder was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetSource {
    /// `$EMERGENCE_LAB_ASSETS`.
    Environment,
    /// Next to the executable.
    ExecutableDir,
    /// The workspace checkout this binary was built from.
    Workspace,
}

/// Asset folder resolution failures.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum AssetRootError {
    /// The environment variable points to something that is not a folder.
    #[error("{ASSETS_ENV} is set to `{}`, which is not a folder", .0.display())]
    BadEnvironment(PathBuf),
    /// No candidate exists.
    #[error("no asset folder found; tried: {}", .0.iter().map(|p| format!("`{}`", p.display())).collect::<Vec<_>>().join(", "))]
    NotFound(Vec<PathBuf>),
}

/// The workspace `assets/` folder as seen from this crate's manifest.
pub fn workspace_assets() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("assets")
}

/// Picks the asset folder from the candidates; `is_dir` is injectable for
/// tests.
pub fn resolve(
    env_value: Option<PathBuf>,
    exe_dir: Option<PathBuf>,
    workspace: PathBuf,
    is_dir: impl Fn(&Path) -> bool,
) -> Result<(PathBuf, AssetSource), AssetRootError> {
    if let Some(path) = env_value.filter(|p| !p.as_os_str().is_empty()) {
        return if is_dir(&path) {
            Ok((path, AssetSource::Environment))
        } else {
            Err(AssetRootError::BadEnvironment(path))
        };
    }
    let mut tried = Vec::new();
    if let Some(exe_dir) = exe_dir {
        let candidate = exe_dir.join("assets");
        if is_dir(&candidate) {
            return Ok((candidate, AssetSource::ExecutableDir));
        }
        tried.push(candidate);
    }
    if is_dir(&workspace) {
        return Ok((workspace, AssetSource::Workspace));
    }
    tried.push(workspace);
    Err(AssetRootError::NotFound(tried))
}

/// Resolves the asset folder for this process and makes it absolute.
pub fn resolve_for_process() -> Result<(PathBuf, AssetSource), AssetRootError> {
    let env_value = std::env::var_os(ASSETS_ENV).map(PathBuf::from);
    let exe_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf));
    let (path, source) = resolve(env_value, exe_dir, workspace_assets(), Path::is_dir)?;
    Ok((std::fs::canonicalize(&path).unwrap_or(path), source))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(existing: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        move |p: &Path| existing.iter().any(|e| Path::new(e) == p)
    }

    #[test]
    fn precedence() {
        let ws = PathBuf::from("/ws/assets");
        let exe = Some(PathBuf::from("/opt/lab"));
        assert_eq!(
            resolve(Some("/env".into()), exe.clone(), ws.clone(), dirs(&["/env", "/opt/lab/assets", "/ws/assets"])),
            Ok(("/env".into(), AssetSource::Environment))
        );
        assert_eq!(
            resolve(None, exe.clone(), ws.clone(), dirs(&["/opt/lab/assets", "/ws/assets"])),
            Ok(("/opt/lab/assets".into(), AssetSource::ExecutableDir))
        );
        assert_eq!(
            resolve(None, exe.clone(), ws.clone(), dirs(&["/ws/assets"])),
            Ok((ws.clone(), AssetSource::Workspace))
        );
        assert_eq!(
            resolve(Some("".into()), None, ws.clone(), dirs(&["/ws/assets"])),
            Ok((ws.clone(), AssetSource::Workspace)),
            "an empty variable is ignored"
        );
    }

    #[test]
    fn failures_name_the_paths() {
        let ws = PathBuf::from("/ws/assets");
        assert_eq!(
            resolve(Some("/nope".into()), None, ws.clone(), dirs(&["/ws/assets"])),
            Err(AssetRootError::BadEnvironment("/nope".into()))
        );
        let err = resolve(None, Some("/opt/lab".into()), ws, dirs(&[])).unwrap_err();
        assert_eq!(err, AssetRootError::NotFound(vec!["/opt/lab/assets".into(), "/ws/assets".into()]));
        assert!(err.to_string().contains("/opt/lab/assets"));
    }

    #[test]
    fn workspace_assets_exist_in_this_checkout() {
        assert!(workspace_assets().join("patterns").is_dir());
    }
}

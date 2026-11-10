//! Locating the asset and scenario folders and building the registry.

use std::path::{Path, PathBuf};

use anyhow::bail;
use model_registry::{Registry, RegistryConfig};
use persistence::UserDataDirs;

use crate::cli::Cli;

/// Environment variable naming the asset folder (shared with the desktop app).
pub const ASSETS_ENV: &str = "EMERGENCE_LAB_ASSETS";

/// The workspace this binary was built from.
fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Asset folder: `--assets`, `$EMERGENCE_LAB_ASSETS`, `<exe dir>/assets`,
/// then the workspace `assets/`. `None` when none exists.
///
/// # Errors
/// An explicit folder (option or environment variable) that does not exist.
fn asset_root(explicit: Option<&Path>) -> anyhow::Result<Option<PathBuf>> {
    let named = match explicit {
        Some(path) => Some((path.to_path_buf(), "--assets")),
        None => std::env::var_os(ASSETS_ENV).filter(|v| !v.is_empty()).map(|v| (PathBuf::from(v), ASSETS_ENV)),
    };
    if let Some((path, source)) = named {
        if !path.is_dir() {
            bail!("{source} names `{}`, which is not a folder", path.display());
        }
        return Ok(Some(path));
    }
    let exe_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|p| p.join("assets")));
    Ok(exe_dir.into_iter().chain([workspace().join("assets")]).find(|p| p.is_dir()))
}

/// Scenario directories in increasing priority.
fn scenario_dirs(cli: &Cli) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if !cli.builtin_only {
        let repo = workspace().join("scenarios");
        if repo.is_dir() {
            dirs.push(repo);
        }
        if let Ok(user) = UserDataDirs::resolve() {
            dirs.push(user.scenarios());
        }
    }
    dirs.extend(cli.scenario_dirs.iter().cloned());
    dirs.into_iter().map(|d| std::fs::canonicalize(&d).unwrap_or(d)).collect()
}

/// The registry for this invocation.
///
/// # Errors
/// See [`asset_root`].
pub fn registry(cli: &Cli) -> anyhow::Result<Registry> {
    let assets = asset_root(cli.assets.as_deref())?.map(|p| std::fs::canonicalize(&p).unwrap_or(p));
    Ok(Registry::new(RegistryConfig { assets, scenario_dirs: scenario_dirs(cli) }))
}

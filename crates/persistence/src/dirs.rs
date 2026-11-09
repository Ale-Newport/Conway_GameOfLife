//! Per-user data folders.
//!
//! | Platform | Root |
//! |---|---|
//! | any (override) | `$EMERGENCE_LAB_DATA` |
//! | macOS | `~/Library/Application Support/EmergenceLab` |
//! | Linux / other Unix | `$XDG_DATA_HOME/emergence-lab`, else `~/.local/share/emergence-lab` |
//! | Windows | `%APPDATA%\EmergenceLab`, else `%USERPROFILE%\AppData\Roaming\EmergenceLab` |
//!
//! Following the XDG Base Directory specification, a relative
//! `XDG_DATA_HOME` is ignored. Only environment variables are consulted
//! (`std::env`), so the result is predictable and testable.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::envelope::FileKind;
use crate::error::PersistenceError;

/// Environment variable overriding the data root.
pub const DATA_DIR_ENV: &str = "EMERGENCE_LAB_DATA";

/// Operating-system family, for choosing the default location.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Platform {
    /// macOS.
    MacOs,
    /// Windows.
    Windows,
    /// Linux and other Unix-like systems (XDG conventions).
    Unix,
}

impl Platform {
    /// Platform this build targets.
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Unix
        }
    }
}

/// Standard subfolder of the data root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DataFolder {
    /// Saved simulations and snapshot histories (`saves/`).
    Saves,
    /// User scenarios, rule sets and patterns (`scenarios/`).
    Scenarios,
    /// CSV / JSON exports and experiment results (`exports/`).
    Exports,
    /// Screenshots (`screenshots/`).
    Screenshots,
}

impl DataFolder {
    /// Every folder.
    pub const ALL: [DataFolder; 4] =
        [DataFolder::Saves, DataFolder::Scenarios, DataFolder::Exports, DataFolder::Screenshots];

    /// Folder name.
    pub fn name(self) -> &'static str {
        match self {
            DataFolder::Saves => "saves",
            DataFolder::Scenarios => "scenarios",
            DataFolder::Exports => "exports",
            DataFolder::Screenshots => "screenshots",
        }
    }

    /// Default folder for documents of `kind` (`None` for settings, which
    /// live at the root: [`UserDataDirs::settings_file`]).
    pub fn for_kind(kind: FileKind) -> Option<Self> {
        match kind {
            FileKind::Snapshot | FileKind::SnapshotHistory => Some(DataFolder::Saves),
            FileKind::Scenario | FileKind::RuleSet | FileKind::Pattern => Some(DataFolder::Scenarios),
            FileKind::Experiment => Some(DataFolder::Exports),
            FileKind::Settings => None,
        }
    }
}

/// Resolved per-user data root and its standard subfolders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserDataDirs {
    root: PathBuf,
}

fn non_empty(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|v| !v.is_empty()).map(PathBuf::from)
}

impl UserDataDirs {
    /// Uses `root` directly (tests, portable installations).
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Resolves the root for the current platform from the process
    /// environment.
    pub fn resolve() -> Result<Self, PersistenceError> {
        Self::resolve_with(Platform::current(), |name: &str| std::env::var_os(name))
    }

    /// Resolves the root for `platform` using `var` to read environment
    /// variables (injectable for tests).
    pub fn resolve_with(platform: Platform, var: impl Fn(&str) -> Option<OsString>) -> Result<Self, PersistenceError> {
        if let Some(root) = non_empty(var(DATA_DIR_ENV)) {
            return Ok(Self::at(root));
        }
        let home = || {
            non_empty(var("HOME")).ok_or_else(|| {
                PersistenceError::DataDir(format!("HOME is not set; set {DATA_DIR_ENV} to choose a data folder"))
            })
        };
        let root = match platform {
            Platform::MacOs => home()?.join("Library").join("Application Support").join("EmergenceLab"),
            Platform::Unix => match non_empty(var("XDG_DATA_HOME")).filter(|p| p.is_absolute()) {
                Some(xdg) => xdg.join("emergence-lab"),
                None => home()?.join(".local").join("share").join("emergence-lab"),
            },
            Platform::Windows => match non_empty(var("APPDATA")) {
                Some(appdata) => appdata.join("EmergenceLab"),
                None => non_empty(var("USERPROFILE"))
                    .ok_or_else(|| {
                        PersistenceError::DataDir(format!(
                            "neither APPDATA nor USERPROFILE is set; set {DATA_DIR_ENV} to choose a data folder"
                        ))
                    })?
                    .join("AppData")
                    .join("Roaming")
                    .join("EmergenceLab"),
            },
        };
        Ok(Self::at(root))
    }

    /// The data root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Path of a standard subfolder.
    pub fn folder(&self, folder: DataFolder) -> PathBuf {
        self.root.join(folder.name())
    }

    /// `saves/`.
    pub fn saves(&self) -> PathBuf {
        self.folder(DataFolder::Saves)
    }

    /// `scenarios/`.
    pub fn scenarios(&self) -> PathBuf {
        self.folder(DataFolder::Scenarios)
    }

    /// `exports/`.
    pub fn exports(&self) -> PathBuf {
        self.folder(DataFolder::Exports)
    }

    /// `screenshots/`.
    pub fn screenshots(&self) -> PathBuf {
        self.folder(DataFolder::Screenshots)
    }

    /// Path of the settings file (`settings.toml` at the root).
    pub fn settings_file(&self) -> PathBuf {
        self.root.join("settings.toml")
    }

    /// Creates the root and every standard subfolder if missing.
    pub fn ensure(&self) -> Result<(), PersistenceError> {
        for folder in DataFolder::ALL {
            let path = self.folder(folder);
            fs::create_dir_all(&path)
                .map_err(|e| PersistenceError::at(&path, PersistenceError::io("create the folder", e)))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atomic::test_dir::TestDir;
    use std::collections::BTreeMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: BTreeMap<String, OsString> = pairs.iter().map(|(k, v)| (k.to_string(), OsString::from(v))).collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn platform_defaults() {
        let mac = UserDataDirs::resolve_with(Platform::MacOs, env(&[("HOME", "/Users/ada")])).unwrap();
        assert_eq!(mac.root(), Path::new("/Users/ada/Library/Application Support/EmergenceLab"));
        let linux = UserDataDirs::resolve_with(Platform::Unix, env(&[("HOME", "/home/ada")])).unwrap();
        assert_eq!(linux.root(), Path::new("/home/ada/.local/share/emergence-lab"));
        let xdg = UserDataDirs::resolve_with(Platform::Unix, env(&[("HOME", "/home/ada"), ("XDG_DATA_HOME", "/data")]))
            .unwrap();
        assert_eq!(xdg.root(), Path::new("/data/emergence-lab"));
        let windows =
            UserDataDirs::resolve_with(Platform::Windows, env(&[("APPDATA", "C:/Users/ada/AppData/Roaming")])).unwrap();
        assert_eq!(windows.root(), Path::new("C:/Users/ada/AppData/Roaming").join("EmergenceLab"));
        let profile = UserDataDirs::resolve_with(Platform::Windows, env(&[("USERPROFILE", "C:/Users/ada")])).unwrap();
        assert_eq!(profile.root(), Path::new("C:/Users/ada").join("AppData").join("Roaming").join("EmergenceLab"));
    }

    #[test]
    fn override_and_xdg_rules() {
        for platform in [Platform::MacOs, Platform::Unix, Platform::Windows] {
            let dirs =
                UserDataDirs::resolve_with(platform, env(&[(DATA_DIR_ENV, "/portable/data"), ("HOME", "/h")])).unwrap();
            assert_eq!(dirs.root(), Path::new("/portable/data"));
        }
        // Empty override is ignored; relative XDG_DATA_HOME is ignored.
        let dirs = UserDataDirs::resolve_with(
            Platform::Unix,
            env(&[(DATA_DIR_ENV, ""), ("HOME", "/h"), ("XDG_DATA_HOME", "rel")]),
        )
        .unwrap();
        assert_eq!(dirs.root(), Path::new("/h/.local/share/emergence-lab"));
        let err = UserDataDirs::resolve_with(Platform::MacOs, env(&[])).unwrap_err();
        assert!(err.to_string().contains(DATA_DIR_ENV));
        assert!(UserDataDirs::resolve_with(Platform::Windows, env(&[])).is_err());
    }

    #[test]
    fn subfolders_are_created() {
        let tmp = TestDir::new("dirs");
        let dirs = UserDataDirs::at(tmp.path().join("EmergenceLab"));
        dirs.ensure().unwrap();
        for (path, name) in [
            (dirs.saves(), "saves"),
            (dirs.scenarios(), "scenarios"),
            (dirs.exports(), "exports"),
            (dirs.screenshots(), "screenshots"),
        ] {
            assert!(path.is_dir());
            assert_eq!(path.file_name().unwrap(), name);
        }
        assert_eq!(dirs.settings_file(), dirs.root().join("settings.toml"));
        assert_eq!(DataFolder::for_kind(FileKind::Snapshot), Some(DataFolder::Saves));
        assert_eq!(DataFolder::for_kind(FileKind::Pattern), Some(DataFolder::Scenarios));
        assert_eq!(DataFolder::for_kind(FileKind::Experiment), Some(DataFolder::Exports));
        assert_eq!(DataFolder::for_kind(FileKind::Settings), None);
        // A file where a folder should be gives a clear error naming the path.
        let blocked = UserDataDirs::at(tmp.path().join("blocked"));
        std::fs::create_dir_all(blocked.root()).unwrap();
        std::fs::write(blocked.saves(), b"").unwrap();
        let err = blocked.ensure().unwrap_err();
        assert_eq!(err.path(), Some(blocked.saves().as_path()));
    }
}

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub theme: Theme,
    #[serde(default = "default_download_directory")]
    pub download_directory: PathBuf,
    #[serde(default)]
    pub installer_language: Option<String>,
    #[serde(default = "default_true")]
    pub installer_windows: bool,
    #[serde(default = "default_true")]
    pub installer_linux: bool,
    #[serde(default = "default_true")]
    pub installer_macos: bool,
    #[serde(default = "default_installation_source_order")]
    pub installation_source_order: Vec<PreferredInstallationSource>,
    #[serde(default = "default_true")]
    pub download_extras_by_default: bool,
    #[serde(default)]
    pub download_patches_by_default: bool,
    /// Prefer an exact GOG patch over a full installer for installed-game updates.
    /// Disabled by default because patch results cannot always be verified reliably.
    #[serde(default)]
    pub prefer_patch_updates: bool,
    #[serde(default)]
    pub interactive_installer_prompts: bool,
    #[serde(default)]
    pub interactive_installer_explanation_dismissed: bool,
    #[serde(default)]
    pub show_retired_artifacts: bool,
    #[serde(default = "default_max_concurrent_downloads")]
    pub max_concurrent_downloads: usize,
    #[serde(default)]
    pub download_bandwidth_limit_bps: Option<u64>,
    #[serde(default = "default_game_libraries")]
    pub game_libraries: Vec<GameLibrary>,
    #[serde(default)]
    pub installer_library_id: Option<String>,
    #[serde(default = "default_library_card_size")]
    pub library_card_size: u8,
    #[serde(default = "default_true")]
    pub show_sidebar_game_icons: bool,
    #[serde(default = "default_true")]
    pub show_backup_status: bool,
    #[serde(default)]
    pub sidebar_sort_mode: SidebarSortMode,
    #[serde(default = "default_window_width")]
    pub window_width: i32,
    #[serde(default = "default_window_height")]
    pub window_height: i32,
    #[serde(default)]
    pub window_maximized: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidebarSortMode {
    #[default]
    Alphabetical,
    LastPlayed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreferredInstallationSource {
    LinuxOffline,
    WindowsGalaxy,
    WindowsOffline,
}

pub fn default_installation_source_order() -> Vec<PreferredInstallationSource> {
    vec![
        PreferredInstallationSource::LinuxOffline,
        PreferredInstallationSource::WindowsGalaxy,
        PreferredInstallationSource::WindowsOffline,
    ]
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameLibrary {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    #[serde(default)]
    pub default: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Default for Config {
    fn default() -> Self {
        let game_libraries = default_game_libraries();
        let installer_library_id = game_libraries.first().map(|library| library.id.clone());
        let download_directory = game_libraries
            .first()
            .map(|library| library.path.clone())
            .unwrap_or_else(default_download_directory);
        Self {
            theme: Theme::System,
            download_directory,
            installer_language: None,
            installer_windows: true,
            installer_linux: true,
            installer_macos: true,
            installation_source_order: default_installation_source_order(),
            download_extras_by_default: true,
            download_patches_by_default: false,
            prefer_patch_updates: false,
            interactive_installer_prompts: false,
            interactive_installer_explanation_dismissed: false,
            show_retired_artifacts: false,
            max_concurrent_downloads: default_max_concurrent_downloads(),
            download_bandwidth_limit_bps: None,
            game_libraries,
            installer_library_id,
            library_card_size: default_library_card_size(),
            show_sidebar_game_icons: true,
            show_backup_status: true,
            sidebar_sort_mode: SidebarSortMode::Alphabetical,
            window_width: default_window_width(),
            window_height: default_window_height(),
            window_maximized: false,
        }
    }
}

const fn default_library_card_size() -> u8 {
    1
}

const fn default_window_width() -> i32 {
    1280
}

const fn default_window_height() -> i32 {
    800
}

impl Config {
    pub fn path() -> PathBuf {
        crate::identity::config_file()
    }

    pub fn load_or_create() -> Result<Self> {
        let path = Self::path();
        if path.exists() {
            let text =
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            let mut config: Self =
                toml::from_str(&text).context("parsing Ludomere configuration")?;
            config.max_concurrent_downloads = config.max_concurrent_downloads.clamp(1, 4);
            config.download_bandwidth_limit_bps = config
                .download_bandwidth_limit_bps
                .filter(|limit| *limit > 0);
            config.library_card_size = config.library_card_size.min(3);
            config.normalize_game_libraries();
            config.normalize_installation_source_order();
            write_config(&path, &config)?;
            return Ok(config);
        }

        let config = Self::default();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        write_config(&path, &config)?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        write_config(&path, self)
    }

    pub fn default_game_library(&self) -> Option<&GameLibrary> {
        self.game_libraries
            .iter()
            .find(|library| library.default)
            .or_else(|| self.game_libraries.first())
    }

    pub fn installer_library(&self) -> Option<&GameLibrary> {
        self.installer_library_id
            .as_deref()
            .and_then(|id| self.game_libraries.iter().find(|library| library.id == id))
            .or_else(|| self.default_game_library())
    }

    pub fn normalize_game_libraries(&mut self) {
        let mut paths = std::collections::HashSet::new();
        self.game_libraries
            .retain(|library| paths.insert(library.path.clone()));
        if self.game_libraries.is_empty() {
            self.game_libraries = default_game_libraries();
        }
        let default_index = self
            .game_libraries
            .iter()
            .position(|library| library.default)
            .unwrap_or(0);
        for (index, library) in self.game_libraries.iter_mut().enumerate() {
            library.default = index == default_index;
            if library.id.trim().is_empty() {
                library.id = game_library_id(&library.path);
            }
            if library.name.trim().is_empty() {
                library.name = library
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Games")
                    .to_owned();
            }
        }
        if self
            .installer_library_id
            .as_ref()
            .is_none_or(|id| !self.game_libraries.iter().any(|library| &library.id == id))
        {
            self.installer_library_id = self
                .default_game_library()
                .map(|library| library.id.clone());
        }
        if let Some(library) = self.installer_library() {
            self.download_directory = library.path.clone();
        }
    }

    pub fn normalize_installation_source_order(&mut self) {
        let mut normalized = Vec::new();
        for source in self
            .installation_source_order
            .iter()
            .copied()
            .chain(default_installation_source_order())
        {
            if !normalized.contains(&source) {
                normalized.push(source);
            }
        }
        self.installation_source_order = normalized;
    }
}

fn write_config(path: &std::path::Path, config: &Config) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("toml.part");
    fs::write(&temporary, toml::to_string_pretty(config)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&temporary, path)?;
    Ok(())
}

pub fn default_download_directory() -> PathBuf {
    crate::identity::data_root().join("downloads")
}

pub fn default_game_directory() -> PathBuf {
    crate::identity::data_root().join("games")
}

pub fn game_library_id(path: &std::path::Path) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(path.as_os_str().as_encoded_bytes());
    format!("library-{:x}", digest)[..24].to_owned()
}

fn default_game_libraries() -> Vec<GameLibrary> {
    let path = default_game_directory();
    vec![GameLibrary {
        id: game_library_id(&path),
        name: "Default".to_owned(),
        path,
        default: true,
    }]
}

fn default_true() -> bool {
    true
}

fn default_max_concurrent_downloads() -> usize {
    2
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn obsolete_roots_are_ignored_and_not_serialized() {
        let obsolete_key = ["library", "_roots"].concat();
        let old = format!(
            r#"
theme = "dark"
download_directory = "/tmp/gog-downloads"
installer_language = "English"
installer_windows = false
installer_linux = true
installer_macos = false
[[{obsolete_key}]]
path = "/old/archive"
name = "Old"
priority = 0
enabled = true
"#
        );
        let config: Config = toml::from_str(&old).unwrap();
        assert_eq!(
            config.download_directory,
            PathBuf::from("/tmp/gog-downloads")
        );
        assert_eq!(config.installer_language.as_deref(), Some("English"));
        assert!(!config.installer_windows);
        assert!(config.download_extras_by_default);
        assert!(!config.download_patches_by_default);
        assert!(!config.show_retired_artifacts);
        assert_eq!(config.max_concurrent_downloads, 2);
        assert_eq!(config.game_libraries.len(), 1);
        assert!(config.game_libraries[0].default);
        let saved = toml::to_string(&config).unwrap();
        assert!(saved.contains("download_extras_by_default = true"));
        assert!(saved.contains("download_patches_by_default = false"));
        assert!(saved.contains("show_retired_artifacts = false"));
        assert!(!saved.contains(&obsolete_key));
    }

    #[test]
    fn game_libraries_are_unique_and_have_exactly_one_default() {
        let path = PathBuf::from("/games/fast");
        let mut config = Config {
            game_libraries: vec![
                GameLibrary {
                    id: String::new(),
                    name: String::new(),
                    path: path.clone(),
                    default: false,
                },
                GameLibrary {
                    id: "duplicate".into(),
                    name: "Duplicate".into(),
                    path,
                    default: true,
                },
                GameLibrary {
                    id: "archive".into(),
                    name: "Archive".into(),
                    path: PathBuf::from("/games/archive"),
                    default: true,
                },
            ],
            ..Config::default()
        };

        config.normalize_game_libraries();

        assert_eq!(config.game_libraries.len(), 2);
        assert_eq!(
            config
                .game_libraries
                .iter()
                .filter(|library| library.default)
                .count(),
            1
        );
        assert!(!config.game_libraries[0].id.is_empty());
        assert_eq!(config.game_libraries[0].name, "fast");
    }

    #[test]
    fn installation_source_order_defaults_and_normalizes() {
        assert_eq!(
            Config::default().installation_source_order,
            default_installation_source_order()
        );
        let mut config = Config {
            installation_source_order: vec![
                PreferredInstallationSource::WindowsOffline,
                PreferredInstallationSource::WindowsOffline,
            ],
            ..Config::default()
        };
        config.normalize_installation_source_order();
        assert_eq!(
            config.installation_source_order,
            [
                PreferredInstallationSource::WindowsOffline,
                PreferredInstallationSource::LinuxOffline,
                PreferredInstallationSource::WindowsGalaxy,
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn configuration_writes_are_private_and_replace_partial_files() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "gog-config-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join("config.toml");
        write_config(&path, &Config::default()).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!path.with_extension("toml.part").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn patch_updates_are_opt_in() {
    assert!(!Config::default().prefer_patch_updates);
}

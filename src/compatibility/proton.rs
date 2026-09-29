use super::{CompatibilityFailure, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, OpenOptions},
    io::Read,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProtonFamily {
    Ge,
    Umu,
    Valve,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtonInstallation {
    pub path: PathBuf,
    pub name: String,
    pub family: ProtonFamily,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProtonPreferences {
    pub default: Option<PathBuf>,
    pub overrides: BTreeMap<String, PathBuf>,
}

/// Validate a Proton directory without executing or changing an external installation.
pub fn validate_proton(path: &Path) -> Result<ProtonInstallation> {
    let path = fs::canonicalize(path)
        .map_err(|_| CompatibilityFailure::ProtonSelectionMissing(path.to_owned()))?;
    let executable = |path: &Path| {
        fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if !executable(&path.join("proton"))
        || !path.join("toolmanifest.vdf").is_file()
        || !["files/bin/wine", "dist/bin/wine"]
            .iter()
            .any(|wine| executable(&path.join(wine)))
    {
        return Err(CompatibilityFailure::ProtonSelectionMissing(path));
    }
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let lower = name.to_ascii_lowercase();
    let family = if lower.starts_with("ge-proton") || lower.contains("-ge") {
        ProtonFamily::Ge
    } else if lower.starts_with("umu-proton") {
        ProtonFamily::Umu
    } else if lower.starts_with("proton") {
        ProtonFamily::Valve
    } else {
        ProtonFamily::Custom
    };
    Ok(ProtonInstallation { path, name, family })
}

/// Shallow scans only; the caller must run this off the GTK thread.
pub fn discover_proton() -> Vec<ProtonInstallation> {
    let home = dirs::home_dir().unwrap_or_default();
    discover_at(
        &home,
        &dirs::data_dir().unwrap_or_else(|| home.join(".local/share")),
        &dirs::config_dir().unwrap_or_else(|| home.join(".config")),
    )
}

fn discover_at(home: &Path, data: &Path, config: &Path) -> Vec<ProtonInstallation> {
    let steam = [
        home.join(".steam/root"),
        home.join(".steam/steam"),
        data.join("Steam"),
        home.join(".var/app/com.valvesoftware.Steam/data/Steam"),
    ];
    let mut roots = vec![
        data.join("ludomere/proton"),
        config.join("heroic/tools/proton"),
        data.join("lutris/runners/proton"),
        data.join("lutris/runners/wine"),
        home.join(".var/app/com.heroicgameslauncher.hgl/config/heroic/tools/proton"),
        home.join(".var/app/net.lutris.Lutris/data/lutris/runners/proton"),
        home.join(".var/app/net.lutris.Lutris/data/lutris/runners/wine"),
    ];
    for directory in steam {
        roots.push(directory.join("compatibilitytools.d"));
        roots.push(directory.join("steamapps/common"));
        for vdf in ["steamapps/libraryfolders.vdf", "config/libraryfolders.vdf"] {
            if let Ok(text) = read_bounded(&directory.join(vdf)) {
                for path in library_paths(&text) {
                    roots.push(path.join("compatibilitytools.d"));
                    roots.push(path.join("steamapps/common"));
                }
            }
        }
    }
    let mut seen_roots = HashSet::new();
    let mut seen = HashSet::new();
    let mut installations = Vec::new();
    for root in roots {
        let Ok(root) = fs::canonicalize(root) else {
            continue;
        };
        if !seen_roots.insert(root.clone()) {
            continue;
        }
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        for entry in entries.take(4096).filter_map(std::result::Result::ok) {
            if let Ok(proton) = validate_proton(&entry.path())
                && seen.insert(proton.path.clone())
            {
                installations.push(proton);
            }
        }
    }
    installations.sort_by(|a, b| {
        a.family
            .cmp(&b.family)
            .then_with(|| stable(&b.name).cmp(&stable(&a.name)))
            .then_with(|| version(&b.name).cmp(&version(&a.name)))
            .then_with(|| a.path.cmp(&b.path))
    });
    installations
}

fn stable(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !["experimental", "beta", "rc", "nightly", "bleeding", "alpha"]
        .iter()
        .any(|marker| lower.contains(marker))
}

fn version(name: &str) -> Vec<u64> {
    name.split(|c: char| !c.is_ascii_digit())
        .filter_map(|part| part.parse().ok())
        .collect()
}

fn read_bounded(path: &Path) -> std::io::Result<String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other(
            "Proton metadata is not a regular file",
        ));
    }
    let mut text = String::new();
    file.take(1024 * 1024 + 1).read_to_string(&mut text)?;
    if text.len() > 1024 * 1024 {
        return Err(std::io::Error::other("Proton metadata exceeds 1 MiB"));
    }
    Ok(text)
}

// Steam's libraryfolders.vdf uses quoted path values. Decode escapes rather than
// splitting on whitespace, so library names containing spaces remain intact.
fn library_paths(text: &str) -> Vec<PathBuf> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'/') {
            for c in chars.by_ref() {
                if c == '\n' {
                    break;
                }
            }
        } else if c == '"' {
            let mut token = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' => {
                        if let Some(c) = chars.next() {
                            token.push(c);
                        }
                    }
                    c => token.push(c),
                }
            }
            tokens.push(token);
        }
    }
    tokens
        .windows(2)
        .filter(|pair| {
            (pair[0] == "path" || pair[0].chars().all(|c| c.is_ascii_digit()))
                && Path::new(&pair[1]).is_absolute()
        })
        .map(|pair| PathBuf::from(&pair[1]))
        .collect()
}

pub fn proton_preferences() -> Result<ProtonPreferences> {
    read_preferences(&crate::identity::config_root().join("proton.json"))
}

fn read_preferences(path: &Path) -> Result<ProtonPreferences> {
    match read_bounded(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|error| {
            CompatibilityFailure::Io(format!("Invalid Proton preferences: {error}"))
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(ProtonPreferences::default())
        }
        Err(error) => Err(error.into()),
    }
}

fn update_preferences<T>(
    path: &Path,
    update: impl FnOnce(&mut ProtonPreferences) -> Result<T>,
) -> Result<T> {
    fs::create_dir_all(path.parent().unwrap())?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path.with_extension("lock"))?;
    fs2::FileExt::lock_exclusive(&lock)?;
    let mut preferences = read_preferences(path)?;
    let result = update(&mut preferences)?;
    let mut temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    serde_json::to_writer_pretty(&mut temporary, &preferences)
        .map_err(|error| CompatibilityFailure::Io(error.to_string()))?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| CompatibilityFailure::Io(error.to_string()))?;
    Ok(result)
}

pub fn set_default_proton(path: &Path) -> Result<()> {
    let proton = validate_proton(path)?;
    update_preferences(
        &crate::identity::config_root().join("proton.json"),
        |preferences| {
            preferences.default = Some(proton.path);
            Ok(())
        },
    )
}

pub fn set_game_proton(product_id: i64, path: Option<&Path>) -> Result<()> {
    let proton = path.map(validate_proton).transpose()?;
    update_preferences(
        &crate::identity::config_root().join("proton.json"),
        |preferences| {
            if let Some(proton) = proton {
                preferences
                    .default
                    .get_or_insert_with(|| proton.path.clone());
                preferences
                    .overrides
                    .insert(product_id.to_string(), proton.path);
            } else {
                preferences.overrides.remove(&product_id.to_string());
            }
            Ok(())
        },
    )
}

/// Resolve and persist the first default. Missing saved choices never fall back.
pub fn select_proton(product_id: Option<i64>) -> Result<ProtonInstallation> {
    update_preferences(
        &crate::identity::config_root().join("proton.json"),
        |preferences| choose(preferences, product_id, discover_proton),
    )
}

fn choose(
    preferences: &mut ProtonPreferences,
    product_id: Option<i64>,
    discover: impl FnOnce() -> Vec<ProtonInstallation>,
) -> Result<ProtonInstallation> {
    if let Some(path) = product_id
        .and_then(|id| preferences.overrides.get(&id.to_string()))
        .or(preferences.default.as_ref())
    {
        return validate_proton(path);
    }
    let proton = discover()
        .into_iter()
        .find(|proton| stable(&proton.name) && proton.family != ProtonFamily::Custom)
        .ok_or(CompatibilityFailure::ProtonMissing)?;
    preferences.default = Some(proton.path.clone());
    Ok(proton)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(root: &Path, name: &str) -> PathBuf {
        let path = root.join(name);
        fs::create_dir_all(path.join("files/bin")).unwrap();
        for file in ["proton", "files/bin/wine"] {
            fs::write(path.join(file), "#!/bin/sh\n").unwrap();
            fs::set_permissions(path.join(file), fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::write(path.join("toolmanifest.vdf"), "manifest {}").unwrap();
        path
    }

    #[test]
    fn discovery_orders_deduplicates_and_reads_other_steam_libraries() {
        let root =
            std::env::temp_dir().join(format!("ludomere-proton-discovery-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let steam = root.join(".local/share/Steam");
        fixture(&steam.join("compatibilitytools.d"), "GE-Proton9-9");
        fixture(&steam.join("compatibilitytools.d"), "GE-Proton9-10");
        fixture(&steam.join("compatibilitytools.d"), "UMU-Proton99-1");
        fixture(&steam.join("steamapps/common"), "Proton 10.0");
        let external = root.join("Other Games");
        fixture(&external.join("compatibilitytools.d"), "GE-Proton10-1");
        fs::write(
            steam.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\" {{ \"1\" {{ \"path\" \"{}\" }} }}",
                external.display()
            ),
        )
        .unwrap();
        fs::create_dir_all(root.join(".steam")).unwrap();
        std::os::unix::fs::symlink(&steam, root.join(".steam/root")).unwrap();
        let discovered = discover_at(&root, &root.join(".local/share"), &root.join(".config"));
        assert_eq!(
            discovered
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            [
                "GE-Proton10-1",
                "GE-Proton9-10",
                "GE-Proton9-9",
                "UMU-Proton99-1",
                "Proton 10.0"
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn first_default_is_sticky_and_missing_override_never_falls_back() {
        let root =
            std::env::temp_dir().join(format!("ludomere-proton-choice-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let first = validate_proton(&fixture(&root, "GE-Proton9-1")).unwrap();
        let mut preferences = ProtonPreferences::default();
        assert_eq!(
            choose(&mut preferences, Some(42), || vec![first.clone()]).unwrap(),
            first
        );
        assert_eq!(
            choose(&mut preferences, Some(43), || panic!(
                "saved default must win"
            ))
            .unwrap(),
            first
        );
        let newer = validate_proton(&fixture(&root, "GE-Proton10-1")).unwrap();
        assert_eq!(
            choose(&mut preferences, Some(43), || vec![newer]).unwrap(),
            first
        );
        preferences
            .overrides
            .insert("42".into(), root.join("missing"));
        assert!(matches!(
            choose(&mut preferences, Some(42), || vec![first]),
            Err(CompatibilityFailure::ProtonSelectionMissing(_))
        ));
        let path = root.join("proton.json");
        update_preferences(&path, |saved| {
            *saved = preferences.clone();
            Ok(())
        })
        .unwrap();
        let saved = read_preferences(&path).unwrap();
        assert_eq!(saved.default, preferences.default);
        assert_eq!(saved.overrides, preferences.overrides);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn manual_custom_versions_work_and_preferences_updates_preserve_overrides() {
        let root = tempfile::tempdir().unwrap();
        let custom = validate_proton(&fixture(root.path(), "renamed-runtime")).unwrap();
        assert_eq!(custom.family, ProtonFamily::Custom);
        let path = root.path().join("proton.json");
        update_preferences(&path, |saved| {
            saved.default = Some(custom.path.clone());
            saved.overrides.insert("42".into(), custom.path.clone());
            Ok(())
        })
        .unwrap();
        let other = fixture(root.path(), "UMU-Proton9-1");
        update_preferences(&path, |saved| {
            saved.default = Some(other);
            Ok(())
        })
        .unwrap();
        let mut saved = read_preferences(&path).unwrap();
        assert_eq!(choose(&mut saved, Some(42), Vec::new).unwrap(), custom);
    }

    #[test]
    fn discovers_flatpak_and_heroic_lutris_versions_without_scanning_wine() {
        let root = tempfile::tempdir().unwrap();
        for (folder, name) in [
            (
                ".var/app/com.valvesoftware.Steam/data/Steam/compatibilitytools.d",
                "GE-Proton10-1",
            ),
            (".config/heroic/tools/proton", "GE-Proton10-2"),
            (".local/share/lutris/runners/proton", "GE-Proton10-3"),
            (
                ".var/app/com.heroicgameslauncher.hgl/config/heroic/tools/proton",
                "GE-Proton10-4",
            ),
            (
                ".var/app/net.lutris.Lutris/data/lutris/runners/wine",
                "GE-Proton10-5",
            ),
            (".local/share/ludomere/proton", "UMU-Proton10-1"),
        ] {
            fixture(&root.path().join(folder), name);
        }
        let wine = root.path().join(".local/share/lutris/runners/wine/wine-ge");
        fs::create_dir_all(&wine).unwrap();
        fs::write(wine.join("wine"), "not proton").unwrap();
        let found = discover_at(
            root.path(),
            &root.path().join(".local/share"),
            &root.path().join(".config"),
        );
        assert_eq!(found.len(), 6);
        assert_eq!(found.first().unwrap().name, "GE-Proton10-5");
        assert_eq!(found.last().unwrap().name, "UMU-Proton10-1");
    }

    #[test]
    fn concurrent_updates_preserve_other_games_and_missing_defaults_are_errors() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("proton.json");
        std::thread::scope(|scope| {
            for id in 0..8 {
                let path = &path;
                scope.spawn(move || {
                    update_preferences(path, |saved| {
                        saved
                            .overrides
                            .insert(id.to_string(), PathBuf::from(format!("/runtime/{id}")));
                        Ok(())
                    })
                    .unwrap()
                });
            }
        });
        let mut saved = read_preferences(&path).unwrap();
        assert_eq!(saved.overrides.len(), 8);
        saved.default = Some(root.path().join("removed"));
        assert!(matches!(
            choose(&mut saved, None, || panic!("must not fallback")),
            Err(CompatibilityFailure::ProtonSelectionMissing(_))
        ));
        fs::write(&path, "broken json").unwrap();
        assert!(read_preferences(&path).is_err());
    }
}

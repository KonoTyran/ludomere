use crate::domain::CloudSaveLocation;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

pub fn resolve_locations(
    configured: &[crate::cloud_saves::metadata::RemoteLocation],
    install: &Path,
    prefix: &Path,
    client_id: &str,
) -> Result<Vec<CloudSaveLocation>> {
    if configured.is_empty() {
        return Ok(vec![CloudSaveLocation {
            name: "galaxy-sdk".into(),
            path: variable_root("APPLICATION_DATA_LOCAL", install, prefix)?
                .join("GOG.com/Galaxy/Applications")
                .join(client_id)
                .join("Storage"),
            remote_namespace: "galaxy-sdk".into(),
            user_override: false,
        }]);
    }
    configured
        .iter()
        .enumerate()
        .map(|(index, location)| {
            let name = location
                .name
                .clone()
                .unwrap_or_else(|| format!("location-{index}"));
            Ok(CloudSaveLocation {
                path: resolve(&location.location, install, prefix)?,
                remote_namespace: name.clone(),
                name,
                user_override: false,
            })
        })
        .collect()
}

pub fn resolve(template: &str, install: &Path, prefix: &Path) -> Result<PathBuf> {
    let (variable, rest) = parse_variable(template)?;
    let root = variable_root(variable, install, prefix)?;
    let mut relative = PathBuf::new();
    for component in rest.trim_start_matches(['/', '\\']).split(['/', '\\']) {
        match component {
            "" | "." => {}
            ".." => bail!("cloud-save location escapes its managed root"),
            component if component.contains(':') => {
                bail!("cloud-save location contains an absolute Windows path")
            }
            component => relative.push(component),
        }
    }
    let candidate = root.join(&relative);
    validate_containment(&candidate, &root)?;
    Ok(candidate)
}

fn parse_variable(value: &str) -> Result<(&str, &str)> {
    let value = value.trim();
    let end = value
        .find("?>")
        .context("cloud-save location has no Galaxy variable")?;
    value
        .strip_prefix("<?")
        .context("cloud-save location must begin with a Galaxy variable")?;
    Ok((&value[2..end], &value[end + 2..]))
}

fn variable_root(variable: &str, install: &Path, prefix: &Path) -> Result<PathBuf> {
    let user = prefix.join("drive_c/users/steamuser");
    Ok(match variable {
        "INSTALL" => install.to_owned(),
        "DOCUMENTS" => user.join("Documents"),
        "SAVED_GAMES" => user.join("Saved Games"),
        "APPLICATION_DATA_LOCAL" => user.join("AppData/Local"),
        "APPLICATION_DATA_LOCAL_LOW" => user.join("AppData/LocalLow"),
        "APPLICATION_DATA_ROAMING" => user.join("AppData/Roaming"),
        _ => bail!("unknown Galaxy cloud-save variable"),
    })
}

fn validate_containment(path: &Path, root: &Path) -> Result<()> {
    let canonical_root = canonical_existing(root)?;
    let canonical_parent = canonical_existing(path.parent().unwrap_or(path))?;
    if !canonical_parent.starts_with(&canonical_root) {
        bail!("cloud-save location resolves outside its managed root");
    }
    Ok(())
}

fn canonical_existing(path: &Path) -> Result<PathBuf> {
    let mut current = path;
    while !current.exists() {
        current = current.parent().context("path has no existing ancestor")?;
    }
    std::fs::canonicalize(current).context("canonicalizing cloud-save path")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_supported_variables_and_rejects_traversal() {
        let root =
            std::env::temp_dir().join(format!("ludomere-cloud-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("prefix/drive_c/users/steamuser")).unwrap();
        std::fs::create_dir_all(root.join("game")).unwrap();
        for variable in [
            "DOCUMENTS",
            "SAVED_GAMES",
            "APPLICATION_DATA_LOCAL",
            "APPLICATION_DATA_LOCAL_LOW",
            "APPLICATION_DATA_ROAMING",
        ] {
            assert!(
                resolve(
                    &format!("<?{variable}?>/Game"),
                    &root.join("game"),
                    &root.join("prefix")
                )
                .is_ok()
            );
        }
        assert!(
            resolve(
                "<?INSTALL?>/../other",
                &root.join("game"),
                &root.join("prefix")
            )
            .is_err()
        );
        assert!(resolve("<?UNKNOWN?>/save", &root.join("game"), &root.join("prefix")).is_err());
        assert_eq!(
            resolve(
                r"<?DOCUMENTS?>\My Games\Grim Dawn\save",
                &root.join("game"),
                &root.join("prefix")
            )
            .unwrap(),
            root.join("prefix/drive_c/users/steamuser/Documents/My Games/Grim Dawn/save")
        );
        assert!(
            resolve(
                r"<?DOCUMENTS?>\My Games\..\secrets",
                &root.join("game"),
                &root.join("prefix")
            )
            .is_err()
        );
        assert!(
            resolve(
                r"<?DOCUMENTS?>\C:\outside",
                &root.join("game"),
                &root.join("prefix")
            )
            .is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

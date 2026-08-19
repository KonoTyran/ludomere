use crate::state::{DepotOperationRecord, InstallationOperationRecord};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OperationJournal {
    Offline {
        version: u32,
        record: InstallationOperationRecord,
    },
    Depot {
        version: u32,
        record: DepotOperationRecord,
    },
}

pub fn path(library: &Path, slug: &str) -> Result<PathBuf> {
    if slug.is_empty() || slug.contains(['/', '\\']) || matches!(slug, "." | "..") {
        bail!("unsafe operation journal slug");
    }
    Ok(library
        .join(".ludomere/staging")
        .join(format!("{slug}.operation.json")))
}

pub fn write_offline(path: &Path, record: &InstallationOperationRecord) -> Result<()> {
    write(
        path,
        &OperationJournal::Offline {
            version: VERSION,
            record: record.clone(),
        },
    )
}

pub fn write_depot(path: &Path, record: &DepotOperationRecord) -> Result<()> {
    write(
        path,
        &OperationJournal::Depot {
            version: VERSION,
            record: record.clone(),
        },
    )
}

fn write(path: &Path, journal: &OperationJournal) -> Result<()> {
    let parent = path.parent().context("operation journal has no parent")?;
    if let Some(control) = parent.parent() {
        reject_symlink(control)?;
    }
    reject_symlink(parent)?;
    fs::create_dir_all(parent)?;
    reject_symlink(parent)?;
    reject_symlink(path)?;
    let temporary = path.with_extension("operation.json.tmp");
    reject_symlink(&temporary)?;
    let bytes = serde_json::to_vec_pretty(journal)?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(temporary, path)?;
    Ok(())
}

pub fn read(path: &Path) -> Result<OperationJournal> {
    reject_symlink(path)?;
    let journal: OperationJournal = serde_json::from_slice(&fs::read(path)?)?;
    let version = match &journal {
        OperationJournal::Offline { version, .. } | OperationJournal::Depot { version, .. } => {
            *version
        }
    };
    if version != VERSION {
        bail!("unsupported operation journal version");
    }
    Ok(journal)
}

pub fn scan() -> Result<Vec<(PathBuf, OperationJournal)>> {
    let mut journals = Vec::new();
    for library in crate::config::Config::load_or_create()?.game_libraries {
        let staging = library.path.join(".ludomere/staging");
        let Ok(entries) = fs::read_dir(staging) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".operation.json"))
            {
                journals.push((path.clone(), read(&path)?));
            }
        }
    }
    Ok(journals)
}

pub fn find_depot(operation_id: &str) -> Result<(PathBuf, DepotOperationRecord)> {
    scan()?
        .into_iter()
        .find_map(|(path, journal)| match journal {
            OperationJournal::Depot { record, .. } if record.operation_id == operation_id => {
                Some((path, record))
            }
            _ => None,
        })
        .context("saved depot operation was not found")
}

pub fn remove(path: &Path) -> Result<()> {
    reject_symlink(path)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn depot_path(staging: &Path) -> PathBuf {
    staging.with_extension("operation.json")
}

pub fn offline_path(record: &InstallationOperationRecord) -> Result<PathBuf> {
    let value: serde_json::Value = serde_json::from_str(&record.plan_json)?;
    let directory = value
        .get("game")
        .unwrap_or(&value)
        .get("installation_directory")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .context("saved installation plan has no destination")?;
    let library = directory
        .parent()
        .context("installation destination has no library")?;
    let slug = directory
        .file_name()
        .and_then(|name| name.to_str())
        .context("invalid installation slug")?;
    path(library, slug)
}

fn reject_symlink(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        bail!("operation journal is a symlink");
    }
    Ok(())
}

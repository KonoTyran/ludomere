use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

const MAX_SAVE_FILES: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveLocation {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationPhase {
    PreparingBackup,
    BackedUp,
    Uninstalled,
    Installed,
    Restoring,
    Complete,
}

#[derive(Clone, Serialize, Deserialize)]
pub enum MigrationTarget {
    Offline {
        game: crate::domain::InstalledGame,
        additional_installers: Vec<super::AdditionalInstaller>,
        interactive_prompts: bool,
    },
    Galaxy(crate::gog::depot_service::PrepareOperationRequest),
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MigrationJournal {
    pub operation_id: String,
    pub product_id: i64,
    pub slug: String,
    pub phase: MigrationPhase,
    pub locations: Vec<SaveLocation>,
    #[serde(default)]
    pub target: Option<MigrationTarget>,
    #[serde(default)]
    pub old_game: Option<crate::domain::InstalledGame>,
}

pub fn migration_root(library: &Path, slug: &str, operation_id: &str) -> Result<PathBuf> {
    validate_component(slug)?;
    validate_component(operation_id)?;
    Ok(library
        .join(".ludomere/save-migrations")
        .join(format!("{slug}-{operation_id}")))
}

pub fn begin_backup(
    library: &Path,
    operation_id: &str,
    product_id: i64,
    slug: &str,
    locations: &[SaveLocation],
) -> Result<MigrationJournal> {
    let root = migration_root(library, slug, operation_id)?;
    reject_symlink_ancestors(&root)?;
    if root.exists() {
        bail!("save migration already exists");
    }
    fs::create_dir_all(root.join("saves"))?;
    set_private_directory(&root)?;
    set_private_directory(&root.join("saves"))?;
    let mut journal = MigrationJournal {
        operation_id: operation_id.to_owned(),
        product_id,
        slug: slug.to_owned(),
        phase: MigrationPhase::PreparingBackup,
        locations: locations.to_vec(),
        target: None,
        old_game: None,
    };
    write_journal(&root, &journal)?;
    let mut files = 0;
    for (index, location) in locations.iter().enumerate() {
        if location.path.exists() {
            copy_tree(
                &location.path,
                &root.join("saves").join(index.to_string()),
                &mut files,
            )?;
        }
    }
    journal.phase = MigrationPhase::BackedUp;
    write_journal(&root, &journal)?;
    Ok(journal)
}

pub fn configure(
    library: &Path,
    journal: &mut MigrationJournal,
    old_game: crate::domain::InstalledGame,
    target: MigrationTarget,
) -> Result<()> {
    if journal.phase != MigrationPhase::BackedUp {
        bail!("save migration must be backed up before it is configured");
    }
    journal.old_game = Some(old_game);
    journal.target = Some(target);
    let root = migration_root(library, &journal.slug, &journal.operation_id)?;
    write_journal(&root, journal)
}

pub fn restore(
    library: &Path,
    journal: &mut MigrationJournal,
    destinations: &[SaveLocation],
) -> Result<()> {
    if !matches!(
        journal.phase,
        MigrationPhase::Installed | MigrationPhase::Restoring
    ) {
        bail!("save migration is not ready to restore");
    }
    if journal.locations.len() != destinations.len()
        || journal
            .locations
            .iter()
            .zip(destinations)
            .any(|(source, target)| source.name != target.name)
    {
        bail!("save migration locations do not match");
    }
    let root = migration_root(library, &journal.slug, &journal.operation_id)?;
    journal.phase = MigrationPhase::Restoring;
    write_journal(&root, journal)?;
    let mut files = 0;
    for (index, destination) in destinations.iter().enumerate() {
        let source = root.join("saves").join(index.to_string());
        if source.exists() {
            copy_tree(&source, &destination.path, &mut files)?;
        }
    }
    journal.phase = MigrationPhase::Complete;
    write_journal(&root, journal)
}

pub fn set_phase(
    library: &Path,
    journal: &mut MigrationJournal,
    phase: MigrationPhase,
) -> Result<()> {
    let root = migration_root(library, &journal.slug, &journal.operation_id)?;
    journal.phase = phase;
    write_journal(&root, journal)
}

pub fn load(library: &Path, slug: &str, operation_id: &str) -> Result<MigrationJournal> {
    let root = migration_root(library, slug, operation_id)?;
    reject_symlink_ancestors(&root)?;
    let bytes = fs::read(root.join("journal.json"))?;
    serde_json::from_slice(&bytes).context("parsing save migration journal")
}

pub fn discover(
    libraries: &[crate::config::GameLibrary],
) -> Result<Vec<(PathBuf, MigrationJournal)>> {
    let mut found = Vec::new();
    for library in libraries {
        let root = library.path.join(".ludomere/save-migrations");
        if !root.is_dir() {
            continue;
        }
        reject_symlink_ancestors(&root)?;
        for entry in fs::read_dir(&root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let bytes = fs::read(entry.path().join("journal.json"))?;
            let journal: MigrationJournal =
                serde_json::from_slice(&bytes).context("parsing save migration journal")?;
            if migration_root(&library.path, &journal.slug, &journal.operation_id)? != entry.path()
            {
                bail!("save migration journal identity does not match its directory");
            }
            if journal.phase != MigrationPhase::Complete {
                found.push((library.path.clone(), journal));
            }
        }
    }
    Ok(found)
}

pub fn finish(library: &Path, journal: &MigrationJournal) -> Result<()> {
    if journal.phase != MigrationPhase::Complete {
        bail!("incomplete save migration cannot be removed");
    }
    let root = migration_root(library, &journal.slug, &journal.operation_id)?;
    reject_symlink_ancestors(&root)?;
    fs::remove_dir_all(root)?;
    Ok(())
}

fn write_journal(root: &Path, journal: &MigrationJournal) -> Result<()> {
    reject_symlink_ancestors(root)?;
    let temporary = root.join("journal.json.part");
    let final_path = root.join("journal.json");
    if temporary.exists() {
        let metadata = fs::symlink_metadata(&temporary)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            bail!("unsafe save migration journal temporary");
        }
        fs::remove_file(&temporary)?;
    }
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(&serde_json::to_vec(journal)?)?;
    file.sync_all()?;
    fs::rename(temporary, final_path)?;
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path, files: &mut usize) -> Result<()> {
    reject_symlink_ancestors(source)?;
    reject_symlink_ancestors(destination)?;
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        bail!("save migration cannot copy symlinks");
    }
    if metadata.is_file() {
        *files = files.checked_add(1).context("save file count overflow")?;
        if *files > MAX_SAVE_FILES {
            bail!("save migration contains too many files");
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, destination)?;
        set_private_file(destination)?;
        return Ok(());
    }
    if !metadata.is_dir() {
        bail!("unsupported save entry type");
    }
    fs::create_dir_all(destination)?;
    set_private_directory(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        copy_tree(&entry.path(), &destination.join(entry.file_name()), files)?;
    }
    Ok(())
}

fn set_private_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn set_private_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn validate_component(value: &str) -> Result<()> {
    if value.is_empty() || value.contains(['/', '\\']) || matches!(value, "." | "..") {
        bail!("unsafe save migration identity");
    }
    Ok(())
}

fn reject_symlink_ancestors(path: &Path) -> Result<()> {
    let mut cursor = PathBuf::new();
    for component in path.components() {
        cursor.push(component.as_os_str());
        if fs::symlink_metadata(&cursor).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            bail!("save migration path crosses a symlink");
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub enum MigrationEvent {
    Phase(MigrationPhase),
    Failed { message: String, backup: PathBuf },
    Complete,
}

pub fn start(
    library: PathBuf,
    mut journal: MigrationJournal,
    destinations: Vec<SaveLocation>,
) -> std::sync::mpsc::Receiver<MigrationEvent> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let backup = migration_root(&library, &journal.slug, &journal.operation_id)
            .unwrap_or_else(|_| library.join(".ludomere/save-migrations"));
        let result = run(&library, &mut journal, &destinations, &sender);
        match result {
            Ok(()) => {
                let _ = sender.send(MigrationEvent::Complete);
            }
            Err(error) => {
                let _ = sender.send(MigrationEvent::Failed {
                    message: format!("{error:#}"),
                    backup,
                });
            }
        }
    });
    receiver
}

fn run(
    library: &Path,
    journal: &mut MigrationJournal,
    destinations: &[SaveLocation],
    events: &std::sync::mpsc::Sender<MigrationEvent>,
) -> Result<()> {
    let old = journal
        .old_game
        .clone()
        .context("save migration has no existing installation plan")?;
    let target = journal
        .target
        .clone()
        .context("save migration has no target installation plan")?;
    if journal.phase == MigrationPhase::BackedUp {
        uninstall(&old, library)?;
        set_phase(library, journal, MigrationPhase::Uninstalled)?;
        let _ = events.send(MigrationEvent::Phase(MigrationPhase::Uninstalled));
    }
    if journal.phase == MigrationPhase::Uninstalled {
        install(target, journal.product_id)?;
        set_phase(library, journal, MigrationPhase::Installed)?;
        let _ = events.send(MigrationEvent::Phase(MigrationPhase::Installed));
    }
    if matches!(
        journal.phase,
        MigrationPhase::Installed | MigrationPhase::Restoring
    ) {
        let _ = events.send(MigrationEvent::Phase(MigrationPhase::Restoring));
        restore(library, journal, destinations)?;
    }
    if journal.phase == MigrationPhase::Complete {
        finish(library, journal)?;
    }
    Ok(())
}

fn uninstall(game: &crate::domain::InstalledGame, library: &Path) -> Result<()> {
    if game.installation_directory.parent() != Some(library) {
        bail!("source migration installation is outside its library");
    }
    reject_symlink_ancestors(&game.installation_directory)?;
    let marker = super::marker::load(&game.installation_directory)?
        .context("source migration installation marker is missing")?;
    if marker.product_id != game.product_id
        || Some(marker.slug.as_str())
            != game
                .installation_directory
                .file_name()
                .and_then(|name| name.to_str())
    {
        bail!("source migration installation identity is inconsistent");
    }
    if marker.source == crate::domain::InstallationSource::OfflineInstaller {
        let handle = super::start_uninstallation(game.clone());
        loop {
            match handle.events.recv()? {
                super::UninstallationEvent::Complete => break,
                super::UninstallationEvent::Cancelled => {
                    bail!("source uninstallation was cancelled")
                }
                super::UninstallationEvent::Failed(error) => bail!(error),
                _ => {}
            }
        }
    }
    if game.installation_directory.exists() {
        fs::remove_dir_all(&game.installation_directory)?;
    }
    if let Some(compatibility) = &game.compatibility {
        let prefix = crate::compatibility::prefix_path(library, &compatibility.prefix_slug);
        reject_symlink_ancestors(&prefix)?;
        if prefix.exists() {
            fs::remove_dir_all(prefix)?;
        }
    }
    Ok(())
}

fn install(target: MigrationTarget, product_id: i64) -> Result<()> {
    match target {
        MigrationTarget::Offline {
            game,
            additional_installers,
            interactive_prompts,
        } => {
            let handle =
                super::start_installation(game, additional_installers, true, interactive_prompts);
            loop {
                match handle.events.recv()? {
                    super::InstallationEvent::Complete { .. } => return Ok(()),
                    super::InstallationEvent::Cancelled => {
                        bail!("replacement installation was cancelled")
                    }
                    super::InstallationEvent::Failed(error) => bail!(error),
                    super::InstallationEvent::Prompt { .. } => {
                        bail!("replacement installer requires interactive input")
                    }
                    _ => {}
                }
            }
        }
        MigrationTarget::Galaxy(request) => {
            let operation_id = request.operation_id.clone();
            let events = super::subscribe_depot_events();
            let store = crate::state::StateStore::open()?;
            crate::gog::depot_service::start_operation(
                &store,
                &reqwest::blocking::Client::new(),
                request,
            )?;
            loop {
                let super::DepotManagerEvent::Snapshot(snapshot) = events.recv()?;
                if snapshot.operation_id != operation_id || snapshot.product_id != product_id {
                    continue;
                }
                match snapshot.state.as_str() {
                    "complete" => return Ok(()),
                    "failed" | "cancelled" => {
                        bail!(
                            snapshot
                                .error
                                .unwrap_or_else(|| "replacement Galaxy installation failed".into())
                        )
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        std::env::temp_dir().join(format!(
            "ludomere-save-migration-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn backup_restore_and_success_cleanup_are_forward_only() {
        let root = temp();
        let old = root.join("old/save");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("hero.sav"), b"save").unwrap();
        let mut journal = begin_backup(
            &root,
            "abc123",
            1,
            "game",
            &[SaveLocation {
                name: "main".into(),
                path: old,
            }],
        )
        .unwrap();
        assert_eq!(journal.phase, MigrationPhase::BackedUp);
        assert!(finish(&root, &journal).is_err());
        set_phase(&root, &mut journal, MigrationPhase::Uninstalled).unwrap();
        set_phase(&root, &mut journal, MigrationPhase::Installed).unwrap();
        let new = root.join("new/save");
        restore(
            &root,
            &mut journal,
            &[SaveLocation {
                name: "main".into(),
                path: new.clone(),
            }],
        )
        .unwrap();
        assert_eq!(fs::read(new.join("hero.sav")).unwrap(), b"save");
        finish(&root, &journal).unwrap();
        assert!(!migration_root(&root, "game", "abc123").unwrap().exists());
        let _ = fs::remove_dir_all(root);
    }
}

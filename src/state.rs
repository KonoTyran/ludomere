use crate::{
    auth::Profile,
    domain::{
        ArtifactKind, DownloadCategory, DownloadRevision, GalaxyBuild, Game, GamePreferences,
        InstalledGame, ProductMetadata, RemoteArtifact,
    },
};
use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProductActivity {
    pub last_played_at: Option<i64>,
    pub last_activity_at: Option<i64>,
    pub playtime_seconds: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadState {
    Queued,
    Downloading,
    Paused,
    Failed,
    Complete,
}

impl DownloadState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Downloading => "downloading",
            Self::Paused => "paused",
            Self::Failed => "failed",
            Self::Complete => "complete",
        }
    }

    fn from_database(value: &str) -> Self {
        match value {
            "queued" => Self::Queued,
            "downloading" => Self::Downloading,
            "paused" | "cancelled" => Self::Paused,
            "complete" => Self::Complete,
            _ => Self::Failed,
        }
    }
}

impl PartialEq<&str> for DownloadState {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other || (*self == Self::Paused && *other == "cancelled")
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DownloadJobRecord {
    pub job_id: String,
    pub product_id: i64,
    pub title: String,
    pub artifacts: Vec<RemoteArtifact>,
    pub state: DownloadState,
    pub destination: PathBuf,
    pub bytes_downloaded: u64,
    pub total_bytes: Option<u64>,
    pub completed_files: Vec<PathBuf>,
    pub error: Option<String>,
    pub status_message: Option<String>,
    pub queue_position: Option<i64>,
    pub retry_started_at: Option<i64>,
    pub next_retry_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
}

pub struct DownloadJobUpdate<'a> {
    pub job_id: &'a str,
    pub product_id: i64,
    pub title: &'a str,
    pub artifacts: &'a [crate::domain::RemoteArtifact],
    pub destination: &'a std::path::Path,
    pub state: DownloadState,
    pub bytes_downloaded: u64,
    pub total_bytes: Option<u64>,
    pub completed_files: &'a [PathBuf],
    pub error: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallationOperationRecord {
    pub product_id: i64,
    pub operation: String,
    pub state: String,
    pub plan_json: String,
    pub message: Option<String>,
    pub percentage: Option<u8>,
    pub queue_position: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedFileRecord {
    pub path: PathBuf,
    pub product_id: i64,
    pub product_slug: String,
    pub kind: ArtifactKind,
    pub operating_system: Option<String>,
    pub language: Option<String>,
    pub filename: String,
    pub size: u64,
    pub artifact_path: Option<String>,
    pub matched: bool,
    pub present: bool,
    pub artifact_id: Option<String>,
    pub job_id: Option<String>,
    pub version: Option<String>,
    pub expected_size: Option<u64>,
    pub gog_checksum: Option<String>,
    pub verified_at: Option<i64>,
    pub revision_id: Option<i64>,
    pub part_id: Option<i64>,
    pub provider_file_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CatalogArtifact {
    pub artifact: RemoteArtifact,
    pub currently_offered: bool,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

pub struct StateStore {
    connection: Connection,
}

const CURRENT_SCHEMA_VERSION: i64 = 24;

impl StateStore {
    pub fn open() -> Result<Self> {
        let path = data_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(data_path(), fs::Permissions::from_mode(0o600))?;
        }
        Self::initialize(connection)
    }

    #[cfg(test)]
    pub(crate) fn open_at(path: &std::path::Path) -> Result<Self> {
        Self::initialize(Connection::open(path)?)
    }

    fn initialize(connection: Connection) -> Result<Self> {
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version != 0 && version != CURRENT_SCHEMA_VERSION {
            bail!(
                "database schema version {version} is unsupported; expected {CURRENT_SCHEMA_VERSION}"
            );
        }

        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS user_game_state (product_id INTEGER PRIMARY KEY, favorite INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS custom_tags (product_id INTEGER NOT NULL, tag TEXT NOT NULL COLLATE NOCASE, PRIMARY KEY (product_id, tag));
             CREATE TABLE IF NOT EXISTS account_cache (cache_key INTEGER PRIMARY KEY CHECK(cache_key = 1), profile_json TEXT NOT NULL, updated_at INTEGER NOT NULL DEFAULT (unixepoch()));
             CREATE TABLE IF NOT EXISTS owned_products (product_id INTEGER PRIMARY KEY, synchronized_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS online_sync_state (sync_key TEXT PRIMARY KEY, completed_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS online_library_cache (cache_key INTEGER PRIMARY KEY CHECK(cache_key = 1), games_json TEXT NOT NULL, updated_at INTEGER NOT NULL DEFAULT (unixepoch()));
             CREATE TABLE IF NOT EXISTS download_manifest_cache (product_id INTEGER PRIMARY KEY, artifacts_json TEXT NOT NULL, updated_at INTEGER NOT NULL DEFAULT (unixepoch()));
             CREATE TABLE IF NOT EXISTS download_jobs (
                job_id TEXT PRIMARY KEY, product_id INTEGER NOT NULL, title TEXT NOT NULL,
                artifacts_json TEXT NOT NULL, destination TEXT NOT NULL, state TEXT NOT NULL,
                bytes_downloaded INTEGER NOT NULL DEFAULT 0, total_bytes INTEGER,
                completed_files_json TEXT NOT NULL DEFAULT '[]', error TEXT,
                updated_at INTEGER NOT NULL DEFAULT (unixepoch()), status_message TEXT,
                queue_position INTEGER, retry_started_at INTEGER, next_retry_at INTEGER,
                created_at INTEGER NOT NULL DEFAULT 0, completed_at INTEGER
             );
             CREATE TABLE IF NOT EXISTS managed_files (
                path TEXT PRIMARY KEY, product_id INTEGER NOT NULL, product_slug TEXT NOT NULL,
                artifact_kind TEXT NOT NULL, operating_system TEXT, language TEXT,
                filename TEXT NOT NULL, size INTEGER NOT NULL, artifact_path TEXT,
                matched INTEGER NOT NULL DEFAULT 0, present INTEGER NOT NULL DEFAULT 1,
                updated_at INTEGER NOT NULL DEFAULT (unixepoch()), artifact_id TEXT, job_id TEXT,
                version TEXT, expected_size INTEGER, gog_checksum TEXT, verified_at INTEGER,
                created_at INTEGER NOT NULL DEFAULT 0, revision_id INTEGER, part_id INTEGER,
                provider_file_id TEXT
             );
             CREATE INDEX IF NOT EXISTS managed_files_product ON managed_files(product_id, present);
             CREATE UNIQUE INDEX IF NOT EXISTS managed_files_artifact_id ON managed_files(artifact_id) WHERE artifact_id IS NOT NULL;
             CREATE TABLE IF NOT EXISTS download_artifact_catalog (
                artifact_id TEXT PRIMARY KEY, product_id INTEGER NOT NULL, artifact_json TEXT NOT NULL,
                currently_offered INTEGER NOT NULL DEFAULT 1, first_seen_at INTEGER NOT NULL,
                last_seen_at INTEGER NOT NULL, retired_at INTEGER
             );
             CREATE INDEX IF NOT EXISTS download_artifact_catalog_product ON download_artifact_catalog(product_id, currently_offered);
             CREATE TABLE IF NOT EXISTS products (
                product_id INTEGER PRIMARY KEY, parent_product_id INTEGER, product_type TEXT NOT NULL,
                slug TEXT NOT NULL, title TEXT NOT NULL, release_date INTEGER, gog_release_date INTEGER,
                description TEXT NOT NULL, changelog TEXT NOT NULL, metadata_json TEXT NOT NULL,
                links_json TEXT NOT NULL, media_json TEXT NOT NULL, currently_owned INTEGER NOT NULL,
                first_seen_at INTEGER NOT NULL, last_seen_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS product_relationships (
                parent_product_id INTEGER NOT NULL, child_product_id INTEGER NOT NULL,
                relationship TEXT NOT NULL, source TEXT NOT NULL,
                PRIMARY KEY (parent_product_id, child_product_id, relationship)
             );
             CREATE TABLE IF NOT EXISTS download_slots (
                slot_id INTEGER PRIMARY KEY, product_id INTEGER NOT NULL, provider_group_id TEXT NOT NULL,
                provider_category TEXT NOT NULL, name TEXT NOT NULL, operating_system TEXT,
                language_code TEXT, language_name TEXT, first_seen_at INTEGER NOT NULL,
                last_seen_at INTEGER NOT NULL, UNIQUE(product_id, provider_category, provider_group_id)
             );
             CREATE TABLE IF NOT EXISTS download_revisions (
                revision_id INTEGER PRIMARY KEY, slot_id INTEGER NOT NULL, version TEXT,
                total_size INTEGER, manifest_fingerprint TEXT NOT NULL, currently_offered INTEGER NOT NULL,
                first_seen_at INTEGER NOT NULL, last_seen_at INTEGER NOT NULL, retired_at INTEGER,
                UNIQUE(slot_id, manifest_fingerprint)
             );
             CREATE TABLE IF NOT EXISTS download_parts (
                part_id INTEGER PRIMARY KEY, revision_id INTEGER NOT NULL, provider_file_id TEXT NOT NULL,
                part_index INTEGER NOT NULL, expected_size INTEGER, downlink TEXT NOT NULL, checksum TEXT,
                checksum_fetched_at INTEGER, UNIQUE(revision_id, provider_file_id)
             );
             CREATE TABLE IF NOT EXISTS galaxy_builds (
                build_id TEXT PRIMARY KEY, product_id INTEGER NOT NULL, operating_system TEXT NOT NULL,
                version TEXT, branch TEXT, tags_json TEXT NOT NULL, public INTEGER NOT NULL,
                generation INTEGER NOT NULL, repository_url TEXT NOT NULL, repository_id TEXT,
                published_at INTEGER, currently_returned INTEGER NOT NULL, first_seen_at INTEGER NOT NULL,
                last_seen_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS galaxy_builds_product ON galaxy_builds(product_id, operating_system, published_at DESC);
             CREATE INDEX IF NOT EXISTS download_slots_product ON download_slots(product_id, provider_category);
             CREATE INDEX IF NOT EXISTS download_revisions_slot ON download_revisions(slot_id, currently_offered, last_seen_at DESC);
             CREATE INDEX IF NOT EXISTS download_parts_revision ON download_parts(revision_id, part_index);
             CREATE TABLE IF NOT EXISTS enrichment_observations (
                product_id INTEGER NOT NULL, source TEXT NOT NULL, status TEXT NOT NULL,
                checked_at INTEGER NOT NULL, PRIMARY KEY (product_id, source)
             );
             CREATE INDEX IF NOT EXISTS enrichment_observations_due ON enrichment_observations(source, status, checked_at);
             CREATE TABLE IF NOT EXISTS sync_stage_outcomes (
                stage TEXT PRIMARY KEY, status TEXT NOT NULL, started_at INTEGER NOT NULL,
                completed_at INTEGER, error TEXT
             );
             CREATE TABLE IF NOT EXISTS installation_operations (
                product_id INTEGER PRIMARY KEY, operation TEXT NOT NULL, state TEXT NOT NULL,
                plan_json TEXT NOT NULL, message TEXT, percentage INTEGER, created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL, completed_at INTEGER, queue_position INTEGER
             );
             CREATE INDEX IF NOT EXISTS installation_operations_state ON installation_operations(state, updated_at);
             CREATE INDEX IF NOT EXISTS installation_operations_queue ON installation_operations(state, queue_position);
             CREATE TABLE IF NOT EXISTS product_activity (
                product_id INTEGER PRIMARY KEY, last_played_at INTEGER,
                playtime_seconds INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL,
                last_activity_at INTEGER
             );
             CREATE TABLE IF NOT EXISTS game_preferences (
                product_id INTEGER PRIMARY KEY, executable_path TEXT,
                launch_arguments_json TEXT NOT NULL DEFAULT '[]', compatibility_json TEXT,
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS game_compatibility_fix_overrides (
                product_id INTEGER NOT NULL, fix_id TEXT NOT NULL, enabled INTEGER NOT NULL,
                PRIMARY KEY(product_id, fix_id)
             );"
        )?;
        if version == 0 {
            connection.pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)?;
        }
        Ok(Self { connection })
    }

    pub fn compatibility_fix_overrides(&self, product_id: i64) -> Result<HashMap<String, bool>> {
        let mut statement = self.connection.prepare(
            "SELECT fix_id, enabled FROM game_compatibility_fix_overrides WHERE product_id = ?1",
        )?;
        let rows = statement.query_map([product_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<HashMap<String, bool>>>()
            .map_err(Into::into)
    }

    pub fn set_compatibility_fix_override(
        &self,
        product_id: i64,
        fix_id: &str,
        enabled: bool,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO game_compatibility_fix_overrides(product_id, fix_id, enabled)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(product_id, fix_id) DO UPDATE SET enabled = excluded.enabled",
            params![product_id, fix_id, enabled],
        )?;
        Ok(())
    }

    pub fn clear_compatibility_fix_overrides(&self, product_id: i64) -> Result<()> {
        self.connection.execute(
            "DELETE FROM game_compatibility_fix_overrides WHERE product_id = ?1",
            [product_id],
        )?;
        Ok(())
    }

    pub fn game_preferences(&self, product_id: i64) -> Result<Option<GamePreferences>> {
        self.connection
            .query_row(
                "SELECT product_id, executable_path, launch_arguments_json, compatibility_json,
                        created_at, updated_at
                 FROM game_preferences WHERE product_id = ?1",
                params![product_id],
                |row| {
                    Ok(GamePreferences {
                        product_id: row.get(0)?,
                        executable_path: row.get::<_, Option<String>>(1)?.map(PathBuf::from),
                        launch_arguments: serde_json::from_str(&row.get::<_, String>(2)?)
                            .unwrap_or_default(),
                        compatibility: row
                            .get::<_, Option<String>>(3)?
                            .and_then(|value| serde_json::from_str(&value).ok()),
                        created_at: row.get(4)?,
                        updated_at: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn upsert_game_preferences(&self, preferences: &GamePreferences) -> Result<()> {
        self.connection.execute(
            "INSERT INTO game_preferences(
                product_id, executable_path, launch_arguments_json, compatibility_json,
                created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(product_id) DO UPDATE SET
                executable_path = excluded.executable_path,
                launch_arguments_json = excluded.launch_arguments_json,
                compatibility_json = excluded.compatibility_json,
                updated_at = excluded.updated_at",
            params![
                preferences.product_id,
                preferences
                    .executable_path
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
                serde_json::to_string(&preferences.launch_arguments)?,
                preferences
                    .compatibility
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?,
                preferences.created_at,
                preferences.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn upsert_installation_operation(
        &self,
        operation: &InstallationOperationRecord,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO installation_operations(
                product_id, operation, state, plan_json, message, percentage,
                queue_position, created_at, updated_at, completed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(product_id) DO UPDATE SET
                operation = excluded.operation, state = excluded.state,
                plan_json = excluded.plan_json, message = excluded.message,
                percentage = excluded.percentage, updated_at = excluded.updated_at,
                queue_position = excluded.queue_position,
                completed_at = excluded.completed_at",
            params![
                operation.product_id,
                operation.operation,
                operation.state,
                operation.plan_json,
                operation.message,
                operation.percentage,
                operation.queue_position,
                operation.created_at,
                operation.updated_at,
                operation.completed_at,
            ],
        )?;
        Ok(())
    }

    pub fn installation_operations(&self) -> Result<Vec<InstallationOperationRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT product_id, operation, state, plan_json, message, percentage,
                    queue_position, created_at, updated_at, completed_at
             FROM installation_operations
             ORDER BY queue_position IS NULL, queue_position, created_at, product_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(InstallationOperationRecord {
                product_id: row.get(0)?,
                operation: row.get(1)?,
                state: row.get(2)?,
                plan_json: row.get(3)?,
                message: row.get(4)?,
                percentage: row.get(5)?,
                queue_position: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
                completed_at: row.get(9)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn installation_update_available(&self, game: &InstalledGame) -> Result<bool> {
        if let Some(revision_id) = game.installer_revision_id {
            return self
                .connection
                .query_row(
                    "SELECT EXISTS(
                    SELECT 1 FROM download_revisions installed
                    JOIN download_revisions current ON current.slot_id = installed.slot_id
                    WHERE installed.revision_id = ?1 AND current.currently_offered = 1
                      AND current.revision_id != installed.revision_id
                 )",
                    params![revision_id],
                    |row| row.get(0),
                )
                .map_err(Into::into);
        }

        // Portable markers can identify an installed release even when the
        // installer was retained before revision tracking existed. Match its
        // OS/language context to the currently offered installer and compare
        // the stored version instead of suppressing Update permanently.
        let Some(installed_version) = game.installed_version.as_deref() else {
            return Ok(false);
        };
        let Some(operating_system) = game.installer_operating_system.as_deref() else {
            return Ok(false);
        };
        self.connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1
                    FROM download_slots slot
                    JOIN download_revisions current USING(slot_id)
                    WHERE slot.product_id = ?1
                      AND slot.provider_category = 'installer'
                      AND current.currently_offered = 1
                      AND lower(slot.operating_system) = lower(?2)
                      AND (?3 IS NULL
                           OR lower(slot.language_code) = lower(?3)
                           OR lower(slot.language_name) = lower(?3))
                      AND current.version IS NOT NULL
                      AND lower(current.version) != lower(?4)
                 )",
                params![
                    game.product_id,
                    operating_system,
                    game.installer_language.as_deref(),
                    installed_version,
                ],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn installer_backup_update_available(&self, product_id: i64) -> Result<bool> {
        Ok(self.installer_backup_updates()?.contains(&product_id))
    }

    pub fn installer_backup_updates(&self) -> Result<HashSet<i64>> {
        let mut statement = self.connection.prepare(
            "SELECT DISTINCT product_id FROM (
                    SELECT local.product_id AS product_id FROM managed_files local
                    JOIN download_revisions retained ON retained.revision_id = local.revision_id
                    JOIN download_slots slot ON slot.slot_id = retained.slot_id
                    JOIN download_revisions current ON current.slot_id = retained.slot_id
                    WHERE local.present = 1
                      AND local.artifact_kind = 'installer'
                      AND slot.provider_category = 'installer'
                      AND current.currently_offered = 1
                      AND current.revision_id != retained.revision_id
                      AND EXISTS(
                          SELECT 1 FROM download_parts current_part
                          WHERE current_part.revision_id = current.revision_id
                            AND NOT EXISTS(
                                SELECT 1 FROM managed_files current_file
                                WHERE current_file.part_id = current_part.part_id
                                  AND current_file.present = 1
                            )
                      )
                    UNION ALL
                    SELECT local.product_id AS product_id FROM managed_files local
                    JOIN download_slots slot ON slot.product_id = local.product_id
                    JOIN download_revisions current ON current.slot_id = slot.slot_id
                    WHERE local.present = 1
                      AND local.artifact_kind = 'installer'
                      AND local.revision_id IS NULL
                      AND local.version IS NOT NULL
                      AND slot.provider_category = 'installer'
                      AND lower(slot.operating_system) = lower(local.operating_system)
                      AND (lower(slot.language_code) = lower(local.language)
                           OR lower(slot.language_name) = lower(local.language))
                      AND current.currently_offered = 1
                      AND current.version IS NOT local.version
                      AND EXISTS(
                          SELECT 1 FROM download_parts current_part
                          WHERE current_part.revision_id = current.revision_id
                            AND NOT EXISTS(
                                SELECT 1 FROM managed_files current_file
                                WHERE current_file.part_id = current_part.part_id
                                  AND current_file.present = 1
                            )
                      )
                 )",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, i64>(0))?;
        rows.collect::<rusqlite::Result<HashSet<_>>>()
            .map_err(Into::into)
    }

    pub fn record_game_session(
        &self,
        product_id: i64,
        started_at: i64,
        seconds: u64,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO product_activity(product_id, last_played_at, playtime_seconds, updated_at, last_activity_at)
             VALUES (?1, ?2, ?3, unixepoch(), ?2)
             ON CONFLICT(product_id) DO UPDATE SET
                last_played_at = excluded.last_played_at,
                last_activity_at = MAX(COALESCE(product_activity.last_activity_at, 0), ?2),
                playtime_seconds = product_activity.playtime_seconds + excluded.playtime_seconds,
                updated_at = unixepoch()",
            params![product_id, started_at, seconds],
        )?;
        Ok(())
    }

    pub fn preserve_product_activity(
        &self,
        product_id: i64,
        last_played_at: Option<i64>,
        playtime_seconds: u64,
    ) -> Result<()> {
        if last_played_at.is_none() && playtime_seconds == 0 {
            return Ok(());
        }
        self.connection.execute(
            "INSERT INTO product_activity(product_id, last_played_at, playtime_seconds, updated_at, last_activity_at)
             VALUES (?1, ?2, ?3, unixepoch(), ?2)
             ON CONFLICT(product_id) DO UPDATE SET
                last_played_at = CASE
                    WHEN excluded.last_played_at > product_activity.last_played_at
                    THEN excluded.last_played_at ELSE product_activity.last_played_at END,
                playtime_seconds = MAX(product_activity.playtime_seconds, excluded.playtime_seconds),
                last_activity_at = MAX(COALESCE(product_activity.last_activity_at, 0), COALESCE(?2, 0)),
                updated_at = unixepoch()",
            params![product_id, last_played_at, playtime_seconds],
        )?;
        Ok(())
    }

    pub fn record_product_activity(&self, product_id: i64, timestamp: i64) -> Result<()> {
        self.connection.execute(
            "INSERT INTO product_activity(
                product_id, last_played_at, playtime_seconds, updated_at, last_activity_at)
             VALUES (?1, NULL, 0, unixepoch(), ?2)
             ON CONFLICT(product_id) DO UPDATE SET
                last_activity_at = MAX(COALESCE(product_activity.last_activity_at, 0), ?2),
                updated_at = unixepoch()",
            params![product_id, timestamp],
        )?;
        Ok(())
    }

    pub fn product_activity(&self, product_id: i64) -> Result<(Option<i64>, u64)> {
        self.connection
            .query_row(
                "SELECT last_played_at, playtime_seconds FROM product_activity WHERE product_id = ?1",
                params![product_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map(|activity| activity.unwrap_or((None, 0)))
            .map_err(Into::into)
    }

    /// Loads permanent play activity for the complete library in one query.
    pub fn all_product_activity(&self) -> Result<HashMap<i64, ProductActivity>> {
        let mut statement = self
            .connection
            .prepare("SELECT product_id, last_played_at, last_activity_at, playtime_seconds FROM product_activity")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get(0)?,
                ProductActivity {
                    last_played_at: row.get(1)?,
                    last_activity_at: row.get(2)?,
                    playtime_seconds: row.get(3)?,
                },
            ))
        })?;
        rows.collect::<rusqlite::Result<HashMap<_, _>>>()
            .map_err(Into::into)
    }

    pub(crate) fn revision_has_update(&self, revision_id: i64) -> Result<bool> {
        self.connection
            .query_row(
                "SELECT EXISTS(
                SELECT 1 FROM download_revisions retained
                JOIN download_revisions current ON current.slot_id = retained.slot_id
                WHERE retained.revision_id = ?1 AND current.currently_offered = 1
                  AND current.revision_id != retained.revision_id)",
                params![revision_id],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn mark_sync_stage_started(&self, stage: &str) -> Result<()> {
        self.connection.execute(
            "INSERT INTO sync_stage_outcomes(stage, status, started_at, completed_at, error)
             VALUES (?1, 'running', unixepoch(), NULL, NULL)
             ON CONFLICT(stage) DO UPDATE SET status = 'running',
                 started_at = unixepoch(), completed_at = NULL, error = NULL",
            params![stage],
        )?;
        Ok(())
    }

    pub fn mark_sync_stage_finished(
        &self,
        stage: &str,
        succeeded: bool,
        error: Option<&str>,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO sync_stage_outcomes(stage, status, started_at, completed_at, error)
             VALUES (?1, ?2, unixepoch(), unixepoch(), ?3)
             ON CONFLICT(stage) DO UPDATE SET status = excluded.status,
                 completed_at = excluded.completed_at, error = excluded.error",
            params![stage, if succeeded { "success" } else { "failed" }, error],
        )?;
        Ok(())
    }

    pub fn favorites(&self) -> Result<HashSet<i64>> {
        let mut statement = self
            .connection
            .prepare("SELECT product_id FROM user_game_state WHERE favorite = 1")?;
        Ok(statement
            .query_map([], |row| row.get(0))?
            .filter_map(Result::ok)
            .collect())
    }

    pub fn set_favorite(&self, product_id: i64, favorite: bool) -> Result<()> {
        self.connection.execute(
            "INSERT INTO user_game_state(product_id, favorite) VALUES (?1, ?2)
             ON CONFLICT(product_id) DO UPDATE SET favorite = excluded.favorite",
            params![product_id, favorite],
        )?;
        Ok(())
    }

    pub fn tags(&self) -> Result<HashMap<i64, Vec<String>>> {
        let mut statement = self
            .connection
            .prepare("SELECT product_id, tag FROM custom_tags ORDER BY tag COLLATE NOCASE")?;
        let mut tags: HashMap<i64, Vec<String>> = HashMap::new();
        for row in statement.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })? {
            let (product_id, tag) = row?;
            tags.entry(product_id).or_default().push(tag);
        }
        Ok(tags)
    }

    pub fn add_tag(&self, product_id: i64, tag: &str) -> Result<()> {
        self.connection.execute(
            "INSERT OR IGNORE INTO custom_tags(product_id, tag) VALUES (?1, ?2)",
            params![product_id, tag],
        )?;
        Ok(())
    }

    pub fn cached_profile(&self) -> Result<Option<Profile>> {
        let result = self.connection.query_row(
            "SELECT profile_json FROM account_cache WHERE cache_key = 1",
            [],
            |row| row.get::<_, String>(0),
        );
        match result {
            Ok(json) => Ok(Some(serde_json::from_str(&json)?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn cache_profile(&self, profile: &Profile) -> Result<()> {
        self.connection.execute(
            "INSERT INTO account_cache(cache_key, profile_json, updated_at) VALUES (1, ?1, unixepoch())
             ON CONFLICT(cache_key) DO UPDATE SET profile_json = excluded.profile_json, updated_at = excluded.updated_at",
            params![serde_json::to_string(profile)?],
        )?;
        Ok(())
    }

    pub fn clear_cached_profile(&self) -> Result<()> {
        self.connection
            .execute("DELETE FROM account_cache WHERE cache_key = 1", [])?;
        Ok(())
    }

    pub fn replace_owned_products(&mut self, product_ids: &[i64]) -> Result<()> {
        let transaction = self.connection.transaction()?;
        let synchronized_at = chrono::Utc::now().timestamp();
        transaction.execute("DELETE FROM owned_products", [])?;
        {
            let mut statement = transaction.prepare(
                "INSERT INTO owned_products(product_id, synchronized_at) VALUES (?1, ?2)",
            )?;
            for product_id in product_ids {
                statement.execute(params![product_id, synchronized_at])?;
            }
        }
        transaction.execute(
            "INSERT INTO online_sync_state(sync_key, completed_at) VALUES ('owned_library', ?1)
             ON CONFLICT(sync_key) DO UPDATE SET completed_at = excluded.completed_at",
            params![synchronized_at],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn current_managed_paths(&self, artifacts: &[&RemoteArtifact]) -> Result<Vec<PathBuf>> {
        let mut paths = Vec::with_capacity(artifacts.len());
        for artifact in artifacts {
            let Some(file_id) = artifact.provider_file_id.as_deref() else {
                return Ok(Vec::new());
            };
            let path = self
                .connection
                .query_row(
                    "SELECT mf.path FROM managed_files mf
                     JOIN download_parts p ON p.part_id = mf.part_id
                     JOIN download_revisions r ON r.revision_id = p.revision_id
                     WHERE mf.product_id = ?1 AND mf.present = 1
                       AND r.currently_offered = 1 AND p.provider_file_id = ?2
                     LIMIT 1",
                    params![artifact.product_id, file_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            let Some(path) = path else {
                return Ok(Vec::new());
            };
            let path = PathBuf::from(path);
            if !path.is_file() {
                return Ok(Vec::new());
            }
            paths.push(path);
        }
        Ok(paths)
    }

    pub fn owned_library_status(&self) -> Result<(usize, Option<i64>)> {
        let count = self
            .connection
            .query_row("SELECT COUNT(*) FROM owned_products", [], |row| {
                row.get::<_, i64>(0)
            })? as usize;
        let synchronized_at = self.connection.query_row(
            "SELECT completed_at FROM online_sync_state WHERE sync_key = 'owned_library'",
            [],
            |row| row.get::<_, i64>(0),
        );
        match synchronized_at {
            Ok(value) => Ok((count, Some(value))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok((count, None)),
            Err(error) => Err(error.into()),
        }
    }

    pub fn cached_online_games(&self) -> Result<Vec<Game>> {
        let result = self.connection.query_row(
            "SELECT games_json FROM online_library_cache WHERE cache_key = 1",
            [],
            |row| row.get::<_, String>(0),
        );
        match result {
            Ok(json) => Ok(serde_json::from_str(&json)?),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(Vec::new()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn enrichment_observation(
        &self,
        product_id: i64,
        source: &str,
    ) -> Result<Option<(String, i64)>> {
        let result = self.connection.query_row(
            "SELECT status, checked_at FROM enrichment_observations
             WHERE product_id = ?1 AND source = ?2",
            params![product_id, source],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );
        match result {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn record_enrichment_observation(
        &self,
        product_id: i64,
        source: &str,
        status: &str,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO enrichment_observations(product_id, source, status, checked_at)
             VALUES (?1, ?2, ?3, unixepoch())
             ON CONFLICT(product_id, source) DO UPDATE SET
                status = excluded.status, checked_at = excluded.checked_at",
            params![product_id, source, status],
        )?;
        Ok(())
    }

    pub fn cached_product_metadata(&self, product_id: i64) -> Result<Option<ProductMetadata>> {
        let result = self.connection.query_row(
            "SELECT metadata_json FROM products WHERE product_id = ?1",
            params![product_id],
            |row| row.get::<_, String>(0),
        );
        match result {
            Ok(json) => Ok(Some(serde_json::from_str(&json)?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn normalized_games(&self) -> Result<Vec<Game>> {
        let mut statement = self.connection.prepare(
            "SELECT product_id, parent_product_id, product_type, slug, title, release_date,
                    description, changelog, metadata_json, links_json, media_json, currently_owned
             FROM products
             WHERE (product_type = 'game' AND currently_owned = 1)
                OR (product_type = 'dlc' AND EXISTS (
                    SELECT 1 FROM product_relationships relationship
                    JOIN products parent
                      ON parent.product_id = relationship.parent_product_id
                    WHERE relationship.child_product_id = products.product_id
                      AND relationship.relationship = 'dlc'
                      AND parent.product_type = 'game'
                      AND parent.currently_owned = 1
                ))
             ORDER BY title COLLATE NOCASE",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, bool>(11)?,
            ))
        })?;
        let mut bases = Vec::new();
        let mut dlcs = Vec::new();
        for row in rows {
            let (
                id,
                parent_id,
                product_type,
                slug,
                title,
                release_date,
                description,
                changelog,
                metadata_json,
                links_json,
                media_json,
                currently_owned,
            ) = row?;
            let metadata: crate::domain::ProductMetadata = serde_json::from_str(&metadata_json)?;
            let description = if description.trim().is_empty() {
                metadata.store_description.clone().unwrap_or_default()
            } else {
                description
            };
            let links = serde_json::from_str(&links_json)?;
            let media: serde_json::Value = serde_json::from_str(&media_json)?;
            let revisions = self.load_current_download_revisions(id)?;
            let artifacts = revisions_to_artifacts(&revisions);
            let platforms = platforms_from_artifacts(&artifacts);
            let release_date = release_date
                .and_then(|timestamp| chrono::DateTime::from_timestamp(timestamp, 0))
                .map(|value| value.fixed_offset());
            let artwork = media_path(&media, "artwork");
            let detail_artwork = media_path(&media, "detail_artwork");
            let hero_logo = media_path(&media, "hero_logo");
            let icon = media_path(&media, "icon");
            let screenshots = media
                .get("screenshots")
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok())
                .unwrap_or_default();
            let builds = self.load_galaxy_builds(id)?;
            if product_type == "dlc" {
                dlcs.push((
                    parent_id,
                    crate::domain::Dlc {
                        product_id: id,
                        owned: currently_owned,
                        slug,
                        title,
                        release_date,
                        description,
                        changelog,
                        platforms,
                        languages: metadata
                            .localizations
                            .iter()
                            .map(|value| value.name.clone())
                            .collect(),
                        metadata,
                        galaxy_builds: builds,
                        location: PathBuf::new(),
                        artwork,
                        detail_artwork,
                        hero_logo,
                        icon,
                        screenshots,
                        links,
                        installers: Vec::new(),
                        extras: Vec::new(),
                        remote_artifacts: artifacts,
                        disk_usage: 0,
                    },
                ));
            } else if product_type == "game" {
                bases.push(Game {
                    product_id: id,
                    slug,
                    title,
                    release_date,
                    description,
                    changelog,
                    platforms,
                    features: metadata
                        .features
                        .iter()
                        .map(|value| value.name.clone())
                        .collect(),
                    languages: metadata
                        .localizations
                        .iter()
                        .map(|value| value.name.clone())
                        .collect(),
                    metadata,
                    galaxy_builds: builds,
                    location: PathBuf::new(),
                    artwork,
                    detail_artwork,
                    hero_logo,
                    icon,
                    screenshots,
                    links,
                    installers: Vec::new(),
                    patches: Vec::new(),
                    extras: Vec::new(),
                    remote_artifacts: artifacts,
                    dlc_count: 0,
                    dlcs: Vec::new(),
                    disk_usage: 0,
                });
            }
        }
        for (parent_id, dlc) in dlcs {
            if let Some(parent) =
                parent_id.and_then(|id| bases.iter_mut().find(|game| game.product_id == id))
            {
                parent.dlcs.push(dlc);
            }
        }
        for game in &mut bases {
            game.dlcs.sort_by_key(|dlc| dlc.title.to_lowercase());
            game.dlc_count = game.dlcs.len();
        }
        Ok(bases)
    }

    pub fn cache_online_games(&self, games: &[Game]) -> Result<()> {
        self.connection.execute(
            "INSERT INTO online_library_cache(cache_key, games_json, updated_at)
             VALUES (1, ?1, unixepoch())
             ON CONFLICT(cache_key) DO UPDATE SET games_json = excluded.games_json,
                 updated_at = excluded.updated_at",
            params![serde_json::to_string(games)?],
        )?;
        Ok(())
    }

    pub fn upsert_normalized_library(&self, games: &[Game]) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute("UPDATE products SET currently_owned = 0", [])?;
        transaction.execute("DELETE FROM product_relationships", [])?;
        for game in games {
            upsert_product_row(&transaction, game, None, "game")?;
            insert_edition_relationships(&transaction, game.product_id, &game.metadata.editions)?;
            for dlc in &game.dlcs {
                upsert_dlc_row(&transaction, dlc, game.product_id)?;
                insert_edition_relationships(&transaction, dlc.product_id, &dlc.metadata.editions)?;
                transaction.execute(
                    "INSERT INTO product_relationships(
                        parent_product_id, child_product_id, relationship, source
                     ) VALUES (?1, ?2, 'dlc', 'product_api')
                     ON CONFLICT(parent_product_id, child_product_id, relationship) DO UPDATE SET
                        source = excluded.source",
                    params![game.product_id, dlc.product_id],
                )?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn cache_download_manifest(
        &self,
        product_id: i64,
        artifacts: &[crate::domain::RemoteArtifact],
    ) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO download_manifest_cache(product_id, artifacts_json, updated_at)
             VALUES (?1, ?2, unixepoch())
             ON CONFLICT(product_id) DO UPDATE SET
                 artifacts_json = excluded.artifacts_json, updated_at = excluded.updated_at",
            params![product_id, serde_json::to_string(artifacts)?],
        )?;
        transaction.execute(
            "UPDATE download_artifact_catalog
             SET currently_offered = 0, retired_at = COALESCE(retired_at, unixepoch())
             WHERE product_id = ?1",
            params![product_id],
        )?;
        for artifact in artifacts {
            let artifact_id = catalog_artifact_id(artifact);
            transaction.execute(
                "INSERT INTO download_artifact_catalog(
                    artifact_id, product_id, artifact_json, currently_offered,
                    first_seen_at, last_seen_at, retired_at
                 ) VALUES (?1, ?2, ?3, 1, unixepoch(), unixepoch(), NULL)
                 ON CONFLICT(artifact_id) DO UPDATE SET
                    artifact_json = excluded.artifact_json,
                    currently_offered = 1,
                    last_seen_at = unixepoch(),
                    retired_at = NULL",
                params![artifact_id, product_id, serde_json::to_string(artifact)?],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn observe_download_manifest(
        &self,
        product_id: i64,
        artifacts: &[RemoteArtifact],
    ) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "UPDATE download_revisions SET currently_offered = 0,
                 retired_at = COALESCE(retired_at, unixepoch())
             WHERE slot_id IN (SELECT slot_id FROM download_slots WHERE product_id = ?1)",
            params![product_id],
        )?;

        let mut groups: std::collections::BTreeMap<
            (DownloadCategory, String),
            Vec<&RemoteArtifact>,
        > = std::collections::BTreeMap::new();
        for artifact in artifacts {
            let category = artifact.provider_category.unwrap_or(match artifact.kind {
                ArtifactKind::Installer => DownloadCategory::Installer,
                ArtifactKind::Patch => DownloadCategory::Patch,
                ArtifactKind::Extra => DownloadCategory::Bonus,
            });
            let group_id = artifact.provider_group_id.clone().unwrap_or_else(|| {
                format!(
                    "legacy:{}:{}:{}",
                    artifact.kind.as_str(),
                    artifact.operating_system.as_deref().unwrap_or("any"),
                    artifact.language.as_deref().unwrap_or("neutral")
                )
            });
            groups
                .entry((category, group_id))
                .or_default()
                .push(artifact);
        }

        for ((category, group_id), mut parts) in groups {
            parts.sort_by_key(|part| part.part_number.unwrap_or(1));
            let first = parts[0];
            transaction.execute(
                "INSERT INTO download_slots(
                    product_id, provider_group_id, provider_category, name,
                    operating_system, language_code, language_name, first_seen_at, last_seen_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, unixepoch(), unixepoch())
                 ON CONFLICT(product_id, provider_category, provider_group_id) DO UPDATE SET
                    name = excluded.name, operating_system = excluded.operating_system,
                    language_code = excluded.language_code, language_name = excluded.language_name,
                    last_seen_at = unixepoch()",
                params![
                    product_id,
                    group_id,
                    category.as_str(),
                    first.name,
                    first.operating_system,
                    first.language.as_deref().map(language_code),
                    first.language,
                ],
            )?;
            let slot_id: i64 = transaction.query_row(
                "SELECT slot_id FROM download_slots
                 WHERE product_id = ?1 AND provider_category = ?2 AND provider_group_id = ?3",
                params![product_id, category.as_str(), group_id],
                |row| row.get(0),
            )?;
            let fingerprint = manifest_fingerprint(product_id, category, &group_id, &parts);
            let effective_fingerprint = transaction
                .query_row(
                    "SELECT manifest_fingerprint FROM download_revisions
                     WHERE slot_id = ?1 AND (manifest_fingerprint = ?2 OR manifest_fingerprint LIKE ?3)
                     ORDER BY CASE WHEN manifest_fingerprint = ?2 THEN 1 ELSE 0 END,
                              currently_offered DESC, last_seen_at DESC LIMIT 1",
                    params![slot_id, fingerprint, format!("{fingerprint}:checksum:%")],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .unwrap_or(fingerprint);
            transaction.execute(
                "INSERT INTO download_revisions(
                    slot_id, version, total_size, manifest_fingerprint, currently_offered,
                    first_seen_at, last_seen_at, retired_at
                 ) VALUES (?1, ?2, ?3, ?4, 1, unixepoch(), unixepoch(), NULL)
                 ON CONFLICT(slot_id, manifest_fingerprint) DO UPDATE SET
                    currently_offered = 1, last_seen_at = unixepoch(), retired_at = NULL",
                params![
                    slot_id,
                    first.version,
                    sum_sizes(&parts),
                    effective_fingerprint,
                ],
            )?;
            let revision_id: i64 = transaction.query_row(
                "SELECT revision_id FROM download_revisions
                 WHERE slot_id = ?1 AND manifest_fingerprint = ?2",
                params![slot_id, effective_fingerprint],
                |row| row.get(0),
            )?;
            for (index, artifact) in parts.iter().enumerate() {
                let file_id = artifact.provider_file_id.clone().unwrap_or_else(|| {
                    format!(
                        "legacy-{}",
                        artifact.part_number.unwrap_or(index as u32 + 1)
                    )
                });
                transaction.execute(
                    "INSERT INTO download_parts(
                        revision_id, provider_file_id, part_index, expected_size, downlink
                     ) VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(revision_id, provider_file_id) DO UPDATE SET
                        part_index = excluded.part_index, expected_size = excluded.expected_size,
                        downlink = excluded.downlink",
                    params![
                        revision_id,
                        file_id,
                        artifact.part_number.unwrap_or(index as u32 + 1),
                        artifact.size_bytes,
                        artifact.download_path,
                    ],
                )?;
                let part_id: i64 = transaction.query_row(
                    "SELECT part_id FROM download_parts
                     WHERE revision_id = ?1 AND provider_file_id = ?2",
                    params![revision_id, file_id],
                    |row| row.get(0),
                )?;
                transaction.execute(
                    "UPDATE managed_files SET revision_id = ?2, part_id = ?3,
                         provider_file_id = ?4, updated_at = unixepoch()
                     WHERE product_id = ?1 AND revision_id IS NULL
                       AND artifact_path IS NOT NULL AND artifact_path LIKE '%' || ?4
                       AND COALESCE(version, '') = COALESCE(?5, '')",
                    params![
                        artifact.product_id,
                        revision_id,
                        part_id,
                        file_id,
                        artifact.version
                    ],
                )?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn observe_galaxy_builds(
        &self,
        product_id: i64,
        operating_system: &str,
        builds: &[GalaxyBuild],
    ) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "UPDATE galaxy_builds SET currently_returned = 0
             WHERE product_id = ?1 AND operating_system = ?2",
            params![product_id, operating_system],
        )?;
        for build in builds {
            transaction.execute(
                "INSERT INTO galaxy_builds(
                    build_id, product_id, operating_system, version, branch, tags_json, public,
                    generation, repository_url, repository_id, published_at, currently_returned,
                    first_seen_at, last_seen_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1,
                    unixepoch(), unixepoch())
                 ON CONFLICT(build_id) DO UPDATE SET
                    version = excluded.version, branch = excluded.branch,
                    tags_json = excluded.tags_json, public = excluded.public,
                    repository_url = excluded.repository_url,
                    repository_id = excluded.repository_id, published_at = excluded.published_at,
                    currently_returned = 1, last_seen_at = unixepoch()",
                params![
                    build.build_id,
                    product_id,
                    operating_system,
                    build.version,
                    build.branch,
                    serde_json::to_string(&build.tags)?,
                    build.public,
                    build.generation,
                    build.repository_url,
                    build.repository_id,
                    build.published_at,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn observe_part_checksum(&self, artifact: &RemoteArtifact, checksum: &str) -> Result<()> {
        let (Some(group_id), Some(file_id), Some(category)) = (
            artifact.provider_group_id.as_deref(),
            artifact.provider_file_id.as_deref(),
            artifact.provider_category,
        ) else {
            return Ok(());
        };
        let existing = self
            .connection
            .query_row(
                "SELECT r.revision_id, r.slot_id, r.version, r.total_size, r.manifest_fingerprint,
                    p.part_id, p.checksum
             FROM download_slots s JOIN download_revisions r USING(slot_id)
             JOIN download_parts p USING(revision_id)
             WHERE s.product_id = ?1 AND s.provider_category = ?2 AND s.provider_group_id = ?3
               AND p.provider_file_id = ?4 AND r.currently_offered = 1
             ORDER BY r.last_seen_at DESC LIMIT 1",
                params![artifact.product_id, category.as_str(), group_id, file_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, Option<String>>(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((revision_id, slot_id, version, total_size, fingerprint, part_id, old_checksum)) =
            existing
        else {
            return Ok(());
        };
        if old_checksum
            .as_deref()
            .is_none_or(|old| old.eq_ignore_ascii_case(checksum))
        {
            self.connection.execute(
                "UPDATE download_parts SET checksum = ?2, checksum_fetched_at = unixepoch()
                 WHERE part_id = ?1",
                params![part_id, checksum],
            )?;
            return Ok(());
        }
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "UPDATE download_revisions SET currently_offered = 0,
                 retired_at = COALESCE(retired_at, unixepoch()) WHERE revision_id = ?1",
            params![revision_id],
        )?;
        let checksum_fingerprint = format!(
            "{}:checksum:{:x}",
            fingerprint,
            Sha256::digest(format!("{file_id}:{checksum}").as_bytes())
        );
        transaction.execute(
            "INSERT INTO download_revisions(
                slot_id, version, total_size, manifest_fingerprint, currently_offered,
                first_seen_at, last_seen_at, retired_at
             ) VALUES (?1, ?2, ?3, ?4, 1, unixepoch(), unixepoch(), NULL)",
            params![slot_id, version, total_size, checksum_fingerprint],
        )?;
        let new_revision = transaction.last_insert_rowid();
        transaction.execute(
            "INSERT INTO download_parts(
                revision_id, provider_file_id, part_index, expected_size, downlink,
                checksum, checksum_fetched_at
             ) SELECT ?1, provider_file_id, part_index, expected_size, downlink,
                CASE WHEN provider_file_id = ?2 THEN ?3 ELSE checksum END,
                CASE WHEN provider_file_id = ?2 THEN unixepoch() ELSE checksum_fetched_at END
             FROM download_parts WHERE revision_id = ?4",
            params![new_revision, file_id, checksum, revision_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn mark_managed_file_verified(
        &self,
        path: &std::path::Path,
        artifact: &RemoteArtifact,
        checksum: &str,
    ) -> Result<()> {
        let identity = match (
            artifact.provider_group_id.as_deref(),
            artifact.provider_file_id.as_deref(),
            artifact.provider_category,
        ) {
            (Some(group), Some(file), Some(category)) => self
                .connection
                .query_row(
                    "SELECT r.revision_id, p.part_id FROM download_slots s
                 JOIN download_revisions r USING(slot_id) JOIN download_parts p USING(revision_id)
                 WHERE s.product_id = ?1 AND s.provider_category = ?2 AND s.provider_group_id = ?3
                   AND p.provider_file_id = ?4 AND r.currently_offered = 1 LIMIT 1",
                    params![artifact.product_id, category.as_str(), group, file],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .optional()?,
            _ => None,
        };
        self.connection.execute(
            "UPDATE managed_files SET gog_checksum = ?2, verified_at = unixepoch(),
                 revision_id = COALESCE(?3, revision_id), part_id = COALESCE(?4, part_id),
                 provider_file_id = COALESCE(?5, provider_file_id), updated_at = unixepoch()
             WHERE path = ?1",
            params![
                path.display().to_string(),
                checksum,
                identity.map(|value| value.0),
                identity.map(|value| value.1),
                artifact.provider_file_id,
            ],
        )?;
        Ok(())
    }

    pub fn load_galaxy_builds(&self, product_id: i64) -> Result<Vec<GalaxyBuild>> {
        let mut statement = self.connection.prepare(
            "SELECT build_id, operating_system, version, branch, tags_json, public, generation,
                    repository_url, repository_id, published_at, currently_returned,
                    first_seen_at, last_seen_at
             FROM galaxy_builds WHERE product_id = ?1
             ORDER BY published_at DESC, build_id DESC",
        )?;
        let rows = statement.query_map(params![product_id], |row| {
            let tags: String = row.get(4)?;
            Ok(GalaxyBuild {
                build_id: row.get(0)?,
                product_id,
                operating_system: row.get(1)?,
                version: row.get(2)?,
                branch: row.get(3)?,
                tags: serde_json::from_str(&tags).unwrap_or_default(),
                public: row.get(5)?,
                generation: row.get(6)?,
                repository_url: row.get(7)?,
                repository_id: row.get(8)?,
                published_at: row.get(9)?,
                currently_returned: row.get(10)?,
                first_seen_at: row.get(11)?,
                last_seen_at: row.get(12)?,
            })
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    #[allow(dead_code)]
    pub fn load_current_download_revisions(
        &self,
        product_id: i64,
    ) -> Result<Vec<DownloadRevision>> {
        self.load_download_revisions(product_id, true)
    }

    pub fn load_all_download_revisions(&self, product_id: i64) -> Result<Vec<DownloadRevision>> {
        self.load_download_revisions(product_id, false)
    }

    fn load_download_revisions(
        &self,
        product_id: i64,
        current_only: bool,
    ) -> Result<Vec<DownloadRevision>> {
        let mut statement = self.connection.prepare(
            "SELECT r.revision_id, s.slot_id, s.provider_group_id, s.provider_category,
                    s.name, s.operating_system, s.language_code, s.language_name,
                    r.version, r.total_size, r.manifest_fingerprint, r.currently_offered,
                    r.first_seen_at, r.last_seen_at, r.retired_at
             FROM download_revisions r JOIN download_slots s USING(slot_id)
             WHERE s.product_id = ?1 AND (?2 = 0 OR r.currently_offered = 1)
             ORDER BY s.provider_category, s.slot_id",
        )?;
        let rows = statement.query_map(params![product_id, current_only], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<i64>>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, bool>(11)?,
                row.get::<_, i64>(12)?,
                row.get::<_, i64>(13)?,
                row.get::<_, Option<i64>>(14)?,
            ))
        })?;
        let mut revisions = Vec::new();
        for row in rows {
            let (
                revision_id,
                slot_id,
                group_id,
                category,
                name,
                os,
                language_code,
                language_name,
                version,
                total_size,
                fingerprint,
                current,
                first_seen,
                last_seen,
                retired_at,
            ) = row?;
            let mut parts_statement = self.connection.prepare(
                "SELECT part_id, provider_file_id, part_index, expected_size, downlink,
                        checksum, checksum_fetched_at
                 FROM download_parts WHERE revision_id = ?1 ORDER BY part_index",
            )?;
            let parts = parts_statement
                .query_map(params![revision_id], |row| {
                    Ok(crate::domain::DownloadPart {
                        part_id: row.get(0)?,
                        revision_id,
                        provider_file_id: row.get(1)?,
                        part_index: row.get::<_, i64>(2)? as u32,
                        expected_size: row.get::<_, Option<i64>>(3)?.map(|value| value as u64),
                        downlink: row.get(4)?,
                        checksum: row.get(5)?,
                        checksum_fetched_at: row.get(6)?,
                    })
                })?
                .filter_map(Result::ok)
                .collect();
            revisions.push(DownloadRevision {
                revision_id,
                slot_id,
                product_id,
                provider_group_id: group_id,
                provider_category: parse_download_category(&category),
                name,
                operating_system: os,
                language_code,
                language_name,
                version,
                total_size: total_size.map(|value| value as u64),
                manifest_fingerprint: fingerprint,
                currently_offered: current,
                first_seen_at: first_seen,
                last_seen_at: last_seen,
                retired_at,
                parts,
            });
        }
        Ok(revisions)
    }

    pub fn artifact_catalog(&self, product_id: i64) -> Result<Vec<CatalogArtifact>> {
        let mut statement = self.connection.prepare(
            "SELECT artifact_json, currently_offered, first_seen_at, last_seen_at
             FROM download_artifact_catalog WHERE product_id = ?1
             ORDER BY currently_offered DESC, last_seen_at DESC, artifact_id",
        )?;
        let rows = statement.query_map(params![product_id], |row| {
            let json: String = row.get(0)?;
            Ok((json, row.get::<_, bool>(1)?, row.get(2)?, row.get(3)?))
        })?;
        Ok(rows
            .filter_map(Result::ok)
            .filter_map(|(json, currently_offered, first_seen_at, last_seen_at)| {
                serde_json::from_str(&json)
                    .ok()
                    .map(|artifact| CatalogArtifact {
                        artifact,
                        currently_offered,
                        first_seen_at,
                        last_seen_at,
                    })
            })
            .collect())
    }

    pub fn retired_artifact_for_file(
        &self,
        path: &std::path::Path,
    ) -> Result<Option<RemoteArtifact>> {
        let result = self.connection.query_row(
            "SELECT product_id, artifact_kind, version, artifact_path
             FROM managed_files WHERE path = ?1",
            params![path.display().to_string()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        );
        match result {
            Ok((product_id, kind, version, artifact_path)) => {
                let endpoint = artifact_path.as_deref().and_then(download_endpoint_name);
                Ok(self
                    .artifact_catalog(product_id)?
                    .into_iter()
                    .filter(|entry| !entry.currently_offered)
                    .map(|entry| entry.artifact)
                    .find(|artifact| {
                        artifact.kind.as_str() == kind
                            && artifact.version == version
                            && endpoint.is_some_and(|endpoint| {
                                download_endpoint_name(&artifact.download_path) == Some(endpoint)
                            })
                    }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn save_download_job(&self, job: &DownloadJobUpdate<'_>) -> Result<()> {
        self.connection.execute(
            "INSERT INTO download_jobs(
                job_id, product_id, title, artifacts_json, destination, state,
                bytes_downloaded, total_bytes, completed_files_json, error, updated_at,
                queue_position, created_at, completed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, unixepoch(),
                CASE WHEN ?6 = 'complete' THEN NULL ELSE
                    COALESCE((SELECT MAX(queue_position) + 1 FROM download_jobs), 1) END,
                unixepoch(), CASE WHEN ?6 = 'complete' THEN unixepoch() ELSE NULL END)
             ON CONFLICT(job_id) DO UPDATE SET
                title = excluded.title, artifacts_json = excluded.artifacts_json,
                destination = excluded.destination, state = excluded.state,
                bytes_downloaded = CASE WHEN excluded.state = 'queued'
                    THEN download_jobs.bytes_downloaded ELSE excluded.bytes_downloaded END,
                total_bytes = excluded.total_bytes,
                completed_files_json = CASE WHEN excluded.state = 'queued'
                    THEN download_jobs.completed_files_json ELSE excluded.completed_files_json END,
                error = excluded.error, updated_at = excluded.updated_at,
                status_message = CASE WHEN excluded.state = 'complete' THEN NULL
                    ELSE download_jobs.status_message END,
                completed_at = CASE WHEN excluded.state = 'complete' THEN unixepoch()
                    ELSE download_jobs.completed_at END",
            params![
                job.job_id,
                job.product_id,
                job.title,
                serde_json::to_string(job.artifacts)?,
                job.destination.display().to_string(),
                job.state.as_str(),
                job.bytes_downloaded as i64,
                job.total_bytes.map(|value| value as i64),
                serde_json::to_string(job.completed_files)?,
                job.error,
            ],
        )?;
        Ok(())
    }

    pub fn download_job(&self, job_id: &str) -> Result<Option<DownloadJobRecord>> {
        let result = self.connection.query_row(
            "SELECT job_id, product_id, title, artifacts_json, state, destination,
                    bytes_downloaded, total_bytes, completed_files_json, error, updated_at,
                    status_message, queue_position, retry_started_at, next_retry_at, created_at, completed_at
             FROM download_jobs WHERE job_id = ?1",
            params![job_id],
            |row| {
                let artifacts: String = row.get(3)?;
                let files: String = row.get(8)?;
                Ok(DownloadJobRecord {
                    job_id: row.get(0)?,
                    product_id: row.get(1)?,
                    title: row.get(2)?,
                    artifacts: serde_json::from_str(&artifacts).unwrap_or_default(),
                    state: DownloadState::from_database(&row.get::<_, String>(4)?),
                    destination: PathBuf::from(row.get::<_, String>(5)?),
                    bytes_downloaded: row.get::<_, i64>(6)? as u64,
                    total_bytes: row.get::<_, Option<i64>>(7)?.map(|value| value as u64),
                    completed_files: serde_json::from_str(&files).unwrap_or_default(),
                    error: row.get(9)?,
                    updated_at: row.get(10)?,
                    status_message: row.get(11)?,
                    queue_position: row.get(12)?,
                    retry_started_at: row.get(13)?,
                    next_retry_at: row.get(14)?,
                    created_at: row.get(15)?,
                    completed_at: row.get(16)?,
                })
            },
        );
        match result {
            Ok(job) => Ok(Some(job)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn download_jobs(&self) -> Result<Vec<DownloadJobRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT job_id, product_id, title, artifacts_json, state, destination,
                    bytes_downloaded, total_bytes, completed_files_json, error, updated_at,
                    status_message, queue_position, retry_started_at, next_retry_at, created_at, completed_at
             FROM download_jobs ORDER BY CASE WHEN queue_position IS NULL THEN 1 ELSE 0 END,
                 queue_position, rowid",
        )?;
        let jobs = statement
            .query_map([], |row| {
                let artifacts: String = row.get(3)?;
                let files: String = row.get(8)?;
                Ok(DownloadJobRecord {
                    job_id: row.get(0)?,
                    product_id: row.get(1)?,
                    title: row.get(2)?,
                    artifacts: serde_json::from_str(&artifacts).unwrap_or_default(),
                    state: DownloadState::from_database(&row.get::<_, String>(4)?),
                    destination: PathBuf::from(row.get::<_, String>(5)?),
                    bytes_downloaded: row.get::<_, i64>(6)? as u64,
                    total_bytes: row.get::<_, Option<i64>>(7)?.map(|value| value as u64),
                    completed_files: serde_json::from_str(&files).unwrap_or_default(),
                    error: row.get(9)?,
                    updated_at: row.get(10)?,
                    status_message: row.get(11)?,
                    queue_position: row.get(12)?,
                    retry_started_at: row.get(13)?,
                    next_retry_at: row.get(14)?,
                    created_at: row.get(15)?,
                    completed_at: row.get(16)?,
                })
            })?
            .filter_map(Result::ok)
            .collect();
        Ok(jobs)
    }

    pub fn delete_download_job(&self, job_id: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM download_jobs WHERE job_id = ?1",
            params![job_id],
        )?;
        Ok(())
    }

    pub fn set_download_job_status(&self, job_id: &str, message: Option<&str>) -> Result<()> {
        self.connection.execute(
            "UPDATE download_jobs SET status_message = ?2, updated_at = unixepoch()
             WHERE job_id = ?1",
            params![job_id, message],
        )?;
        Ok(())
    }

    pub fn set_download_job_failure(&self, job_id: &str, error: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE download_jobs
             SET state = 'failed', error = ?2, status_message = NULL, updated_at = unixepoch()
             WHERE job_id = ?1",
            params![job_id, error],
        )?;
        Ok(())
    }

    pub fn set_queued_download_status(&self, message: Option<&str>) -> Result<()> {
        self.connection.execute(
            "UPDATE download_jobs SET status_message = ?1, updated_at = unixepoch()
             WHERE state IN ('queued', 'downloading')",
            params![message],
        )?;
        Ok(())
    }

    pub fn recover_download_job(&self, job_id: &str, bytes_downloaded: u64) -> Result<()> {
        self.connection.execute(
            "UPDATE download_jobs
             SET state = 'queued', bytes_downloaded = ?2, error = NULL,
                 status_message = 'Waiting to resume', retry_started_at = NULL,
                 next_retry_at = NULL, updated_at = unixepoch()
             WHERE job_id = ?1 AND state IN ('queued', 'downloading')",
            params![job_id, bytes_downloaded as i64],
        )?;
        Ok(())
    }

    pub fn prune_completed_download_history(&self, older_than: i64) -> Result<usize> {
        Ok(self.connection.execute(
            "DELETE FROM download_jobs
             WHERE state = 'complete' AND COALESCE(completed_at, updated_at) < ?1",
            params![older_than],
        )?)
    }

    pub fn relocate_download_job(
        &self,
        job_id: &str,
        destination: &std::path::Path,
        completed_files: &[PathBuf],
    ) -> Result<()> {
        self.connection.execute(
            "UPDATE download_jobs
             SET destination = ?2, completed_files_json = ?3, updated_at = unixepoch()
             WHERE job_id = ?1",
            params![
                job_id,
                destination.display().to_string(),
                serde_json::to_string(completed_files)?,
            ],
        )?;
        Ok(())
    }

    pub fn replace_managed_files(&mut self, files: &[ManagedFileRecord]) -> Result<()> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "UPDATE managed_files
             SET present = 0, artifact_id = NULL, updated_at = unixepoch()",
            [],
        )?;
        {
            let mut statement = transaction.prepare(
                "INSERT INTO managed_files(path, product_id, product_slug, artifact_kind,
                    operating_system, language, filename, size, artifact_path, matched, present,
                    updated_at, artifact_id, job_id, version, expected_size, gog_checksum, verified_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, unixepoch(),
                    ?11, ?12, ?13, ?14, ?15, ?16)
                 ON CONFLICT(path) DO UPDATE SET product_id=excluded.product_id,
                    product_slug=excluded.product_slug, artifact_kind=excluded.artifact_kind,
                    operating_system=excluded.operating_system, language=excluded.language,
                    filename=excluded.filename, size=excluded.size, artifact_path=excluded.artifact_path,
                    matched=excluded.matched, present=1, updated_at=excluded.updated_at,
                    artifact_id=COALESCE(excluded.artifact_id, managed_files.artifact_id),
                    job_id=COALESCE(managed_files.job_id, excluded.job_id),
                    version=excluded.version,
                    expected_size=excluded.expected_size,
                    gog_checksum=COALESCE(managed_files.gog_checksum, excluded.gog_checksum),
                    verified_at=COALESCE(managed_files.verified_at, excluded.verified_at)",
            )?;
            let mut assigned_artifacts = HashSet::new();
            for file in files {
                let artifact_id = file
                    .artifact_id
                    .as_deref()
                    .filter(|id| assigned_artifacts.insert((*id).to_owned()));
                let unique_match = file.artifact_id.is_none() || artifact_id.is_some();
                statement.execute(params![
                    file.path.display().to_string(),
                    file.product_id,
                    file.product_slug,
                    file.kind.as_str(),
                    file.operating_system,
                    file.language,
                    file.filename,
                    file.size as i64,
                    unique_match
                        .then_some(file.artifact_path.as_deref())
                        .flatten(),
                    file.matched && unique_match,
                    artifact_id,
                    file.job_id,
                    file.version,
                    file.expected_size.map(|size| size as i64),
                    file.gog_checksum,
                    file.verified_at,
                ])?;
            }
        }
        transaction.execute_batch(
            "UPDATE managed_files
             SET revision_id = (
                    SELECT r.revision_id FROM download_parts p
                    JOIN download_revisions r USING(revision_id)
                    WHERE p.downlink = managed_files.artifact_path
                      AND r.version IS managed_files.version
                      AND (managed_files.expected_size IS NULL
                           OR p.expected_size = managed_files.expected_size)
                    ORDER BY r.currently_offered DESC, r.last_seen_at DESC LIMIT 1
                 ),
                 part_id = (
                    SELECT p.part_id FROM download_parts p
                    JOIN download_revisions r USING(revision_id)
                    WHERE p.downlink = managed_files.artifact_path
                      AND r.version IS managed_files.version
                      AND (managed_files.expected_size IS NULL
                           OR p.expected_size = managed_files.expected_size)
                    ORDER BY r.currently_offered DESC, r.last_seen_at DESC LIMIT 1
                 ),
                 provider_file_id = (
                    SELECT p.provider_file_id FROM download_parts p
                    JOIN download_revisions r USING(revision_id)
                    WHERE p.downlink = managed_files.artifact_path
                      AND r.version IS managed_files.version
                      AND (managed_files.expected_size IS NULL
                           OR p.expected_size = managed_files.expected_size)
                    ORDER BY r.currently_offered DESC, r.last_seen_at DESC LIMIT 1
                 )
             WHERE present = 1 AND artifact_path IS NOT NULL;",
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn managed_files(&self) -> Result<Vec<ManagedFileRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT path, product_id, product_slug, artifact_kind, operating_system, language,
                    filename, size, artifact_path, matched, present, artifact_id, job_id, version,
                    expected_size, gog_checksum, verified_at, revision_id, part_id,
                    provider_file_id
             FROM managed_files ORDER BY path",
        )?;
        let rows = statement.query_map([], |row| {
            let kind: String = row.get(3)?;
            let kind = match kind.as_str() {
                "installer" => ArtifactKind::Installer,
                "patch" => ArtifactKind::Patch,
                _ => ArtifactKind::Extra,
            };
            Ok(ManagedFileRecord {
                path: PathBuf::from(row.get::<_, String>(0)?),
                product_id: row.get(1)?,
                product_slug: row.get(2)?,
                kind,
                operating_system: row.get(4)?,
                language: row.get(5)?,
                filename: row.get(6)?,
                size: row.get::<_, i64>(7)? as u64,
                artifact_path: row.get(8)?,
                matched: row.get(9)?,
                present: row.get(10)?,
                artifact_id: row.get(11)?,
                job_id: row.get(12)?,
                version: row.get(13)?,
                expected_size: row.get::<_, Option<i64>>(14)?.map(|size| size as u64),
                gog_checksum: row.get(15)?,
                verified_at: row.get(16)?,
                revision_id: row.get(17)?,
                part_id: row.get(18)?,
                provider_file_id: row.get(19)?,
            })
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn mark_managed_file_absent(&self, path: &std::path::Path) -> Result<()> {
        self.connection.execute(
            "UPDATE managed_files SET present = 0, updated_at = unixepoch() WHERE path = ?1",
            params![path.display().to_string()],
        )?;
        Ok(())
    }

    pub fn record_completed_artifacts(
        &self,
        _job_id: &str,
        product_slug: &str,
        artifacts: &[RemoteArtifact],
        files: &[PathBuf],
    ) -> Result<()> {
        for (artifact, path) in artifacts.iter().zip(files) {
            let identity = match (
                &artifact.provider_group_id,
                &artifact.provider_file_id,
                artifact.provider_category,
            ) {
                (Some(group_id), Some(file_id), Some(category)) => self
                    .connection
                    .query_row(
                        "SELECT revision_id, part_id FROM download_parts
                     JOIN download_revisions USING(revision_id)
                     JOIN download_slots USING(slot_id)
                     WHERE product_id = ?1 AND provider_category = ?2 AND provider_group_id = ?3
                       AND provider_file_id = ?4 AND currently_offered = 1
                     ORDER BY download_revisions.last_seen_at DESC LIMIT 1",
                        params![artifact.product_id, category.as_str(), group_id, file_id],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                    )
                    .optional()?,
                _ => None,
            };
            let filename = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&artifact.name);
            let size = path
                .metadata()
                .map_or(artifact.size_bytes.unwrap_or(0), |meta| meta.len());
            self.connection.execute(
                "INSERT INTO managed_files(path, product_id, product_slug, artifact_kind,
                    operating_system, language, filename, size, artifact_path, matched, present,
                    updated_at, artifact_id, job_id, version, expected_size, created_at,
                    revision_id, part_id, provider_file_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, 1, unixepoch(),
                    ?10, ?11, ?12, ?13, unixepoch(), ?14, ?15, ?16)
                 ON CONFLICT(path) DO UPDATE SET product_id=excluded.product_id,
                    product_slug=excluded.product_slug, artifact_kind=excluded.artifact_kind,
                    operating_system=excluded.operating_system, language=excluded.language,
                    filename=excluded.filename, size=excluded.size,
                    artifact_path=excluded.artifact_path, matched=1, present=1,
                    updated_at=excluded.updated_at, artifact_id=excluded.artifact_id,
                    job_id=excluded.job_id, version=excluded.version,
                    expected_size=excluded.expected_size, revision_id=excluded.revision_id,
                    part_id=excluded.part_id, provider_file_id=excluded.provider_file_id",
                params![
                    path.display().to_string(),
                    artifact.product_id,
                    product_slug,
                    artifact.kind.as_str(),
                    artifact.operating_system,
                    artifact.language,
                    filename,
                    size as i64,
                    artifact.download_path,
                    format!(
                        "{}:{}:{}:{}",
                        artifact.product_id,
                        artifact.kind.as_str(),
                        artifact.download_path,
                        artifact.version.as_deref().unwrap_or_default()
                    ),
                    _job_id,
                    artifact.version,
                    artifact.size_bytes.map(|size| size as i64),
                    identity.map(|value| value.0),
                    identity.map(|value| value.1),
                    artifact.provider_file_id,
                ],
            )?;
        }
        Ok(())
    }
}

fn data_path() -> PathBuf {
    crate::identity::database()
}

fn catalog_artifact_id(artifact: &RemoteArtifact) -> String {
    format!(
        "{}:{}:{}:{}",
        artifact.product_id,
        artifact.kind.as_str(),
        artifact.download_path,
        artifact.version.as_deref().unwrap_or_default()
    )
}

fn download_endpoint_name(path: &str) -> Option<&str> {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
}

fn language_code(name: &str) -> String {
    match name.trim().to_ascii_lowercase().as_str() {
        "english" => "en".into(),
        "deutsch" | "german" => "de".into(),
        "français" | "french" => "fr".into(),
        "español" | "spanish" => "es".into(),
        "italiano" | "italian" => "it".into(),
        "polski" | "polish" => "pl".into(),
        "русский" | "russian" => "ru".into(),
        value => value.to_owned(),
    }
}

#[allow(dead_code)]
fn parse_download_category(value: &str) -> DownloadCategory {
    match value {
        "patch" => DownloadCategory::Patch,
        "language_pack" => DownloadCategory::LanguagePack,
        "bonus" => DownloadCategory::Bonus,
        _ => DownloadCategory::Installer,
    }
}

fn revisions_to_artifacts(revisions: &[DownloadRevision]) -> Vec<RemoteArtifact> {
    revisions
        .iter()
        .flat_map(|revision| {
            let count = u32::try_from(revision.parts.len()).ok();
            revision.parts.iter().map(move |part| RemoteArtifact {
                product_id: revision.product_id,
                kind: match revision.provider_category {
                    DownloadCategory::Installer => ArtifactKind::Installer,
                    DownloadCategory::LanguagePack => ArtifactKind::Extra,
                    DownloadCategory::Patch => ArtifactKind::Patch,
                    DownloadCategory::Bonus => ArtifactKind::Extra,
                },
                name: revision.name.clone(),
                language: revision
                    .language_name
                    .clone()
                    .or_else(|| revision.language_code.clone()),
                operating_system: revision.operating_system.clone(),
                version: revision.version.clone(),
                release_date: None,
                size_label: part.expected_size.map(crate::domain::human_size),
                size_bytes: part.expected_size,
                part_number: Some(part.part_index),
                part_count: count,
                download_path: part.downlink.clone(),
                provider_group_id: Some(revision.provider_group_id.clone()),
                provider_file_id: Some(part.provider_file_id.clone()),
                provider_category: Some(revision.provider_category),
            })
        })
        .collect()
}

fn platforms_from_artifacts(artifacts: &[RemoteArtifact]) -> crate::domain::Platforms {
    let mut platforms = crate::domain::Platforms::default();
    for os in artifacts
        .iter()
        .filter_map(|artifact| artifact.operating_system.as_deref())
    {
        match os.to_ascii_lowercase().as_str() {
            "windows" | "win" => platforms.windows = true,
            "linux" => platforms.linux = true,
            "osx" | "mac" | "macos" => platforms.macos = true,
            _ => {}
        }
    }
    platforms
}

fn media_path(media: &serde_json::Value, field: &str) -> Option<PathBuf> {
    media
        .get(field)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .flatten()
}

fn upsert_product_row(
    transaction: &rusqlite::Transaction<'_>,
    game: &Game,
    parent_product_id: Option<i64>,
    product_type: &str,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO products(
            product_id, parent_product_id, product_type, slug, title, release_date,
            gog_release_date, description, changelog, metadata_json, links_json, media_json,
            currently_owned, first_seen_at, last_seen_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, ?9, ?10, ?11, 1,
            unixepoch(), unixepoch(), unixepoch())
         ON CONFLICT(product_id) DO UPDATE SET
            parent_product_id = excluded.parent_product_id, product_type = excluded.product_type,
            slug = excluded.slug, title = excluded.title, release_date = excluded.release_date,
            description = excluded.description, changelog = excluded.changelog,
            metadata_json = excluded.metadata_json, links_json = excluded.links_json,
            media_json = excluded.media_json, currently_owned = 1,
            last_seen_at = unixepoch(), updated_at = unixepoch()",
        params![
            game.product_id,
            parent_product_id,
            product_type,
            game.slug,
            game.title,
            game.release_date.as_ref().map(chrono::DateTime::timestamp),
            game.description,
            game.changelog,
            serde_json::to_string(&game.metadata)?,
            serde_json::to_string(&game.links)?,
            serde_json::to_string(&serde_json::json!({
                "artwork": game.artwork, "detail_artwork": game.detail_artwork,
                "hero_logo": game.hero_logo,
                "icon": game.icon, "screenshots": game.screenshots,
            }))?,
        ],
    )?;
    transaction.execute(
        "INSERT INTO product_activity(
            product_id, last_played_at, playtime_seconds, updated_at, last_activity_at)
         VALUES (?1, NULL, 0, unixepoch(), unixepoch())
         ON CONFLICT(product_id) DO NOTHING",
        params![game.product_id],
    )?;
    Ok(())
}

fn upsert_dlc_row(
    transaction: &rusqlite::Transaction<'_>,
    dlc: &crate::domain::Dlc,
    parent_product_id: i64,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO products(
            product_id, parent_product_id, product_type, slug, title, release_date,
            gog_release_date, description, changelog, metadata_json, links_json, media_json,
            currently_owned, first_seen_at, last_seen_at, updated_at
         ) VALUES (?1, ?2, 'dlc', ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9, ?10, ?11,
            unixepoch(), unixepoch(), unixepoch())
         ON CONFLICT(product_id) DO UPDATE SET
            parent_product_id = excluded.parent_product_id, slug = excluded.slug,
            title = excluded.title, release_date = excluded.release_date,
            description = excluded.description, changelog = excluded.changelog,
            metadata_json = excluded.metadata_json, links_json = excluded.links_json,
            media_json = excluded.media_json, currently_owned = excluded.currently_owned,
            last_seen_at = unixepoch(), updated_at = unixepoch()",
        params![
            dlc.product_id,
            parent_product_id,
            dlc.slug,
            dlc.title,
            dlc.release_date.as_ref().map(chrono::DateTime::timestamp),
            dlc.description,
            dlc.changelog,
            serde_json::to_string(&dlc.metadata)?,
            serde_json::to_string(&dlc.links)?,
            serde_json::to_string(&serde_json::json!({
                "artwork": dlc.artwork, "detail_artwork": dlc.detail_artwork,
                "hero_logo": dlc.hero_logo,
                "icon": dlc.icon, "screenshots": dlc.screenshots,
            }))?,
            dlc.owned,
        ],
    )?;
    Ok(())
}

fn insert_edition_relationships(
    transaction: &rusqlite::Transaction<'_>,
    product_id: i64,
    editions: &[crate::domain::ProductReference],
) -> Result<()> {
    for edition in editions
        .iter()
        .filter(|edition| edition.product_id != product_id)
    {
        transaction.execute(
            "INSERT INTO product_relationships(
                parent_product_id, child_product_id, relationship, source
             ) VALUES (?1, ?2, 'edition', 'store_api')
             ON CONFLICT(parent_product_id, child_product_id, relationship) DO UPDATE SET
                source = excluded.source",
            params![product_id, edition.product_id],
        )?;
    }
    Ok(())
}

fn sum_sizes(parts: &[&RemoteArtifact]) -> Option<u64> {
    parts.iter().try_fold(0_u64, |total, part| {
        part.size_bytes.map(|size| total + size)
    })
}

fn manifest_fingerprint(
    product_id: i64,
    category: DownloadCategory,
    group_id: &str,
    parts: &[&RemoteArtifact],
) -> String {
    let canonical = serde_json::json!({
        "product_id": product_id,
        "category": category.as_str(),
        "group_id": group_id,
        "version": parts.first().and_then(|part| part.version.as_deref()),
        "parts": parts.iter().map(|part| serde_json::json!({
            "file_id": part.provider_file_id,
            "size": part.size_bytes,
            "downlink": part.download_path,
        })).collect::<Vec<_>>(),
    });
    format!("{:x}", Sha256::digest(canonical.to_string().as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::InstallationState;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn current_schema_reopens() {
        let path = std::env::temp_dir().join(format!(
            "gog-state-current-schema-{}.sqlite3",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        drop(StateStore::open_at(&path).unwrap());
        let store = StateStore::open_at(&path).unwrap();
        let version: i64 = store
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
        drop(store);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn obsolete_schema_is_rejected_without_changes() {
        let path = std::env::temp_dir().join(format!(
            "gog-state-obsolete-schema-{}.sqlite3",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE sentinel(value TEXT NOT NULL);
                 INSERT INTO sentinel VALUES ('unchanged');
                 PRAGMA user_version = 23;",
            )
            .unwrap();
        drop(connection);

        let error = StateStore::open_at(&path).err().unwrap().to_string();
        assert!(error.contains("schema version 23 is unsupported"));
        let connection = Connection::open(&path).unwrap();
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        let value: String = connection
            .query_row("SELECT value FROM sentinel", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 23);
        assert_eq!(value, "unchanged");
        drop(connection);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn installation_operations_round_trip() {
        let path = std::env::temp_dir().join(format!(
            "gog-state-install-operation-{}.sqlite3",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let store = StateStore::open_at(&path).unwrap();
        let record = InstallationOperationRecord {
            product_id: 1449651388,
            operation: "install".into(),
            state: "running".into(),
            plan_json: r#"{"install_base":true}"#.into(),
            message: Some("Installing DLC".into()),
            percentage: Some(42),
            queue_position: Some(3),
            created_at: 10,
            updated_at: 20,
            completed_at: None,
        };
        store.upsert_installation_operation(&record).unwrap();
        assert_eq!(store.installation_operations().unwrap(), vec![record]);
        let version: i64 = store
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 24);
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn game_preferences_round_trip_without_installation_state() {
        let path = std::env::temp_dir().join(format!(
            "gog-state-installed-game-{}.sqlite3",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = StateStore::open_at(&path).unwrap();
        let preferences = GamePreferences {
            product_id: 1449651388,
            executable_path: Some(PathBuf::from("Grim Dawn.exe")),
            launch_arguments: vec!["/x64".to_owned(), "/d3d11".to_owned()],
            compatibility: Some(crate::compatibility::GameCompatibilityPreferences {
                backend: crate::compatibility::CompatibilityBackendKind::Umu,
                prefix_slug: "grim-dawn".into(),
                profile: crate::compatibility::UmuProfile::fallback(),
                pending_profile: None,
            }),
            created_at: 50,
            updated_at: 200,
        };
        store.upsert_game_preferences(&preferences).unwrap();
        assert_eq!(
            store.game_preferences(1449651388).unwrap(),
            Some(preferences)
        );
        drop(store);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn play_history_accumulates_without_an_installation_record() {
        let path = std::env::temp_dir().join(format!(
            "gog-state-product-activity-{}.sqlite3",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = StateStore::open_at(&path).unwrap();
        store.record_game_session(42, 100, 60).unwrap();
        store.record_game_session(42, 200, 90).unwrap();
        store.record_product_activity(42, 300).unwrap();
        assert_eq!(store.product_activity(42).unwrap(), (Some(200), 150));
        assert_eq!(
            store.all_product_activity().unwrap().get(&42),
            Some(&ProductActivity {
                last_played_at: Some(200),
                last_activity_at: Some(300),
                playtime_seconds: 150
            })
        );
        drop(store);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn enrichment_observations_persist_positive_and_negative_results() {
        let path = std::env::temp_dir().join(format!(
            "gog-state-enrichment-observation-{}.sqlite3",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = StateStore::open_at(&path).unwrap();
        store
            .record_enrichment_observation(101, "gamesdb", "not_found")
            .unwrap();
        assert_eq!(
            store
                .enrichment_observation(101, "gamesdb")
                .unwrap()
                .map(|value| value.0),
            Some("not_found".to_owned())
        );
        store
            .record_enrichment_observation(101, "gamesdb", "available")
            .unwrap();
        assert_eq!(
            store
                .enrichment_observation(101, "gamesdb")
                .unwrap()
                .map(|value| value.0),
            Some("available".to_owned())
        );
        drop(store);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn manifest_refresh_retires_but_preserves_old_versions() {
        let path = std::env::temp_dir().join(format!(
            "gog-state-artifact-history-{}.sqlite3",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = StateStore::open_at(&path).unwrap();
        let artifact = |version: &str| RemoteArtifact {
            product_id: 42,
            kind: ArtifactKind::Installer,
            name: "Example Game".into(),
            language: Some("English".into()),
            operating_system: Some("linux".into()),
            version: Some(version.into()),
            release_date: None,
            size_label: Some("10 MB".into()),
            size_bytes: Some(10_000_000),
            part_number: None,
            part_count: None,
            download_path: "/downloads/example/en1installer0".into(),
            provider_group_id: None,
            provider_file_id: None,
            provider_category: None,
        };
        store
            .cache_download_manifest(42, &[artifact("1.0")])
            .unwrap();
        store
            .cache_download_manifest(42, &[artifact("2.0")])
            .unwrap();
        let catalog = store.artifact_catalog(42).unwrap();
        assert_eq!(catalog.len(), 2);
        assert!(catalog.iter().any(|entry| {
            entry.artifact.version.as_deref() == Some("1.0") && !entry.currently_offered
        }));
        assert!(catalog.iter().any(|entry| {
            entry.artifact.version.as_deref() == Some("2.0") && entry.currently_offered
        }));
        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn managed_files_distinguish_versions_that_share_a_gog_path() {
        let root = std::env::temp_dir().join(format!(
            "gog-state-managed-version-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let database = root.join("state.sqlite3");
        let store = StateStore::open_at(&database).unwrap();
        let artifact = |version: &str| RemoteArtifact {
            product_id: 42,
            kind: ArtifactKind::Installer,
            name: "Example Game".into(),
            language: Some("English".into()),
            operating_system: Some("linux".into()),
            version: Some(version.into()),
            release_date: None,
            size_label: None,
            size_bytes: Some(4),
            part_number: None,
            part_count: None,
            download_path: "/downloads/example/en1installer0".into(),
            provider_group_id: None,
            provider_file_id: None,
            provider_category: None,
        };
        let first = root.join("example-1.0.sh");
        let second = root.join("example-2.0.sh");
        std::fs::write(&first, b"old!").unwrap();
        std::fs::write(&second, b"new!").unwrap();
        let mut old_download = artifact("1.0");
        old_download.download_path =
            "https://api.gog.com/products/42/downlink/installer/en1installer0".into();
        store
            .record_completed_artifacts(
                "old-job",
                "example",
                &[old_download],
                std::slice::from_ref(&first),
            )
            .unwrap();
        store
            .record_completed_artifacts("new-job", "example", &[artifact("2.0")], &[second])
            .unwrap();
        let files = store.managed_files().unwrap();
        assert_eq!(files.len(), 2);
        assert_ne!(files[0].artifact_id, files[1].artifact_id);
        assert!(
            files
                .iter()
                .any(|file| file.version.as_deref() == Some("1.0"))
        );
        assert!(
            files
                .iter()
                .any(|file| file.version.as_deref() == Some("2.0"))
        );
        store
            .cache_download_manifest(42, &[artifact("1.0")])
            .unwrap();
        store
            .cache_download_manifest(42, &[artifact("2.0")])
            .unwrap();
        assert_eq!(
            store
                .retired_artifact_for_file(&first)
                .unwrap()
                .and_then(|artifact| artifact.version),
            Some("1.0".into())
        );
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn managed_reconciliation_transfers_an_artifact_identity_to_its_current_path() {
        let root = std::env::temp_dir().join(format!(
            "gog-state-managed-move-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut store = StateStore::open_at(&root.join("state.sqlite3")).unwrap();
        let old = root.join("old/setup.exe");
        let current = root.join("current/setup.exe");
        let record = |path: PathBuf| ManagedFileRecord {
            path,
            product_id: 42,
            product_slug: "example".into(),
            kind: ArtifactKind::Installer,
            operating_system: Some("windows".into()),
            language: Some("English".into()),
            filename: "setup.exe".into(),
            size: 4,
            artifact_path: Some("/downlink/installer/en1installer0".into()),
            matched: true,
            present: true,
            artifact_id: Some("42:installer:en1installer0:1.0".into()),
            job_id: None,
            version: Some("1.0".into()),
            expected_size: Some(4),
            gog_checksum: None,
            verified_at: None,
            revision_id: None,
            part_id: None,
            provider_file_id: Some("en1installer0".into()),
        };

        store.replace_managed_files(&[record(old)]).unwrap();
        store
            .replace_managed_files(&[record(current.clone())])
            .unwrap();

        let files = store.managed_files().unwrap();
        assert_eq!(files.iter().filter(|file| file.present).count(), 1);
        assert_eq!(
            files
                .iter()
                .find(|file| file.present)
                .map(|file| file.path.as_path()),
            Some(current.as_path())
        );
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn structured_manifest_reuses_slots_and_preserves_revisions() {
        let path = std::env::temp_dir().join(format!(
            "gog-state-revisions-{}.sqlite3",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = StateStore::open_at(&path).unwrap();
        let artifact = |file_id: &str, size| RemoteArtifact {
            product_id: 42,
            kind: ArtifactKind::Installer,
            name: "Example".into(),
            language: Some("English".into()),
            operating_system: Some("windows".into()),
            version: Some("1.0".into()),
            release_date: None,
            size_label: None,
            size_bytes: Some(size),
            part_number: Some(if file_id.ends_with('0') { 1 } else { 2 }),
            part_count: Some(2),
            download_path: format!("/downlink/installer/{file_id}"),
            provider_group_id: Some("installer_windows_en".into()),
            provider_file_id: Some(file_id.into()),
            provider_category: Some(DownloadCategory::Installer),
        };
        let first = vec![artifact("en1installer0", 10), artifact("en1installer1", 20)];
        store.observe_download_manifest(42, &first).unwrap();
        store.observe_download_manifest(42, &first).unwrap();
        assert_eq!(store.load_current_download_revisions(42).unwrap().len(), 1);
        let changed = vec![artifact("en1installer0", 10), artifact("en1installer1", 21)];
        store.observe_download_manifest(42, &changed).unwrap();
        let revision_count: i64 = store
            .connection
            .query_row("SELECT COUNT(*) FROM download_revisions", [], |row| {
                row.get(0)
            })
            .unwrap();
        let current_count: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM download_revisions WHERE currently_offered = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(revision_count, 2);
        assert_eq!(current_count, 1);
        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn installed_revision_detects_a_newer_observation_in_the_same_slot() {
        let path = std::env::temp_dir().join(format!(
            "gog-state-install-update-{}.sqlite3",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = StateStore::open_at(&path).unwrap();
        let artifact = |file_id: &str, version: &str| RemoteArtifact {
            product_id: 42,
            kind: ArtifactKind::Installer,
            name: "Example".into(),
            language: Some("English".into()),
            operating_system: Some("windows".into()),
            version: Some(version.into()),
            release_date: None,
            size_label: None,
            size_bytes: Some(10),
            part_number: Some(1),
            part_count: Some(1),
            download_path: format!("/downlink/installer/{file_id}"),
            provider_group_id: Some("installer_windows_en".into()),
            provider_file_id: Some(file_id.into()),
            provider_category: Some(DownloadCategory::Installer),
        };
        let old_artifact = artifact("old", "1.0");
        store
            .observe_download_manifest(42, std::slice::from_ref(&old_artifact))
            .unwrap();
        let old_revision = store.load_current_download_revisions(42).unwrap()[0].revision_id;
        store
            .record_completed_artifacts(
                "old-job",
                "example",
                std::slice::from_ref(&old_artifact),
                &[PathBuf::from("/games/example/setup.exe")],
            )
            .unwrap();
        let installed = InstalledGame {
            product_id: 42,
            library_id: "default".into(),
            installed_version: Some("1.0".into()),
            installation_directory: PathBuf::from("/games/example"),
            installer_revision_id: Some(old_revision),
            installer_job_id: None,
            installer_files: vec![PathBuf::from("/games/example/setup.exe")],
            installer_complete: true,
            installer_operating_system: Some("windows".into()),
            installer_language: Some("English".into()),
            compatibility: None,
            primary_executable: None,
            launch_arguments: Vec::new(),
            state: InstallationState::Installed,
            error: None,
            installed_at: Some(1),
            verified_at: None,
            last_played_at: None,
            playtime_seconds: 0,
            created_at: 1,
            updated_at: 1,
        };
        assert!(!store.installation_update_available(&installed).unwrap());
        let mut marker_only_install = installed.clone();
        marker_only_install.installer_revision_id = None;
        assert!(
            !store
                .installation_update_available(&marker_only_install)
                .unwrap()
        );
        assert!(!store.installer_backup_update_available(42).unwrap());

        store
            .observe_download_manifest(42, &[artifact("new", "2.0")])
            .unwrap();
        assert!(store.installation_update_available(&installed).unwrap());
        assert!(
            store
                .installation_update_available(&marker_only_install)
                .unwrap()
        );
        assert!(store.installer_backup_update_available(42).unwrap());
        store
            .connection
            .execute(
                "UPDATE managed_files
                 SET revision_id = NULL, part_id = NULL, provider_file_id = NULL
                 WHERE product_id = 42",
                [],
            )
            .unwrap();
        assert!(store.installer_backup_update_available(42).unwrap());
        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn synchronization_stage_outcomes_replace_stale_results() {
        let path = std::env::temp_dir().join(format!(
            "gog-state-sync-outcomes-{}.sqlite3",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = StateStore::open_at(&path).unwrap();
        store.mark_sync_stage_started("artwork").unwrap();
        store
            .mark_sync_stage_finished("artwork", false, Some("request failed"))
            .unwrap();
        store.mark_sync_stage_started("artwork").unwrap();
        store
            .mark_sync_stage_finished("artwork", true, None)
            .unwrap();
        let (status, completed_at, error): (String, Option<i64>, Option<String>) = store
            .connection
            .query_row(
                "SELECT status, completed_at, error FROM sync_stage_outcomes WHERE stage = 'artwork'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(status, "success");
        assert!(completed_at.is_some());
        assert!(error.is_none());
        drop(store);
        std::fs::remove_file(path).unwrap();
    }
}

use super::{UmuProfile, UmuProfileSource};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, SystemTime},
};

const URL: &str = "https://umu.openwinecomponents.org/umu_api.php";
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UmuDatabaseEntry {
    pub title: String,
    pub store: String,
    pub codename: String,
    pub umu_id: String,
    #[serde(default, alias = "exe_string")]
    pub executable_pattern: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}
fn cache_path() -> PathBuf {
    crate::identity::cache_root().join("umu-database.json")
}
fn valid(entries: &[UmuDatabaseEntry]) -> bool {
    !entries.is_empty()
        && entries.iter().all(|e| {
            !e.title.is_empty()
                && !e.store.is_empty()
                && !e.codename.is_empty()
                && e.umu_id.starts_with("umu-")
                && !e.umu_id.contains('\0')
        })
}
fn read_cache() -> Option<Vec<UmuDatabaseEntry>> {
    let values: Vec<UmuDatabaseEntry> =
        serde_json::from_slice(&fs::read(cache_path()).ok()?).ok()?;
    valid(&values).then_some(values)
}
pub fn resolve_profile(product_id: i64) -> (UmuProfile, Option<UmuDatabaseEntry>) {
    let cached = read_cache();
    let fresh = cache_path()
        .metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < Duration::from_secs(86400));
    let entries = if fresh {
        cached.clone()
    } else {
        download().ok().or(cached)
    };
    let found = entries.and_then(|v| {
        let mut m = v
            .into_iter()
            .filter(|e| e.store == "gog" && e.codename == product_id.to_string());
        let first = m.next()?;
        m.next().is_none().then_some(first)
    });
    found.map_or((UmuProfile::fallback(), None), |e| {
        (
            UmuProfile {
                game_id: e.umu_id.clone(),
                store: "gog".into(),
                source: UmuProfileSource::GogProductId,
            },
            Some(e),
        )
    })
}

/// Recheck unresolved profiles whenever an operation uses their prefix. Known
/// mappings remain stable; only the explicit fallback consults the database.
pub fn profile_for_use(product_id: i64, saved: &UmuProfile) -> UmuProfile {
    if saved.source != UmuProfileSource::DefaultFallback {
        return saved.clone();
    }
    resolve_profile(product_id).0
}
fn download() -> Result<Vec<UmuDatabaseEntry>> {
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(crate::identity::USER_AGENT)
        .build()?
        .get(URL)
        .send()?
        .error_for_status()?;
    if response
        .content_length()
        .is_some_and(|n| n > 8 * 1024 * 1024)
    {
        bail!("UMU database too large")
    }
    let bytes = response.bytes()?;
    if bytes.len() > 8 * 1024 * 1024 {
        bail!("UMU database too large")
    }
    let entries: Vec<UmuDatabaseEntry> =
        serde_json::from_slice(&bytes).context("invalid UMU database")?;
    if !valid(&entries) {
        bail!("invalid UMU database")
    }
    let path = cache_path();
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(path, serde_json::to_vec(&entries)?)?;
    Ok(entries)
}

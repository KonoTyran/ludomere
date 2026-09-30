use crate::domain::{
    ArtifactKind, Dlc, ExternalLinks, GalaxyBuild, Game, Platforms, ProductMetadata,
    RemoteArtifact, Screenshot,
};
use anyhow::{Context, Result};
use chrono::DateTime;
use gdk_pixbuf::Pixbuf;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, sync::mpsc, time::Duration};

pub enum SyncEvent {
    Ownership(usize),
    BasicBatch {
        games: Vec<Game>,
        current: usize,
        total: usize,
    },
    Catalog {
        games: Vec<Game>,
    },
    CoversQueued {
        product_ids: Vec<i64>,
    },
    CoverStarted {
        product_id: i64,
    },
    CoverFinished {
        product_id: i64,
        outcome: CoverOutcome,
        current: usize,
        total: usize,
    },
    IconsQueued {
        product_ids: Vec<i64>,
    },
    IconStarted {
        product_id: i64,
    },
    IconFinished {
        product_id: i64,
        outcome: CoverOutcome,
        current: usize,
        total: usize,
    },
    ImageRetryComplete,
    Media {
        product_id: i64,
        artwork: Option<PathBuf>,
        detail_artwork: Option<PathBuf>,
        hero_logo: Option<PathBuf>,
        icon: Option<PathBuf>,
        current: usize,
        total: usize,
    },
    FileMetadata {
        product_id: i64,
        artifacts: Vec<RemoteArtifact>,
        current: usize,
        total: usize,
    },
    Enrichment {
        product_id: i64,
        metadata: Box<ProductMetadata>,
        current: usize,
        total: usize,
    },
    Builds {
        product_id: i64,
        builds: Vec<GalaxyBuild>,
        windows_observed: bool,
        macos_observed: bool,
        current: usize,
        total: usize,
    },
    Complete {
        games: Vec<Game>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoverOutcome {
    Loaded(PathBuf),
    Unavailable,
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ImageKind {
    Cover,
    Icon,
}

enum ImageWork {
    Started(i64, ImageKind),
    Finished(i64, ImageKind, Result<Option<PathBuf>>),
}

#[derive(Clone)]
struct ImageRequest {
    product_id: i64,
    kind: ImageKind,
    url: Option<String>,
    fallback: Option<String>,
    cached: Option<PathBuf>,
}

struct AssetRequest {
    artwork_url: Option<String>,
    artwork_fallback_url: Option<String>,
    detail_artwork_urls: Vec<String>,
    icon_url: Option<String>,
}

const GAMESDB_AVAILABLE_REFRESH_SECONDS: i64 = 7 * 24 * 60 * 60;
const GAMESDB_NOT_FOUND_REFRESH_SECONDS: i64 = 30 * 24 * 60 * 60;

fn gamesdb_refresh_due(
    observation: Option<&(String, i64)>,
    has_cached_metadata: bool,
    now: i64,
    force: bool,
) -> bool {
    force
        || match observation {
            Some((status, checked_at)) if status == "available" => {
                !has_cached_metadata
                    || now.saturating_sub(*checked_at) >= GAMESDB_AVAILABLE_REFRESH_SECONDS
            }
            Some((status, checked_at)) if status == "not_found" => {
                now.saturating_sub(*checked_at) >= GAMESDB_NOT_FOUND_REFRESH_SECONDS
            }
            _ => true,
        }
}

type DownloadManifest = (
    Vec<RemoteArtifact>,
    String,
    Vec<(i64, Vec<RemoteArtifact>, String)>,
);

#[derive(Deserialize, Serialize)]
struct AssetManifest {
    source_url: String,
    etag: Option<String>,
    last_modified: Option<String>,
    #[serde(default = "manifest_asset_usable")]
    usable: bool,
    #[serde(default)]
    wordmark_processed: bool,
    #[serde(default)]
    wordmark_processing_version: u32,
}

const fn manifest_asset_usable() -> bool {
    true
}

const WORDMARK_PROCESSING_VERSION: u32 = 3;

#[derive(Debug, Clone, Deserialize)]
struct Product {
    id: i64,
    slug: String,
    title: String,
    release_date: Option<String>,
    description: Option<Description>,
    changelog: Option<String>,
    content_system_compatibility: Option<Compatibility>,
    languages: Option<serde_json::Value>,
    links: Option<Links>,
    images: Option<Images>,
    screenshots: Option<Vec<ProductScreenshot>>,
    #[serde(default)]
    game_type: String,
    #[serde(default)]
    is_installable: bool,
    dlcs: Option<DlcCollection>,
    expanded_dlcs: Option<Vec<Product>>,
}

#[derive(Debug, Clone, Deserialize)]
struct DlcCollection {
    #[serde(default)]
    products: Vec<DlcReference>,
}

#[derive(Debug, Clone, Deserialize)]
struct DlcReference {
    id: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct Description {
    full: Option<String>,
    lead: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Compatibility {
    windows: Option<bool>,
    linux: Option<bool>,
    osx: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct Links {
    product_card: Option<String>,
    forum: Option<String>,
    support: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Images {
    background: Option<String>,
    logo2x: Option<String>,
    logo: Option<String>,
    sidebar_icon2x: Option<String>,
    sidebar_icon: Option<String>,
    icon: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ProductScreenshot {
    image_id: Option<String>,
    formatted_images: Option<Vec<FormattedImage>>,
}

#[derive(Debug, Clone, Deserialize)]
struct FormattedImage {
    formatter_name: Option<String>,
    image_url: Option<String>,
}

static LIBRARY_SESSION: std::sync::Mutex<u64> = std::sync::Mutex::new(0);
static ACCOUNT_SESSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static ACCOUNT_COMMIT: std::sync::Mutex<()> = std::sync::Mutex::new(());
static COVER_PRIORITY: std::sync::Mutex<Vec<i64>> = std::sync::Mutex::new(Vec::new());
static FAILED_IMAGES: std::sync::Mutex<ImageRetryState> = std::sync::Mutex::new(ImageRetryState {
    session: 0,
    requests: Vec::new(),
    running: false,
});

struct ImageRetryState {
    session: u64,
    requests: Vec<ImageRequest>,
    running: bool,
}
struct ImageBatchGuard(u64);
impl Drop for ImageBatchGuard {
    fn drop(&mut self) {
        let mut state = FAILED_IMAGES
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.session == self.0 {
            state.running = false;
        }
    }
}
static ASSET_LOCKS: [std::sync::Mutex<()>; 32] = [const { std::sync::Mutex::new(()) }; 32];

fn asset_lock(path: &std::path::Path) -> std::sync::MutexGuard<'static, ()> {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hash);
    ASSET_LOCKS[hash.finish() as usize % ASSET_LOCKS.len()]
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

pub fn begin_library_session() -> u64 {
    let mut session = LIBRARY_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    *session = session.wrapping_add(1);
    *FAILED_IMAGES
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = ImageRetryState {
        session: *session,
        requests: Vec::new(),
        running: false,
    };
    *session
}

pub fn invalidate_library_session() {
    let _account = ACCOUNT_COMMIT
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    ACCOUNT_SESSION.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    begin_library_session();
}

pub fn account_session() -> u64 {
    ACCOUNT_SESSION.load(std::sync::atomic::Ordering::Acquire)
}

pub(crate) fn with_account_session<T>(
    session: u64,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let _current = ACCOUNT_COMMIT
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    anyhow::ensure!(
        account_session() == session,
        "The signed-in account changed; choose the download again"
    );
    operation()
}

pub fn prioritize_covers(ids: Vec<i64>) {
    *COVER_PRIORITY
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = ids;
}

struct CoreEntitlements {
    ids: std::collections::HashSet<i64>,
    packs: Vec<(i64, Vec<i64>)>,
}

fn core_entitlements(
    products: &[Product],
    owned: &[i64],
    store: &crate::state::StateStore,
) -> Result<CoreEntitlements> {
    let packs = products
        .iter()
        .filter(|product| product.game_type == "pack")
        .filter_map(|product| {
            product.dlcs.as_ref().map(|dlcs| {
                (
                    product.id,
                    dlcs.products
                        .iter()
                        .map(|child| child.id)
                        .collect::<Vec<_>>(),
                )
            })
        })
        .collect::<Vec<_>>();
    let unobserved = owned
        .iter()
        .copied()
        .filter(|id| {
            !products.iter().any(|product| {
                product.id == *id && (product.game_type != "pack" || product.dlcs.is_some())
            })
        })
        .collect::<Vec<_>>();
    let mut ids = owned
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    ids.extend(store.cached_pack_entitlements(&unobserved)?);
    ids.extend(
        packs
            .iter()
            .flat_map(|(_, children)| children.iter().copied()),
    );
    Ok(CoreEntitlements { ids, packs })
}

pub fn stream_owned_games(
    product_ids: &[i64],
    access_token: &str,
    sender: &mpsc::Sender<Result<SyncEvent>>,
    force_gamesdb_refresh: bool,
    _preferred_installer_language: Option<&str>,
    session: u64,
) -> Result<()> {
    let client = crate::gog::client()?;
    let store = crate::state::StateStore::open()?;
    let cached = store.normalized_games()?;
    let mut products = Vec::new();
    for (index, ids) in product_ids.chunks(50).enumerate() {
        if *LIBRARY_SESSION
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            != session
        {
            return Ok(());
        }
        let batch = crate::gog::product::fetch_core(&client, ids)?
            .into_iter()
            .map(serde_json::from_value::<Product>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let games = batch
            .iter()
            .filter(|product| product.game_type == "game" && product.is_installable)
            .map(|product| {
                merge_core(
                    product,
                    cached.iter().find(|game| game.product_id == product.id),
                )
            })
            .collect();
        sender.send(Ok(SyncEvent::BasicBatch {
            games,
            current: ((index + 1) * 50).min(product_ids.len()),
            total: product_ids.len(),
        }))?;
        products.extend(batch);
    }
    let CoreEntitlements {
        ids: entitled,
        packs: observed_packs,
    } = core_entitlements(&products, product_ids, &store)?;
    let mut inherited = entitled
        .iter()
        .copied()
        .filter(|id| !product_ids.contains(id))
        .collect::<Vec<_>>();
    inherited.sort_unstable();
    inherited.dedup();
    products.extend(
        crate::gog::product::fetch_core(&client, &inherited)?
            .into_iter()
            .map(serde_json::from_value::<Product>)
            .collect::<std::result::Result<Vec<_>, _>>()?,
    );
    let unresolved = inherited
        .into_iter()
        .filter(|id| !products.iter().any(|product| product.id == *id))
        .collect();
    products.extend(fetch_account_dlc_fallbacks(
        &client,
        access_token,
        &products,
        &unresolved,
    )?);
    let mut games = Vec::new();
    for product in products
        .iter()
        .filter(|product| product.game_type == "game" && product.is_installable)
    {
        let mut game = merge_core(
            product,
            cached.iter().find(|game| game.product_id == product.id),
        );
        if let Some(dlcs) = &product.dlcs {
            for reference in &dlcs.products {
                if let Some(child) = products
                    .iter()
                    .find(|child| child.id == reference.id && child.game_type == "dlc")
                {
                    let existing = game.dlcs.iter().position(|dlc| dlc.product_id == child.id);
                    let mut dlc =
                        game_into_dlc(normalize_product(child), entitled.contains(&child.id));
                    if let Some(index) = existing {
                        let mut old = game.dlcs[index].clone();
                        old.title = dlc.title;
                        old.slug = dlc.slug;
                        old.platforms = merge_core(child, Some(&old.clone().into())).platforms;
                        if child.languages.is_some() {
                            old.languages = dlc.languages;
                        }
                        old.owned = dlc.owned;
                        dlc = old;
                        game.dlcs[index] = dlc;
                    } else {
                        game.dlcs.push(dlc);
                    }
                }
            }
        }
        for dlc in &mut game.dlcs {
            dlc.owned = entitled.contains(&dlc.product_id);
        }
        game.dlc_count = game.dlcs.len();
        games.push(game);
    }
    // An omitted product is not an ownership revocation.
    for game in cached {
        if entitled.contains(&game.product_id)
            && !games.iter().any(|new| new.product_id == game.product_id)
        {
            games.push(game);
        }
    }
    games.sort_by_key(|game| game.title.to_lowercase());
    {
        let current = LIBRARY_SESSION
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if *current != session {
            return Ok(());
        }
        if force_gamesdb_refresh {
            for section in [
                DetailSection::Product,
                DetailSection::Metadata,
                DetailSection::Artwork,
                DetailSection::Acquisition,
                DetailSection::Builds,
            ] {
                store.clear_enrichment_source(section.source())?;
            }
            store.clear_enrichment_source("gamesdb")?;
        }
        store.cache_core_library(
            &games,
            product_ids,
            &entitled.into_iter().collect::<Vec<_>>(),
            &observed_packs,
        )?;
    }
    sender.send(Ok(SyncEvent::Catalog {
        games: games.clone(),
    }))?;
    let assets = image_requests(&games, &products);
    {
        let current = LIBRARY_SESSION
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if *current != session {
            return Ok(());
        }
        *FAILED_IMAGES
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = ImageRetryState {
            session,
            requests: assets.clone(),
            running: true,
        };
    }
    let _guard = ImageBatchGuard(session);
    stream_images(assets, &client, &store, sender, session, &mut games)?;
    if *LIBRARY_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        == session
    {
        sender.send(Ok(SyncEvent::Complete { games }))?;
    }
    Ok(())
}

/// Query retained image requests without exposing their URLs or performing any I/O.
pub fn has_failed_images(session: u64) -> bool {
    let current = LIBRARY_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let state = FAILED_IMAGES
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    *current == session && state.session == session && !state.requests.is_empty()
}

/// Reuse failed job URLs from this library session; never fetch ownership or product metadata.
pub fn retry_failed_images(sender: &mpsc::Sender<Result<SyncEvent>>, session: u64) -> Result<()> {
    let assets = {
        let current = LIBRARY_SESSION
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        anyhow::ensure!(
            *current == session,
            "The library session changed; these image failures are no longer current"
        );
        let mut state = FAILED_IMAGES
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        anyhow::ensure!(
            state.session == session && !state.running,
            "Image loading is already running"
        );
        anyhow::ensure!(
            !state.requests.is_empty(),
            "There are no failed image requests to retry"
        );
        state.running = true;
        state.requests.clone()
    };
    let _guard = ImageBatchGuard(session);
    stream_images(
        assets,
        &crate::gog::client()?,
        &crate::state::StateStore::open()?,
        sender,
        session,
        &mut [],
    )?;
    if *LIBRARY_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        == session
    {
        sender.send(Ok(SyncEvent::ImageRetryComplete))?;
    }
    Ok(())
}

fn stream_images(
    assets: Vec<ImageRequest>,
    client: &reqwest::blocking::Client,
    store: &crate::state::StateStore,
    sender: &mpsc::Sender<Result<SyncEvent>>,
    session: u64,
    games: &mut [Game],
) -> Result<()> {
    let cover_ids = assets
        .iter()
        .filter(|asset| asset.kind == ImageKind::Cover)
        .map(|asset| asset.product_id)
        .collect::<Vec<_>>();
    let icon_ids = assets
        .iter()
        .filter(|asset| asset.kind == ImageKind::Icon)
        .map(|asset| asset.product_id)
        .collect::<Vec<_>>();
    let cover_total = cover_ids.len();
    let icon_total = icon_ids.len();
    sender.send(Ok(SyncEvent::CoversQueued {
        product_ids: cover_ids,
    }))?;
    sender.send(Ok(SyncEvent::IconsQueued {
        product_ids: icon_ids,
    }))?;
    let mut completed_covers = 0;
    let mut completed_icons = 0;
    stream_image_work(
        assets,
        || {
            *LIBRARY_SESSION
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                == session
        },
        |asset| {
            if asset.url.is_none()
                && let Some(path) = &asset.cached
                && validate_cover_file(path).is_ok()
            {
                return Ok(Some(path.clone()));
            }
            cache_asset_with_fallback(
                client,
                asset.url.as_deref(),
                asset.fallback.as_deref(),
                std::path::Path::new(if asset.kind == ImageKind::Cover {
                    "tile.jpg"
                } else {
                    "icon.png"
                }),
            )
        },
        |work| {
            let current = LIBRARY_SESSION
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if *current != session {
                return Ok(());
            }
            match work {
                ImageWork::Started(product_id, kind) => sender.send(Ok(match kind {
                    ImageKind::Cover => SyncEvent::CoverStarted { product_id },
                    ImageKind::Icon => SyncEvent::IconStarted { product_id },
                }))?,
                ImageWork::Finished(product_id, kind, artwork) => {
                    let outcome = finish_image(artwork, |path| match kind {
                        ImageKind::Cover => store.cache_product_cover(product_id, path),
                        ImageKind::Icon => store.cache_product_icon(product_id, path),
                    });
                    if !matches!(outcome, CoverOutcome::Failed(_)) {
                        let mut state = FAILED_IMAGES
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        if state.session == session {
                            state.requests.retain(|request| {
                                request.product_id != product_id || request.kind != kind
                            });
                        }
                    }
                    if let CoverOutcome::Loaded(path) = &outcome {
                        match kind {
                            ImageKind::Cover => {
                                apply_media(games, product_id, Some(path.clone()), None, None, None)
                            }
                            ImageKind::Icon => {
                                apply_media(games, product_id, None, None, None, Some(path.clone()))
                            }
                        }
                    }
                    let event = match kind {
                        ImageKind::Cover => {
                            completed_covers += 1;
                            SyncEvent::CoverFinished {
                                product_id,
                                outcome,
                                current: completed_covers,
                                total: cover_total,
                            }
                        }
                        ImageKind::Icon => {
                            completed_icons += 1;
                            SyncEvent::IconFinished {
                                product_id,
                                outcome,
                                current: completed_icons,
                                total: icon_total,
                            }
                        }
                    };
                    sender.send(Ok(event))?;
                }
            }
            Ok(())
        },
    )?;
    Ok(())
}

fn image_requests(games: &[Game], products: &[Product]) -> Vec<ImageRequest> {
    let mut jobs = Vec::with_capacity(games.len() * 2);
    for game in games {
        let asset = products
            .iter()
            .find(|product| product.id == game.product_id)
            .map(asset_request);
        jobs.push(ImageRequest {
            product_id: game.product_id,
            kind: ImageKind::Cover,
            url: asset.as_ref().and_then(|asset| asset.artwork_url.clone()),
            fallback: asset
                .as_ref()
                .and_then(|asset| asset.artwork_fallback_url.clone()),
            cached: game.artwork.clone(),
        });
        jobs.push(ImageRequest {
            product_id: game.product_id,
            kind: ImageKind::Icon,
            url: asset.and_then(|asset| asset.icon_url),
            fallback: None,
            cached: game.icon.clone(),
        });
    }
    jobs
}

/// Covers and sidebar icons share bounded workers, with independent jobs and outcomes.
fn stream_image_work(
    assets: Vec<ImageRequest>,
    is_current: impl Fn() -> bool + Sync,
    fetch: impl Fn(&ImageRequest) -> Result<Option<PathBuf>> + Sync,
    mut receive: impl FnMut(ImageWork) -> Result<()>,
) -> Result<()> {
    let jobs = std::sync::Mutex::new(assets);
    let (media_sender, media_receiver) = mpsc::sync_channel(8);
    let stopped = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let media_sender = media_sender.clone();
            let jobs = &jobs;
            let fetch = &fetch;
            let is_current = &is_current;
            let stopped = &stopped;
            scope.spawn(move || {
                loop {
                    if !is_current() || stopped.load(std::sync::atomic::Ordering::Acquire) {
                        break;
                    }
                    let asset = {
                        let mut jobs = jobs.lock().unwrap_or_else(|error| error.into_inner());
                        let priorities = COVER_PRIORITY
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        let index = priorities
                            .iter()
                            .find_map(|id| jobs.iter().position(|asset| asset.product_id == *id));
                        if jobs.is_empty() {
                            None
                        } else {
                            Some(jobs.remove(index.unwrap_or(0)))
                        }
                    };
                    let Some(asset) = asset else {
                        break;
                    };
                    if media_sender
                        .send(ImageWork::Started(asset.product_id, asset.kind))
                        .is_err()
                    {
                        break;
                    }
                    let artwork = fetch(&asset);
                    if !is_current()
                        || media_sender
                            .send(ImageWork::Finished(asset.product_id, asset.kind, artwork))
                            .is_err()
                    {
                        break;
                    }
                }
            });
        }
        drop(media_sender);
        // Moving the receiver into this iterator drops it on every early return, unblocking senders.
        for work in media_receiver {
            if !is_current() {
                return Ok(());
            }
            if let Err(error) = receive(work) {
                stopped.store(true, std::sync::atomic::Ordering::Release);
                return Err(error);
            }
        }
        Ok::<_, anyhow::Error>(())
    })
}

/// UI/log-safe classification: never forwards URLs, tokens, response bodies or local paths.
pub fn sync_error_message(error: &anyhow::Error) -> String {
    if let Some(error) = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<reqwest::Error>())
    {
        if let Some(status) = error.status() {
            return format!(
                "GOG request failed (HTTP {}). Retry synchronization.",
                status.as_u16()
            );
        }
        if error.is_timeout() {
            return "GOG request timed out. Check your connection and retry synchronization."
                .into();
        }
        return "Could not complete the GOG request. Check your connection and retry synchronization.".into();
    }
    "Could not load or cache this content. Retry synchronization.".into()
}

fn finish_image(
    artwork: Result<Option<PathBuf>>,
    save: impl FnOnce(&std::path::Path) -> Result<()>,
) -> CoverOutcome {
    match artwork {
        Ok(Some(path)) => match save(&path) {
            Ok(()) => CoverOutcome::Loaded(path),
            Err(_) => CoverOutcome::Failed(
                "Could not save this image to the local cache. Retry synchronization.".into(),
            ),
        },
        Ok(None) => CoverOutcome::Unavailable,
        Err(error) if is_http_not_found(&error) => CoverOutcome::Unavailable,
        Err(error) => CoverOutcome::Failed(sync_error_message(&error)),
    }
}

fn merge_core(product: &Product, cached: Option<&Game>) -> Game {
    let mut core = normalize_product(product);
    let Some(cached) = cached else {
        return core;
    };
    core.platforms = Platforms {
        windows: product
            .content_system_compatibility
            .as_ref()
            .and_then(|value| value.windows)
            .unwrap_or(cached.platforms.windows),
        linux: product
            .content_system_compatibility
            .as_ref()
            .and_then(|value| value.linux)
            .unwrap_or(cached.platforms.linux),
        macos: product
            .content_system_compatibility
            .as_ref()
            .and_then(|value| value.osx)
            .unwrap_or(cached.platforms.macos),
    };
    if product.languages.is_none() {
        core.languages = cached.languages.clone();
    }
    if product.links.is_none() {
        core.links = cached.links.clone();
    }
    let mut metadata = cached.metadata.clone();
    if metadata.localizations.is_empty() {
        metadata.localizations = core.metadata.localizations;
    }
    Game {
        product_id: core.product_id,
        slug: core.slug,
        title: core.title,
        release_date: core.release_date.or(cached.release_date),
        platforms: core.platforms,
        languages: core.languages,
        links: core.links,
        metadata,
        ..cached.clone()
    }
}

pub fn apply_core_product(current: &mut Game, incoming: Game) {
    if current.product_id != incoming.product_id {
        return;
    }
    current.slug = incoming.slug;
    current.title = incoming.title;
    current.release_date = incoming.release_date.or(current.release_date);
    current.platforms = incoming.platforms;
    current.languages = incoming.languages;
    current.links = incoming.links;
    if current.metadata.localizations.is_empty() {
        current.metadata.localizations = incoming.metadata.localizations;
    }
    if incoming.artwork.is_some() {
        current.artwork = incoming.artwork;
    }
    if incoming.icon.is_some() {
        current.icon = incoming.icon;
    }
    for child in &mut current.dlcs {
        child.owned = incoming
            .dlcs
            .iter()
            .any(|dlc| dlc.product_id == child.product_id && dlc.owned);
    }
    for child in incoming.dlcs {
        if let Some(existing) = current
            .dlcs
            .iter_mut()
            .find(|dlc| dlc.product_id == child.product_id)
        {
            existing.title = child.title;
            existing.slug = child.slug;
            existing.platforms = child.platforms;
            existing.languages = child.languages;
        } else {
            current.dlcs.push(child);
        }
    }
    current.dlc_count = current.dlcs.len();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DetailSection {
    Product,
    Metadata,
    Artwork,
    Acquisition,
    Builds,
}

impl DetailSection {
    fn source(self) -> &'static str {
        match self {
            Self::Product => "detail_product",
            Self::Metadata => "detail_metadata",
            Self::Artwork => "detail_artwork",
            Self::Acquisition => "detail_acquisition",
            Self::Builds => "detail_builds",
        }
    }
}

pub fn section_ready(product_id: i64, section: DetailSection) -> Result<bool> {
    let store = crate::state::StateStore::open()?;
    if !store
        .enrichment_observation(product_id, section.source())?
        .is_some_and(|(status, checked)| {
            status == "available"
                && chrono::Utc::now().timestamp().saturating_sub(checked)
                    < if matches!(section, DetailSection::Acquisition | DetailSection::Builds) {
                        300
                    } else {
                        7 * 24 * 60 * 60
                    }
        })
    {
        return Ok(false);
    }
    let Some(game) = store.cached_product_game(product_id)? else {
        return Ok(false);
    };
    Ok(section != DetailSection::Artwork
        || [&game.detail_artwork, &game.hero_logo, &game.icon]
            .into_iter()
            .flatten()
            .all(|path| path.is_file()))
}

pub fn invalidate_section_cache(product_id: i64, section: DetailSection) -> Result<()> {
    crate::state::StateStore::open()?.clear_enrichment_observation(product_id, section.source())
}

pub fn invalidate_all_section_cache(section: DetailSection) -> Result<()> {
    crate::state::StateStore::open()?.clear_enrichment_source(section.source())
}

/// Apply only fields observed by this request, preserving newer unrelated results and entitlement.
pub fn apply_product_section(current: &mut Game, fetched: Game, section: DetailSection) {
    if current.product_id != fetched.product_id {
        return;
    }
    match section {
        DetailSection::Product => {
            current.description = fetched.description;
            current.changelog = fetched.changelog;
            current.screenshots = fetched.screenshots;
            for mut child in fetched.dlcs {
                if let Some(existing) = current
                    .dlcs
                    .iter_mut()
                    .find(|dlc| dlc.product_id == child.product_id)
                {
                    existing.description = child.description;
                    existing.changelog = child.changelog;
                    existing.screenshots = child.screenshots;
                } else {
                    child.owned = false;
                    current.dlcs.push(child);
                }
            }
            current.dlc_count = current.dlcs.len();
        }
        DetailSection::Metadata => {
            current.features = fetched
                .metadata
                .features
                .iter()
                .map(|term| term.name.clone())
                .collect();
            current.metadata = fetched.metadata;
        }
        DetailSection::Artwork => {
            if fetched.detail_artwork.is_some() {
                current.detail_artwork = fetched.detail_artwork;
            }
            if fetched.hero_logo.is_some() {
                current.hero_logo = fetched.hero_logo;
            }
            if fetched.icon.is_some() {
                current.icon = fetched.icon;
            }
        }
        DetailSection::Acquisition => {
            current.remote_artifacts = fetched.remote_artifacts;
            for child in fetched.dlcs {
                if let Some(existing) = current
                    .dlcs
                    .iter_mut()
                    .find(|dlc| dlc.product_id == child.product_id && dlc.owned)
                {
                    existing.remote_artifacts = child.remote_artifacts;
                }
            }
        }
        DetailSection::Builds => {
            current.galaxy_builds = fetched.galaxy_builds;
        }
    }
}

pub fn fetch_product_section(
    game: &Game,
    section: DetailSection,
    access_token: Option<&str>,
    preferred_language: Option<&str>,
    session: u64,
) -> Result<Game> {
    anyhow::ensure!(
        account_session() == session,
        "Account changed before metadata loading started"
    );
    let store = crate::state::StateStore::open()?;
    if section_ready(game.product_id, section)? {
        return Ok(store
            .cached_product_game(game.product_id)?
            .unwrap_or_else(|| game.clone()));
    }
    let client = crate::gog::client()?;
    let mut result = game.clone();
    let mut artwork_failures = Vec::new();
    let mut manifests = Vec::new();
    let mut acquired = None;
    match section {
        DetailSection::Product => {
            let product: Product = serde_json::from_value(crate::gog::product::fetch_expanded(
                &client,
                game.product_id,
                "description,screenshots,changelog,expanded_dlcs",
            )?)?;
            let core = normalize_product(&product);
            result.description = core.description;
            result.changelog = core.changelog;
            result.screenshots = core.screenshots;
            result.dlcs.clear();
            for child in product.expanded_dlcs.unwrap_or_default() {
                result
                    .dlcs
                    .push(game_into_dlc(normalize_product(&child), false));
            }
            result.dlc_count = result.dlcs.len();
        }
        DetailSection::Metadata => {
            let mut metadata = match crate::gog::store::fetch(&client, game.product_id) {
                Ok(metadata) => metadata,
                Err(error) if is_http_not_found(&error) => ProductMetadata::default(),
                Err(error) => return Err(error),
            };
            let observation = store.enrichment_observation(game.product_id, "gamesdb")?;
            if gamesdb_refresh_due(
                observation.as_ref(),
                game.metadata.gamesdb_media_checked,
                chrono::Utc::now().timestamp(),
                false,
            ) {
                match crate::gog::gamesdb::fetch(&client, game.product_id)? {
                    Some(enrichment) => {
                        merge_metadata(&mut metadata, enrichment);
                        store.record_enrichment_observation(
                            game.product_id,
                            "gamesdb",
                            "available",
                        )?;
                    }
                    None => store.record_enrichment_observation(
                        game.product_id,
                        "gamesdb",
                        "not_found",
                    )?,
                }
            } else if observation
                .as_ref()
                .is_some_and(|(status, _)| status == "available")
            {
                merge_metadata(&mut metadata, game.metadata.clone());
            }
            result.metadata = metadata;
        }
        DetailSection::Artwork => {
            let product: Product = serde_json::from_value(crate::gog::product::fetch_expanded(
                &client,
                game.product_id,
                "",
            )?)?;
            let asset = asset_request(&product);
            match cache_detail_artwork(
                &client,
                &detail_artwork_candidates(&game.metadata, asset.detail_artwork_urls),
                std::path::Path::new("detail.png"),
            ) {
                Ok(path) => result.detail_artwork = path.or(result.detail_artwork),
                Err(error) => artwork_failures.push(sync_error_message(&error)),
            };
            match crate::gog::store::fetch_wordmark(&client, &game.slug).and_then(|wordmark| {
                cache_wordmark(
                    &client,
                    wordmark.as_deref(),
                    std::path::Path::new("hero-logo.png"),
                )
            }) {
                Ok(path) => result.hero_logo = path.or(result.hero_logo),
                Err(error) => artwork_failures.push(sync_error_message(&error)),
            }
            match cache_asset(
                &client,
                asset.icon_url.as_deref(),
                std::path::Path::new("icon.png"),
            ) {
                Ok(path) => result.icon = path.or(result.icon),
                Err(error) => artwork_failures.push(sync_error_message(&error)),
            };
        }
        DetailSection::Acquisition => {
            let access_token = access_token.context("Sign in to load available downloads")?;
            let dlcs = game
                .dlcs
                .iter()
                .filter(|dlc| dlc.owned)
                .map(|dlc| (dlc.product_id, dlc.title.clone()))
                .collect::<Vec<_>>();
            for (id, artifacts, _) in
                fetch_product_file_metadata(access_token, game.product_id, &dlcs)?
            {
                apply_remote_artifacts(std::slice::from_mut(&mut result), id, artifacts.clone());
                manifests.push((id, artifacts));
            }
        }
        DetailSection::Builds => {
            let access_token = access_token.context("Sign in to load available Galaxy builds")?;
            let mut builds = Vec::new();
            for (os, supported) in [
                (
                    "windows",
                    game.platforms.windows || !game.platforms.linux && !game.platforms.macos,
                ),
                ("osx", game.platforms.macos),
            ] {
                if !supported {
                    continue;
                }
                let values = crate::gog::builds::fetch(&client, game.product_id, os)?;
                builds.extend(values);
            }
            if let Some(build) = builds
                .iter()
                .filter(|build| {
                    build.generation == 2
                        && build.branch.is_none()
                        && build.operating_system == "windows"
                })
                .max_by_key(|build| (build.published_at.unwrap_or_default(), build.last_seen_at))
            {
                let selection = galaxy_selection(game, preferred_language);
                if !crate::installation::depot_planner::cached_acquisition_available(
                    &store, build, &selection,
                )? {
                    let acquisition = crate::gog::depot_acquisition::acquire(
                        &client,
                        access_token,
                        build,
                        &selection,
                    )?;
                    acquired = Some((build.clone(), acquisition));
                }
            }
            result.galaxy_builds = builds;
        }
    }
    let _account = ACCOUNT_COMMIT
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    anyhow::ensure!(
        account_session() == session,
        "Account changed while metadata was loading; retry for the current account"
    );
    for (id, artifacts) in manifests {
        store.observe_download_manifest(id, &artifacts)?;
        store.cache_download_manifest(id, &artifacts)?;
    }
    if section == DetailSection::Builds {
        for (os, supported) in [
            (
                "windows",
                game.platforms.windows || !game.platforms.linux && !game.platforms.macos,
            ),
            ("osx", game.platforms.macos),
        ] {
            if supported {
                store.observe_galaxy_builds(
                    game.product_id,
                    os,
                    &result
                        .galaxy_builds
                        .iter()
                        .filter(|build| build.operating_system == os)
                        .cloned()
                        .collect::<Vec<_>>(),
                )?;
            }
        }
    }
    if let Some((build, acquisition)) = acquired {
        crate::installation::depot_planner::cache_acquisition(&store, &acquisition, &build)?;
    }
    store.cache_product_section(&result, section)?;
    if !artwork_failures.is_empty() {
        store.record_enrichment_observation(game.product_id, section.source(), "partial")?;
        return Err(PartialSectionError {
            game: result,
            message: format!(
                "Some artwork could not load: {}",
                artwork_failures.join("; ")
            ),
        }
        .into());
    }
    store.record_enrichment_observation(game.product_id, section.source(), "available")?;
    Ok(result)
}

#[derive(Debug)]
pub struct PartialSectionError {
    pub game: Game,
    pub message: String,
}

impl std::fmt::Display for PartialSectionError {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.write_str(&self.message)
    }
}
impl std::error::Error for PartialSectionError {}
fn galaxy_selection(
    game: &Game,
    preferred_installer_language: Option<&str>,
) -> crate::gog::depot_acquisition::Selection {
    let language = game
        .metadata
        .localizations
        .iter()
        .find(|localization| {
            preferred_installer_language.is_some_and(|preferred| {
                localization.name.eq_ignore_ascii_case(preferred)
                    || localization.language_code.eq_ignore_ascii_case(preferred)
            })
        })
        .or_else(|| {
            game.metadata
                .localizations
                .iter()
                .find(|localization| localization.language_code.starts_with("en"))
        })
        .map(|localization| localization.language_code.clone())
        .unwrap_or_else(|| "en".into());
    let owned_dlc = game
        .dlcs
        .iter()
        .filter(|dlc| dlc.owned)
        .map(|dlc| dlc.product_id)
        .collect::<std::collections::BTreeSet<_>>();
    crate::gog::depot_acquisition::Selection {
        language,
        bitness: Some("64".into()),
        owned_dlc: owned_dlc.clone(),
        selected_dlc: owned_dlc,
    }
}

fn merge_metadata(target: &mut ProductMetadata, enrichment: ProductMetadata) {
    target.genres = enrichment.genres;
    target.themes = enrichment.themes;
    target.game_modes = enrichment.game_modes;
    target.gamesdb_summary = enrichment.gamesdb_summary;
    target.gamesdb_artwork_url = enrichment.gamesdb_artwork_url;
    target.gamesdb_horizontal_artwork_url = enrichment.gamesdb_horizontal_artwork_url;
    target.gamesdb_background_url = enrichment.gamesdb_background_url;
    target.gamesdb_media_checked = enrichment.gamesdb_media_checked;
    target.gamesdb_media_version = enrichment.gamesdb_media_version;
    if target.developers.is_empty() {
        target.developers = enrichment.developers;
    }
    if target.publishers.is_empty() {
        target.publishers = enrichment.publishers;
    }
}

fn detail_artwork_candidates(
    metadata: &ProductMetadata,
    product_backgrounds: Vec<String>,
) -> Vec<String> {
    let mut candidates = [
        metadata.store_galaxy_background_url.clone(),
        metadata.gamesdb_artwork_url.clone(),
        metadata.gamesdb_horizontal_artwork_url.clone(),
        metadata.gamesdb_background_url.clone(),
    ]
    .into_iter()
    .flatten()
    .chain(product_backgrounds)
    .collect::<Vec<_>>();
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|url| seen.insert(normalize_asset_url(url)));
    candidates
}

fn apply_remote_artifacts(games: &mut [Game], product_id: i64, artifacts: Vec<RemoteArtifact>) {
    for game in games {
        if game.product_id == product_id {
            game.remote_artifacts = artifacts;
            return;
        }
        if let Some(dlc) = game
            .dlcs
            .iter_mut()
            .find(|dlc| dlc.product_id == product_id)
        {
            dlc.remote_artifacts = artifacts;
            return;
        }
    }
}

fn fetch_download_manifest(
    client: &reqwest::blocking::Client,
    access_token: &str,
    product_id: i64,
    dlcs: &[(i64, String)],
) -> Result<DownloadManifest> {
    let response = client
        .get(format!(
            "https://embed.gog.com/account/gameDetails/{product_id}.json"
        ))
        .bearer_auth(access_token)
        .send()?
        .error_for_status()?;
    let response = String::from_utf8(crate::gog::product::bounded_metadata(response)?)?;
    let value: serde_json::Value = serde_json::from_str(&response)?;
    let dlc_manifests = normalize_dlc_download_artifacts(&value, dlcs);
    Ok((
        normalize_download_artifacts(product_id, &value),
        response,
        dlc_manifests,
    ))
}

pub fn fetch_product_file_metadata(
    access_token: &str,
    product_id: i64,
    dlcs: &[(i64, String)],
) -> Result<Vec<(i64, Vec<RemoteArtifact>, String)>> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(45))
        .user_agent(crate::identity::USER_AGENT)
        .build()?;
    if let Ok(product) =
        crate::gog::product::fetch_expanded(&client, product_id, "downloads,expanded_dlcs")
    {
        let artifacts = crate::gog::product::download_artifacts(product_id, &product);
        if !artifacts.is_empty() || product.get("downloads").is_some() {
            let mut manifests = vec![(product_id, artifacts, String::new())];
            let wanted = dlcs
                .iter()
                .map(|(id, _)| *id)
                .collect::<std::collections::HashSet<_>>();
            for dlc in product
                .get("expanded_dlcs")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(id) = dlc.get("id").and_then(serde_json::Value::as_i64) else {
                    continue;
                };
                if wanted.contains(&id) {
                    manifests.push((
                        id,
                        crate::gog::product::download_artifacts(id, dlc),
                        String::new(),
                    ));
                }
            }
            return Ok(manifests);
        }
    }
    tracing::warn!(
        product_id,
        "structured Product API lacked downloads; using compatibility provider"
    );
    let (artifacts, raw_json, dlc_manifests) =
        fetch_download_manifest(&client, access_token, product_id, dlcs)?;
    let mut manifests = vec![(product_id, artifacts, raw_json)];
    manifests.extend(dlc_manifests);
    Ok(manifests)
}

fn normalize_dlc_download_artifacts(
    value: &serde_json::Value,
    known_dlcs: &[(i64, String)],
) -> Vec<(i64, Vec<RemoteArtifact>, String)> {
    fn visit(
        value: &serde_json::Value,
        known_dlcs: &[(i64, String)],
        artifacts: &mut Vec<(i64, Vec<RemoteArtifact>, String)>,
    ) {
        let Some(dlcs) = value.get("dlcs").and_then(serde_json::Value::as_array) else {
            return;
        };
        for dlc in dlcs {
            let title = dlc
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let key = product_title_key(title);
            if let Some((product_id, _)) = known_dlcs
                .iter()
                .find(|(_, known_title)| product_title_key(known_title) == key)
            {
                artifacts.push((
                    *product_id,
                    normalize_download_artifacts(*product_id, dlc),
                    serde_json::to_string(dlc).unwrap_or_else(|_| "{}".into()),
                ));
            }
            visit(dlc, known_dlcs, artifacts);
        }
    }

    let mut artifacts = Vec::new();
    visit(value, known_dlcs, &mut artifacts);
    artifacts
}

fn product_title_key(title: &str) -> String {
    title
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub(crate) fn normalize_download_artifacts(
    product_id: i64,
    value: &serde_json::Value,
) -> Vec<RemoteArtifact> {
    let mut artifacts = Vec::new();
    if let Some(downloads) = value.get("downloads").and_then(serde_json::Value::as_array) {
        for language_entry in downloads {
            let Some(entry) = language_entry.as_array() else {
                continue;
            };
            let language = entry.first().and_then(serde_json::Value::as_str);
            let Some(systems) = entry.get(1).and_then(serde_json::Value::as_object) else {
                continue;
            };
            for (system, files) in systems {
                let Some(files) = files.as_array() else {
                    continue;
                };
                for file in files {
                    if let Some(artifact) =
                        normalize_download_file(product_id, file, language, Some(system), None)
                    {
                        artifacts.push(artifact);
                    }
                }
            }
        }
    }
    if let Some(extras) = value.get("extras").and_then(serde_json::Value::as_array) {
        for file in extras {
            if let Some(artifact) =
                normalize_download_file(product_id, file, None, None, Some(ArtifactKind::Extra))
            {
                artifacts.push(artifact);
            }
        }
    }
    artifacts
}

fn normalize_download_file(
    product_id: i64,
    value: &serde_json::Value,
    language: Option<&str>,
    system: Option<&str>,
    forced_kind: Option<ArtifactKind>,
) -> Option<RemoteArtifact> {
    let download_path = value
        .get("manualUrl")
        .or_else(|| value.get("path"))?
        .as_str()?
        .to_owned();
    let name = value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("GOG download")
        .to_owned();
    let lower = format!("{} {}", name.to_lowercase(), download_path.to_lowercase());
    let kind = forced_kind.unwrap_or_else(|| {
        if lower.contains("patch") || lower.contains("update") || lower.contains("hotfix") {
            ArtifactKind::Patch
        } else {
            ArtifactKind::Installer
        }
    });
    let (part_number, part_count) = multipart_numbers(&name);
    let size_label = value
        .get("size")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    Some(RemoteArtifact {
        product_id,
        kind,
        name,
        language: language.map(str::to_owned),
        operating_system: system.map(str::to_owned),
        version: value
            .get("version")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        release_date: value
            .get("date")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        size_bytes: size_label.as_deref().and_then(parse_size_label),
        size_label,
        part_number,
        part_count,
        download_path,
        provider_group_id: None,
        provider_file_id: None,
        provider_category: None,
    })
}

fn multipart_numbers(name: &str) -> (Option<u32>, Option<u32>) {
    let Some((_, suffix)) = name.rsplit_once("(Part ") else {
        return (None, None);
    };
    let Some((numbers, _)) = suffix.split_once(')') else {
        return (None, None);
    };
    let Some((part, total)) = numbers.split_once(" of ") else {
        return (None, None);
    };
    (part.parse().ok(), total.parse().ok())
}

fn parse_size_label(value: &str) -> Option<u64> {
    let mut parts = value.split_whitespace();
    let number = parts.next()?.replace(',', ".").parse::<f64>().ok()?;
    let multiplier = match parts.next()?.to_ascii_lowercase().as_str() {
        "b" => 1.0,
        "kb" => 1_000.0,
        "mb" => 1_000_000.0,
        "gb" => 1_000_000_000.0,
        "tb" => 1_000_000_000_000.0,
        _ => return None,
    };
    Some((number * multiplier) as u64)
}

fn fetch_account_dlc_fallbacks(
    client: &reqwest::blocking::Client,
    access_token: &str,
    products: &[Product],
    unresolved_ids: &std::collections::HashSet<i64>,
) -> Result<Vec<Product>> {
    let mut fallbacks = Vec::new();
    for parent in products
        .iter()
        .filter(|product| product.game_type == "game")
    {
        let Some(references) = parent.dlcs.as_ref().map(|dlcs| &dlcs.products) else {
            continue;
        };
        if !references
            .iter()
            .any(|dlc| unresolved_ids.contains(&dlc.id))
        {
            continue;
        }
        let details: serde_json::Value = client
            .get(format!(
                "https://embed.gog.com/account/gameDetails/{}.json",
                parent.id
            ))
            .bearer_auth(access_token)
            .send()?
            .error_for_status()?
            .json()?;
        let account_dlcs = details
            .get("dlcs")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        for (reference, details) in references
            .iter()
            .filter(|reference| unresolved_ids.contains(&reference.id))
            .zip(account_dlcs.iter())
        {
            let serialized = details.to_string();
            let slug = download_slug(details).unwrap_or_else(|| reference.id.to_string());
            let languages = details
                .get("downloads")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|entry| entry.as_array()?.first()?.as_str().map(str::to_owned))
                .collect::<Vec<_>>();
            fallbacks.push(Product {
                id: reference.id,
                slug,
                title: details
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("GOG DLC")
                    .to_owned(),
                release_date: None,
                description: None,
                changelog: details
                    .get("changelog")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                content_system_compatibility: Some(Compatibility {
                    windows: Some(serialized.contains("windows")),
                    linux: Some(serialized.contains("linux")),
                    osx: Some(serialized.contains("mac")),
                }),
                languages: Some(serde_json::to_value(languages)?),
                links: Some(Links {
                    product_card: None,
                    forum: details
                        .get("forumLink")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                    support: None,
                }),
                images: Some(Images {
                    background: details
                        .get("backgroundImage")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                    logo2x: None,
                    logo: None,
                    sidebar_icon2x: None,
                    sidebar_icon: None,
                    icon: None,
                }),
                screenshots: None,
                game_type: "dlc".into(),
                is_installable: true,
                dlcs: None,
                expanded_dlcs: None,
            });
        }
    }
    Ok(fallbacks)
}

fn download_slug(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => value
            .split_once("/downloads/")
            .and_then(|(_, rest)| rest.split('/').next())
            .filter(|slug| !slug.is_empty())
            .map(str::to_owned),
        serde_json::Value::Array(values) => values.iter().find_map(download_slug),
        serde_json::Value::Object(values) => values.values().find_map(download_slug),
        _ => None,
    }
}

fn apply_media(
    games: &mut [Game],
    product_id: i64,
    artwork: Option<PathBuf>,
    detail_artwork: Option<PathBuf>,
    hero_logo: Option<PathBuf>,
    icon: Option<PathBuf>,
) {
    for game in games {
        if game.product_id == product_id {
            if let Some(path) = &artwork {
                game.artwork = Some(path.clone());
            }
            if let Some(path) = &detail_artwork {
                game.detail_artwork = Some(path.clone());
            }
            if let Some(path) = &hero_logo {
                game.hero_logo = Some(path.clone());
            }
            if let Some(path) = icon {
                game.icon = Some(path);
            }
            for dlc in &mut game.dlcs {
                if dlc.artwork.is_none() {
                    dlc.artwork = artwork.clone();
                }
                if dlc.detail_artwork.is_none() {
                    dlc.detail_artwork = detail_artwork.clone();
                }
                if dlc.hero_logo.is_none() {
                    dlc.hero_logo = hero_logo.clone();
                }
            }
            return;
        }
        let parent_artwork = game.artwork.clone();
        let parent_detail_artwork = game.detail_artwork.clone();
        let parent_hero_logo = game.hero_logo.clone();
        if let Some(dlc) = game
            .dlcs
            .iter_mut()
            .find(|dlc| dlc.product_id == product_id)
        {
            if let Some(path) = artwork.or(parent_artwork.filter(|_| dlc.artwork.is_none())) {
                dlc.artwork = Some(path);
            }
            if let Some(path) =
                detail_artwork.or(parent_detail_artwork.filter(|_| dlc.detail_artwork.is_none()))
            {
                dlc.detail_artwork = Some(path);
            }
            if let Some(path) = hero_logo.or(parent_hero_logo.filter(|_| dlc.hero_logo.is_none())) {
                dlc.hero_logo = Some(path);
            }
            if let Some(path) = icon {
                dlc.icon = Some(path);
            }
            return;
        }
    }
}

fn game_into_dlc(game: Game, owned: bool) -> Dlc {
    Dlc {
        product_id: game.product_id,
        owned,
        slug: game.slug,
        title: game.title,
        release_date: game.release_date,
        description: game.description,
        changelog: game.changelog,
        platforms: game.platforms,
        languages: game.languages,
        metadata: game.metadata,
        galaxy_builds: game.galaxy_builds,
        location: game.location,
        artwork: game.artwork,
        detail_artwork: game.detail_artwork,
        hero_logo: game.hero_logo,
        icon: game.icon,
        screenshots: game.screenshots,
        links: game.links,
        installers: game.installers,
        extras: game.extras,
        remote_artifacts: game.remote_artifacts,
        disk_usage: game.disk_usage,
    }
}

fn normalize_product(product: &Product) -> Game {
    let compatibility = product.content_system_compatibility.as_ref();
    Game {
        product_id: product.id,
        slug: product.slug.clone(),
        title: product.title.clone(),
        release_date: product
            .release_date
            .as_deref()
            .and_then(|date| DateTime::parse_from_str(date, "%Y-%m-%dT%H:%M:%S%z").ok()),
        description: product
            .description
            .as_ref()
            .and_then(|description| description.full.clone().or(description.lead.clone()))
            .unwrap_or_default(),
        changelog: product.changelog.clone().unwrap_or_default(),
        platforms: Platforms {
            windows: compatibility
                .and_then(|value| value.windows)
                .unwrap_or(false),
            linux: compatibility.and_then(|value| value.linux).unwrap_or(false),
            macos: compatibility.and_then(|value| value.osx).unwrap_or(false),
        },
        features: Vec::new(),
        languages: normalize_languages(product.languages.clone()),
        metadata: ProductMetadata {
            localizations: product
                .languages
                .as_ref()
                .and_then(serde_json::Value::as_object)
                .into_iter()
                .flatten()
                .filter_map(|(code, name)| {
                    Some(crate::domain::ProductLocalization {
                        language_code: code.clone(),
                        name: name.as_str()?.to_owned(),
                        text: false,
                        audio: false,
                    })
                })
                .collect(),
            ..Default::default()
        },
        galaxy_builds: Vec::new(),
        location: PathBuf::new(),
        artwork: None,
        detail_artwork: None,
        hero_logo: None,
        icon: None,
        screenshots: normalize_screenshots(product.screenshots.as_ref()),
        links: product
            .links
            .as_ref()
            .map_or_else(ExternalLinks::default, |links| ExternalLinks {
                store: links.product_card.clone(),
                forum: links.forum.clone(),
                support: links.support.clone(),
            }),
        installers: Vec::new(),
        patches: Vec::new(),
        extras: Vec::new(),
        remote_artifacts: Vec::new(),
        dlc_count: 0,
        dlcs: Vec::new(),
        disk_usage: 0,
    }
}

fn asset_request(product: &Product) -> AssetRequest {
    let images = product.images.as_ref();
    let generated_tile_url = images
        .and_then(|images| images.logo2x.as_deref().or(images.logo.as_deref()))
        .and_then(product_tile_url);
    let background_url = images.and_then(|images| images.background.clone());
    let tile_url = generated_tile_url
        .clone()
        .or_else(|| background_url.clone());
    let artwork_fallback_url = generated_tile_url.and(background_url);
    // The product background is the clean, text-free art intended to sit behind
    // detail-page UI. The logo cover already contains the game's wordmark and
    // caused the native title below it to be shown twice.
    let detail_artwork_urls = images
        .and_then(|images| images.background.clone())
        .into_iter()
        .collect();
    let icon_url = images.and_then(|images| {
        images
            .sidebar_icon2x
            .clone()
            .or(images.sidebar_icon.clone())
            .or(images.icon.clone())
    });
    AssetRequest {
        artwork_url: tile_url,
        artwork_fallback_url,
        detail_artwork_urls,
        icon_url,
    }
}

fn cache_asset_with_fallback(
    client: &reqwest::blocking::Client,
    primary_url: Option<&str>,
    fallback_url: Option<&str>,
    path: &std::path::Path,
) -> Result<Option<PathBuf>> {
    for url in [primary_url, fallback_url].into_iter().flatten() {
        let normalized = normalize_asset_url(url);
        let shared = shared_asset_path(&normalized, path, false);
        let _guard = asset_lock(&shared);
        if asset_is_current(&shared, &normalized) && validate_cover_file(&shared).is_ok() {
            return Ok(Some(shared));
        }
    }
    let fetch = |url: Option<&str>| -> Result<Option<PathBuf>> {
        let Some(url) = url else {
            return Ok(None);
        };
        let url = normalize_asset_url(url);
        let shared = shared_asset_path(&url, path, false);
        cache_cover_at(client, &url, &shared).map(Some)
    };
    match fetch(primary_url) {
        Ok(asset) => Ok(asset),
        Err(_) if fallback_url.is_some() => fetch(fallback_url),
        Err(error) => Err(error),
    }
}

const COVER_MAX_BYTES: u64 = 16 * 1024 * 1024;

fn validate_cover_file(path: &std::path::Path) -> Result<()> {
    use std::io::Read;
    let file = fs::File::open(path)?;
    anyhow::ensure!(
        file.metadata()?.len() <= COVER_MAX_BYTES,
        "Cover exceeds its size limit"
    );
    let mut bytes = Vec::new();
    file.take(COVER_MAX_BYTES + 1).read_to_end(&mut bytes)?;
    validate_cover(&bytes)
}

fn validate_cover(bytes: &[u8]) -> Result<()> {
    use gdk_pixbuf::prelude::PixbufLoaderExt;
    anyhow::ensure!(
        bytes.len() as u64 <= COVER_MAX_BYTES,
        "Cover exceeds its size limit"
    );
    let loader = gdk_pixbuf::PixbufLoader::new();
    let too_large = std::rc::Rc::new(std::cell::Cell::new(false));
    let check = too_large.clone();
    loader.connect_size_prepared(move |loader, width, height| {
        if width <= 0
            || height <= 0
            || width > 8192
            || height > 8192
            || i64::from(width) * i64::from(height) > 16 * 1024 * 1024
        {
            check.set(true);
            loader.set_size(1, 1);
        }
    });
    let written = loader.write(bytes);
    let closed = loader.close();
    anyhow::ensure!(!too_large.get(), "Cover dimensions exceed their limit");
    written.context("Cover is not a supported image")?;
    closed.context("Cover image is incomplete")?;
    anyhow::ensure!(loader.pixbuf().is_some(), "Cover has no image data");
    Ok(())
}

pub(crate) fn cache_cover_at(
    client: &reqwest::blocking::Client,
    url: &str,
    path: &std::path::Path,
) -> Result<PathBuf> {
    use std::io::Read;
    let _guard = asset_lock(path);
    if asset_is_current(path, url) && validate_cover_file(path).is_ok() {
        return Ok(path.to_owned());
    }
    fs::create_dir_all(path.parent().context("Cover cache path has no parent")?)?;
    let response = client.get(url).send()?.error_for_status()?;
    anyhow::ensure!(
        response
            .content_length()
            .is_none_or(|size| size <= COVER_MAX_BYTES),
        "Cover exceeds its size limit"
    );
    let manifest = AssetManifest {
        source_url: url.to_owned(),
        etag: response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        last_modified: response
            .headers()
            .get(reqwest::header::LAST_MODIFIED)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        usable: true,
        wordmark_processed: false,
        wordmark_processing_version: 0,
    };
    let mut bytes = Vec::new();
    response.take(COVER_MAX_BYTES + 1).read_to_end(&mut bytes)?;
    validate_cover(&bytes)?;
    let temporary = path.with_extension("part");
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)?;
    let manifest_temporary = path.with_extension("source.part");
    fs::write(&manifest_temporary, serde_json::to_vec(&manifest)?)?;
    fs::rename(manifest_temporary, path.with_extension("source.json"))?;
    Ok(path.to_owned())
}

const HERO_MINIMUM_ASPECT_RATIO: f64 = 3.5;
const HERO_WIDTH: u32 = 2560;
const HERO_HEIGHT: u32 = 670;
const HERO_PROCESSING_VERSION: u32 = 1;

fn cache_detail_artwork(
    client: &reqwest::blocking::Client,
    urls: &[String],
    suggested_path: &std::path::Path,
) -> Result<Option<PathBuf>> {
    let mut first = None;
    let mut last_error = None;
    for url in urls {
        match cache_asset(client, Some(url), suggested_path) {
            Ok(Some(path)) => {
                first.get_or_insert_with(|| path.clone());
                if hero_aspect_is_suitable(&path) {
                    return Ok(Some(path));
                }
            }
            Ok(None) => {}
            Err(error) => last_error = Some(error),
        }
    }
    if let Some(source) = first {
        return process_extended_hero(&source).map(Some);
    }
    match last_error {
        Some(error) => Err(error),
        None => Ok(None),
    }
}

fn hero_aspect_is_suitable(path: &std::path::Path) -> bool {
    gdk_pixbuf::Pixbuf::from_file(path).is_ok_and(|image| {
        image.height() > 0
            && image.width() as f64 / image.height() as f64 >= HERO_MINIMUM_ASPECT_RATIO
    })
}

fn process_extended_hero(source: &std::path::Path) -> Result<PathBuf> {
    use image::{DynamicImage, GenericImageView, Rgba, imageops::FilterType};
    use sha2::{Digest, Sha256};

    let source_key = format!(
        "{:x}",
        Sha256::digest(source.as_os_str().as_encoded_bytes())
    );
    let output = crate::identity::cache_root()
        .join("media")
        .join(format!("heroes-v{HERO_PROCESSING_VERSION}"))
        .join(source_key)
        .join("hero.jpg");
    let _guard = asset_lock(&output);
    if output.is_file() && output.metadata().is_ok_and(|metadata| metadata.len() > 0) {
        return Ok(output);
    }
    fs::create_dir_all(
        output
            .parent()
            .context("processed hero path has no parent")?,
    )?;
    let source_image = image::open(source)
        .with_context(|| format!("could not decode hero source {}", source.display()))?;
    let mut background = source_image
        .resize_to_fill(HERO_WIDTH / 4, HERO_HEIGHT / 4, FilterType::Lanczos3)
        .blur(9.0)
        .resize_exact(HERO_WIDTH, HERO_HEIGHT, FilterType::Lanczos3)
        .to_rgba8();
    for pixel in background.pixels_mut() {
        pixel.0[0] = (pixel.0[0] as f32 * 0.68) as u8;
        pixel.0[1] = (pixel.0[1] as f32 * 0.68) as u8;
        pixel.0[2] = (pixel.0[2] as f32 * 0.68) as u8;
    }
    let (width, height) = source_image.dimensions();
    let scale =
        (HERO_HEIGHT as f64 / height.max(1) as f64).min(HERO_WIDTH as f64 / width.max(1) as f64);
    let foreground_width = (width as f64 * scale).round().max(1.0) as u32;
    let foreground_height = (height as f64 * scale).round().max(1.0) as u32;
    let foreground = source_image
        .resize_exact(foreground_width, foreground_height, FilterType::Lanczos3)
        .to_rgba8();
    let offset_x = (HERO_WIDTH - foreground_width) / 2;
    let offset_y = (HERO_HEIGHT - foreground_height) / 2;
    let feather = 96_u32.min(foreground_width / 3).max(1);
    for y in 0..foreground_height {
        for x in 0..foreground_width {
            let edge = x.min(foreground_width - 1 - x);
            let blend = (edge as f32 / feather as f32).clamp(0.0, 1.0);
            let foreground_pixel = foreground.get_pixel(x, y);
            let background_pixel = background.get_pixel_mut(offset_x + x, offset_y + y);
            for channel in 0..3 {
                background_pixel.0[channel] = (foreground_pixel.0[channel] as f32 * blend
                    + background_pixel.0[channel] as f32 * (1.0 - blend))
                    as u8;
            }
            *background_pixel = Rgba([
                background_pixel.0[0],
                background_pixel.0[1],
                background_pixel.0[2],
                255,
            ]);
        }
    }
    let temporary = output.with_extension("part.jpg");
    DynamicImage::ImageRgba8(background).save_with_format(&temporary, image::ImageFormat::Jpeg)?;
    fs::rename(temporary, &output)?;
    Ok(output)
}

fn product_tile_url(url: &str) -> Option<String> {
    let (base, _) = url.split_once("_glx_logo")?;
    Some(format!("{base}_392.jpg"))
}

fn normalize_languages(value: Option<serde_json::Value>) -> Vec<String> {
    match value {
        Some(serde_json::Value::Object(values)) => values
            .into_values()
            .filter_map(|value| value.as_str().map(str::to_owned))
            .collect(),
        Some(serde_json::Value::Array(values)) => values
            .into_iter()
            .filter_map(|value| {
                value.as_str().map(str::to_owned).or_else(|| {
                    value
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn normalize_screenshots(values: Option<&Vec<ProductScreenshot>>) -> Vec<Screenshot> {
    values
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|screenshot| {
            let images = screenshot.formatted_images.as_deref().unwrap_or_default();
            let find = |name: &str| {
                images.iter().find_map(|image| {
                    (image.formatter_name.as_deref() == Some(name))
                        .then(|| image.image_url.clone())
                        .flatten()
                })
            };
            let thumbnail_url = find("ggvgm")
                .or_else(|| find("ggvgt_2x"))
                .or_else(|| find("ggvgt"))?;
            let full_url = find("ggvgl_2x")
                .or_else(|| find("ggvgl"))
                .unwrap_or_else(|| thumbnail_url.clone());
            Some(Screenshot {
                id: screenshot
                    .image_id
                    .clone()
                    .unwrap_or_else(|| "screenshot".into()),
                thumbnail_url,
                full_url,
            })
        })
        .collect()
}

fn cache_asset(
    client: &reqwest::blocking::Client,
    url: Option<&str>,
    path: &std::path::Path,
) -> Result<Option<PathBuf>> {
    let Some(url) = url else {
        return Ok(None);
    };
    let url = normalize_asset_url(url);
    let path = shared_asset_path(&url, path, false);
    cache_cover_at(client, &url, &path).map(Some)
}

fn is_http_not_found(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<reqwest::Error>()
            .and_then(reqwest::Error::status)
            == Some(reqwest::StatusCode::NOT_FOUND)
    })
}

fn cache_wordmark(
    client: &reqwest::blocking::Client,
    url: Option<&str>,
    path: &std::path::Path,
) -> Result<Option<PathBuf>> {
    let Some(url) = url else {
        return Ok(None);
    };
    let url = normalize_asset_url(url);
    let path = shared_asset_path(&url, path, true);
    let _guard = asset_lock(&path);
    fs::create_dir_all(
        path.parent()
            .context("shared wordmark path has no parent")?,
    )?;
    let manifest_path = path.with_extension("source.json");
    let cached_manifest = fs::read(&manifest_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<AssetManifest>(&bytes).ok())
        .filter(|manifest| manifest.source_url == url);
    if let Some(manifest) = &cached_manifest {
        if !manifest.usable {
            return Ok(None);
        }
        if manifest.wordmark_processed
            && manifest.wordmark_processing_version == WORDMARK_PROCESSING_VERSION
            && path.is_file()
            && validate_cover_file(&path).is_ok()
        {
            return Ok(Some(path.to_owned()));
        }
    }

    if cached_manifest.is_none() || validate_cover_file(&path).is_err() {
        download_asset(client, &url, &path)?;
    }
    let usable = trim_transparent_wordmark(&path)?;
    let mut manifest = fs::read(&manifest_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<AssetManifest>(&bytes).ok())
        .unwrap_or(AssetManifest {
            source_url: url,
            etag: None,
            last_modified: None,
            usable,
            wordmark_processed: true,
            wordmark_processing_version: WORDMARK_PROCESSING_VERSION,
        });
    manifest.usable = usable;
    manifest.wordmark_processed = true;
    manifest.wordmark_processing_version = WORDMARK_PROCESSING_VERSION;
    let temporary = manifest_path.with_extension("part");
    fs::write(&temporary, serde_json::to_vec(&manifest)?)?;
    fs::rename(temporary, &manifest_path)?;
    if usable {
        Ok(Some(path))
    } else {
        let _ = fs::remove_file(path);
        Ok(None)
    }
}

fn trim_transparent_wordmark(path: &std::path::Path) -> Result<bool> {
    let pixbuf = Pixbuf::from_file(path)?;
    if !pixbuf.has_alpha() || pixbuf.n_channels() < 4 {
        return Ok(false);
    }
    let width = pixbuf.width();
    let height = pixbuf.height();
    let channels = pixbuf.n_channels() as usize;
    let rowstride = pixbuf.rowstride() as usize;
    let pixels = pixbuf.read_pixel_bytes();
    let pixels = pixels.as_ref();
    let mut left = width;
    let mut top = height;
    let mut right = -1;
    let mut bottom = -1;
    let mut visible = 0usize;
    for y in 0..height {
        for x in 0..width {
            let alpha = pixels[y as usize * rowstride + x as usize * channels + channels - 1];
            // Keep faint antialiasing and glow while ignoring effectively transparent noise.
            if alpha > 2 {
                visible += 1;
                left = left.min(x);
                top = top.min(y);
                right = right.max(x);
                bottom = bottom.max(y);
            }
        }
    }
    if visible == 0 || visible * 100 >= (width as usize * height as usize) * 85 {
        return Ok(false);
    }
    let padding = (width.max(height) / 100).max(4);
    left = (left - padding).max(0);
    top = (top - padding).max(0);
    right = (right + padding).min(width - 1);
    bottom = (bottom + padding).min(height - 1);
    let cropped = pixbuf.new_subpixbuf(left, top, right - left + 1, bottom - top + 1);
    let scale = (360.0 / cropped.width() as f64)
        .min(125.0 / cropped.height() as f64)
        .min(1.0);
    let output = if scale < 1.0 {
        cropped
            .scale_simple(
                (cropped.width() as f64 * scale).round() as i32,
                (cropped.height() as f64 * scale).round() as i32,
                gdk_pixbuf::InterpType::Bilinear,
            )
            .unwrap_or(cropped)
    } else {
        cropped
    };
    let temporary = path.with_extension("trimmed.png");
    output.savev(&temporary, "png", &[])?;
    fs::rename(temporary, path)?;
    Ok(true)
}

fn asset_is_current(path: &std::path::Path, source_url: &str) -> bool {
    if !path.is_file() || !path.metadata().is_ok_and(|metadata| metadata.len() > 0) {
        return false;
    }
    fs::read(path.with_extension("source.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<AssetManifest>(&bytes).ok())
        .is_some_and(|manifest| manifest.source_url == source_url && manifest.usable)
}

fn shared_asset_path(
    source_url: &str,
    suggested_path: &std::path::Path,
    wordmark: bool,
) -> PathBuf {
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(source_url.as_bytes()));
    let namespace = if wordmark {
        format!("wordmarks-v{WORDMARK_PROCESSING_VERSION}")
    } else {
        "originals".to_owned()
    };
    let extension = if wordmark {
        "png".to_owned()
    } else {
        reqwest::Url::parse(source_url)
            .ok()
            .and_then(|url| {
                std::path::Path::new(url.path())
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(str::to_owned)
            })
            .or_else(|| {
                suggested_path
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "img".to_owned())
    };
    crate::identity::cache_root()
        .join("media")
        .join(namespace)
        .join(digest)
        .join(format!("asset.{extension}"))
}

fn download_asset(
    client: &reqwest::blocking::Client,
    url: &str,
    path: &std::path::Path,
) -> Result<()> {
    use std::io::Read;
    let response = client
        .get(url)
        .send()
        .with_context(|| format!("downloading {url}"))?
        .error_for_status()?;
    let manifest = AssetManifest {
        source_url: url.to_owned(),
        etag: response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        last_modified: response
            .headers()
            .get(reqwest::header::LAST_MODIFIED)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        usable: true,
        wordmark_processed: false,
        wordmark_processing_version: 0,
    };
    let temporary = path.with_extension("part");
    anyhow::ensure!(
        response
            .content_length()
            .is_none_or(|size| size <= COVER_MAX_BYTES),
        "Image exceeds its size limit"
    );
    let mut bytes = Vec::new();
    response.take(COVER_MAX_BYTES + 1).read_to_end(&mut bytes)?;
    validate_cover(&bytes)?;
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)?;
    let manifest_path = path.with_extension("source.json");
    let manifest_temporary = path.with_extension("source.part");
    fs::write(&manifest_temporary, serde_json::to_vec(&manifest)?)?;
    fs::rename(manifest_temporary, manifest_path)?;
    Ok(())
}

pub(crate) fn normalize_asset_url(url: &str) -> String {
    if url.starts_with("//") {
        format!("https:{url}")
    } else {
        url.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_generation_reads_do_not_wait_for_commit_but_invalidation_does() {
        let session = account_session();
        let (entered, held) = std::sync::mpsc::channel();
        let (release, ready) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            with_account_session(session, || {
                entered.send(()).unwrap();
                ready.recv().unwrap();
                Ok(())
            })
            .unwrap()
        });
        held.recv().unwrap();
        let started = std::time::Instant::now();
        assert_eq!(account_session(), session);
        crate::installation::request_sign_out_pause();
        assert!(started.elapsed() < Duration::from_millis(100));
        let (done, invalidated) = std::sync::mpsc::channel();
        let invalidator = std::thread::spawn(move || {
            invalidate_library_session();
            done.send(()).unwrap();
        });
        assert!(invalidated.recv_timeout(Duration::from_millis(20)).is_err());
        release.send(()).unwrap();
        holder.join().unwrap();
        invalidated.recv_timeout(Duration::from_secs(1)).unwrap();
        invalidator.join().unwrap();
        assert!(
            with_account_session(session, || -> Result<()> {
                panic!("obsolete commit must not run")
            })
            .is_err()
        );
        crate::installation::finish_sign_out_pause();
    }

    #[test]
    fn screenshot_cache_repairs_corruption_deduplicates_and_hides_sensitive_urls() {
        let corrupt = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let invalid = corrupt.clone();
        let png = cover_png();
        let server = CoverServer::start(move |_| {
            (
                200,
                if invalid.load(std::sync::atomic::Ordering::Acquire) {
                    b"not an image".to_vec()
                } else {
                    png.clone()
                },
                Duration::from_millis(5),
            )
        });
        let screenshot = Screenshot {
            id: "fixture".into(),
            thumbnail_url: format!("{}/image?secret=do-not-log", server.url),
            full_url: format!("{}/image?secret=do-not-log", server.url),
        };
        let error = crate::screenshots::cached_image(7, &screenshot, false).unwrap_err();
        assert!(!format!("{error:#}").contains("do-not-log"));
        assert!(!format!("{error:?}").contains("http"));
        corrupt.store(false, std::sync::atomic::Ordering::Release);
        let paths = std::thread::scope(|scope| {
            (0..12)
                .map(|index| {
                    let screenshot = &screenshot;
                    scope.spawn(move || {
                        crate::screenshots::cached_image(7, screenshot, index % 2 == 0).unwrap()
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(paths.iter().all(|path| path == &paths[0]));
        assert_eq!(server.hits.load(std::sync::atomic::Ordering::Relaxed), 2);
        fs::write(&paths[0], b"poisoned cached response").unwrap();
        assert_eq!(
            crate::screenshots::cached_image(7, &screenshot, true).unwrap(),
            paths[0]
        );
        assert_eq!(server.hits.load(std::sync::atomic::Ordering::Relaxed), 3);
        assert!(validate_cover_file(&paths[0]).is_ok());
    }

    struct CoverServer {
        url: String,
        hits: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl CoverServer {
        fn start(
            response: impl Fn(&str) -> (u16, Vec<u8>, Duration) + Send + Sync + 'static,
        ) -> Self {
            use std::io::{Read, Write};
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            listener.set_nonblocking(true).unwrap();
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let shutdown = stop.clone();
            let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let requests = hits.clone();
            let response = std::sync::Arc::new(response);
            let thread = std::thread::spawn(move || {
                let mut handlers = Vec::new();
                while !shutdown.load(std::sync::atomic::Ordering::Acquire) {
                    match listener.accept() {
                        Ok((mut socket, _)) => {
                            requests.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let response = response.clone();
                            handlers.push(std::thread::spawn(move || {
                                socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                                let mut request=[0;4096];let count=socket.read(&mut request).unwrap();
                                let line=String::from_utf8_lossy(&request[..count]);let route=line.split_whitespace().nth(1).unwrap();
                                let (status,body,delay)=response(route);std::thread::sleep(delay);
                                let _=write!(socket,"HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len());let _=socket.write_all(&body);
                            }));
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2))
                        }
                        Err(error) => panic!("fixture accept failed: {error}"),
                    }
                }
                for handler in handlers {
                    handler.join().unwrap();
                }
            });
            Self {
                url,
                hits,
                stop,
                thread: Some(thread),
            }
        }
    }

    impl Drop for CoverServer {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::Release);
            self.thread.take().unwrap().join().unwrap();
        }
    }

    fn cover_png() -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(8, 4)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    fn cover_jobs(count: i64) -> Vec<ImageRequest> {
        (0..count)
            .map(|id| ImageRequest {
                product_id: id,
                kind: ImageKind::Cover,
                url: Some(id.to_string()),
                fallback: None,
                cached: None,
            })
            .collect()
    }

    #[test]
    fn sidebar_icons_and_covers_overlap_for_all_products_with_independent_failures_and_cache() {
        let directory = tempfile::tempdir().unwrap();
        let png = cover_png();
        let server = CoverServer::start(move |route| match route {
            "/cover/55" => (503, Vec::new(), Duration::ZERO),
            "/icon/57" => (503, Vec::new(), Duration::ZERO),
            "/icon/58" => (404, Vec::new(), Duration::ZERO),
            _ => (
                200,
                png.clone(),
                if route == "/icon/0" {
                    Duration::from_millis(120)
                } else {
                    Duration::ZERO
                },
            ),
        });
        let products=(0..125).map(|id|serde_json::from_value::<Product>(serde_json::json!({"id":id,"slug":format!("game-{id}"),"title":format!("Game {id}"),"images":{"background":format!("{}/cover/{id}",server.url),"sidebarIcon2x":format!("{}/icon/{id}",server.url)}})).unwrap()).collect::<Vec<_>>();
        let games = products.iter().map(normalize_product).collect::<Vec<_>>();
        let database = directory.path().join("state.sqlite3");
        let store = crate::state::StateStore::open_at(&database).unwrap();
        let ids = (0..125).collect::<Vec<_>>();
        store.cache_core_library(&games, &ids, &ids, &[]).unwrap();
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let active = std::sync::atomic::AtomicUsize::new(0);
        let maximum = std::sync::atomic::AtomicUsize::new(0);
        let mut starts = Vec::new();
        let mut outcomes = std::collections::HashMap::new();
        stream_image_work(
            image_requests(&games, &products),
            || true,
            |request| {
                let count = active.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                maximum.fetch_max(count, std::sync::atomic::Ordering::SeqCst);
                let path = directory
                    .path()
                    .join(format!("{:?}-{}.png", request.kind, request.product_id));
                let result =
                    cache_cover_at(&client, request.url.as_deref().unwrap(), &path).map(Some);
                active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                result
            },
            |work| {
                match work {
                    ImageWork::Started(id, kind) => starts.push((id, kind)),
                    ImageWork::Finished(id, kind, result) => {
                        let outcome = finish_image(result, |path| match kind {
                            ImageKind::Cover => store.cache_product_cover(id, path),
                            ImageKind::Icon => store.cache_product_icon(id, path),
                        });
                        assert!(outcomes.insert((id, kind), outcome).is_none());
                    }
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(starts.len(), 250);
        assert_eq!(outcomes.len(), 250);
        assert_eq!(server.hits.load(std::sync::atomic::Ordering::Relaxed), 250);
        assert!(
            starts[..4]
                .iter()
                .any(|(_, kind)| *kind == ImageKind::Cover)
        );
        assert!(starts[..4].iter().any(|(_, kind)| *kind == ImageKind::Icon));
        assert!(maximum.load(std::sync::atomic::Ordering::SeqCst) <= 4);
        assert!(matches!(
            outcomes[&(55, ImageKind::Cover)],
            CoverOutcome::Failed(_)
        ));
        assert!(matches!(
            outcomes[&(55, ImageKind::Icon)],
            CoverOutcome::Loaded(_)
        ));
        assert!(matches!(
            outcomes[&(57, ImageKind::Icon)],
            CoverOutcome::Failed(_)
        ));
        assert!(matches!(
            outcomes[&(57, ImageKind::Cover)],
            CoverOutcome::Loaded(_)
        ));
        assert_eq!(outcomes[&(58, ImageKind::Icon)], CoverOutcome::Unavailable);
        drop(store);
        let store = crate::state::StateStore::open_at(&database).unwrap();
        let cached = store.normalized_games().unwrap();
        assert_eq!(cached.len(), 125);
        assert!(
            cached
                .iter()
                .find(|game| game.product_id == 124)
                .unwrap()
                .icon
                .is_some()
        );
        assert!(
            cached
                .iter()
                .find(|game| game.product_id == 55)
                .unwrap()
                .artwork
                .is_none()
        );
        assert_eq!(
            cached.iter().filter(|game| game.icon.is_some()).count(),
            123
        );
        let hits = server.hits.load(std::sync::atomic::Ordering::Relaxed);
        for (id, kind) in [(124, ImageKind::Cover), (124, ImageKind::Icon)] {
            let path = directory.path().join(format!("{kind:?}-{id}.png"));
            let route = if kind == ImageKind::Cover {
                "cover"
            } else {
                "icon"
            };
            cache_cover_at(&client, &format!("{}/{route}/{id}", server.url), &path).unwrap();
        }
        assert_eq!(server.hits.load(std::sync::atomic::Ordering::Relaxed), hits);
        let game = cached.iter().find(|game| game.product_id == 124).unwrap();
        let missing = image_requests(std::slice::from_ref(game), &[]);
        assert!(missing[1].url.is_none());
        assert_eq!(missing[1].cached, game.icon);
        let mut current = game.clone();
        apply_core_product(
            &mut current,
            Game {
                product_id: 124,
                ..Default::default()
            },
        );
        assert_eq!(current.icon, game.icon);
        let mut committed = Game {
            product_id: 124,
            ..Default::default()
        };
        apply_core_product(&mut committed, game.clone());
        assert_eq!(committed.icon, game.icon);
    }

    #[test]
    fn retry_requests_only_failed_images_and_rejects_old_sessions_without_requests() {
        let failing = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let failures = failing.clone();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = requests.clone();
        let png = cover_png();
        let server = CoverServer::start(move |route| {
            seen.lock().unwrap().push(route.to_owned());
            let status = if route == "/icon/3" {
                404
            } else if failures.load(std::sync::atomic::Ordering::Acquire)
                && matches!(route, "/cover/1" | "/icon/2")
            {
                503
            } else {
                200
            };
            (status, png.clone(), Duration::ZERO)
        });
        let products=(1..=3).map(|id|serde_json::from_value::<Product>(serde_json::json!({"id":id,"slug":format!("game-{id}"),"title":format!("Game {id}"),"images":{"background":format!("{}/cover/{id}",server.url),"sidebarIcon2x":format!("{}/icon/{id}",server.url)}})).unwrap()).collect::<Vec<_>>();
        let mut games = products.iter().map(normalize_product).collect::<Vec<_>>();
        let store = crate::state::StateStore::open().unwrap();
        store
            .cache_core_library(&games, &[1, 2, 3], &[1, 2, 3], &[])
            .unwrap();
        let session = begin_library_session();
        assert!(!has_failed_images(session));
        let assets = image_requests(&games, &products);
        *FAILED_IMAGES.lock().unwrap() = ImageRetryState {
            session,
            requests: assets.clone(),
            running: true,
        };
        let guard = ImageBatchGuard(session);
        let (sender, receiver) = mpsc::channel();
        stream_images(
            assets,
            &reqwest::blocking::Client::builder()
                .no_proxy()
                .build()
                .unwrap(),
            &store,
            &sender,
            session,
            &mut games,
        )
        .unwrap();
        drop(guard);
        assert_eq!(requests.lock().unwrap().len(), 6);
        assert_eq!(FAILED_IMAGES.lock().unwrap().requests.len(), 2);
        assert!(has_failed_images(session));
        requests.lock().unwrap().clear();
        failing.store(false, std::sync::atomic::Ordering::Release);
        retry_failed_images(&sender, session).unwrap();
        let mut retried = requests.lock().unwrap().clone();
        retried.sort();
        assert_eq!(retried, ["/cover/1", "/icon/2"]);
        assert!(FAILED_IMAGES.lock().unwrap().requests.is_empty());
        assert!(!has_failed_images(session));
        assert!(retry_failed_images(&sender, session).is_err());
        let events = receiver.try_iter().map(Result::unwrap).collect::<Vec<_>>();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, SyncEvent::ImageRetryComplete))
        );
        assert!(events.iter().all(|event| !matches!(
            event,
            SyncEvent::Ownership(_)
                | SyncEvent::BasicBatch { .. }
                | SyncEvent::Catalog { .. }
                | SyncEvent::Complete { .. }
        )));
        begin_library_session();
        assert!(!has_failed_images(session));
        assert!(retry_failed_images(&sender, session).is_err());
        assert_eq!(requests.lock().unwrap().len(), 2);
    }

    #[test]
    fn more_than_two_core_batches_finish_all_covers_despite_slow_missing_and_failed_images() {
        let directory = tempfile::tempdir().unwrap();
        let png = cover_png();
        let server = CoverServer::start(move |route| {
            let id = route.trim_start_matches('/').parse::<i64>().unwrap();
            match id {
                51 => (404, Vec::new(), Duration::ZERO),
                52 => (503, Vec::new(), Duration::ZERO),
                53 => (200, b"<html>not an image</html>".to_vec(), Duration::ZERO),
                _ => (
                    200,
                    png.clone(),
                    if id == 0 {
                        Duration::from_millis(150)
                    } else {
                        Duration::ZERO
                    },
                ),
            }
        });
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let active = std::sync::atomic::AtomicUsize::new(0);
        let maximum = std::sync::atomic::AtomicUsize::new(0);
        let mut started = std::collections::HashSet::new();
        let mut finished = Vec::new();
        stream_image_work(
            cover_jobs(125),
            || true,
            |asset| {
                let current = active.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                maximum.fetch_max(current, std::sync::atomic::Ordering::SeqCst);
                let result = cache_cover_at(
                    &client,
                    &format!("{}/{}", server.url, asset.product_id),
                    &directory.path().join(format!("{}.png", asset.product_id)),
                )
                .map(Some);
                active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                result
            },
            |event| {
                match event {
                    ImageWork::Started(id, ImageKind::Cover) => {
                        assert!(started.insert(id));
                    }
                    ImageWork::Finished(id, ImageKind::Cover, result) => {
                        let outcome = finish_image(result, |_| {
                            if id == 54 {
                                anyhow::bail!("fixture database busy");
                            }
                            Ok(())
                        });
                        finished.push((id, outcome));
                    }
                    _ => panic!("unexpected sidebar job in cover fixture"),
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(started.len(), 125);
        assert_eq!(finished.len(), 125);
        assert_eq!(server.hits.load(std::sync::atomic::Ordering::Relaxed), 125);
        assert!(maximum.load(std::sync::atomic::Ordering::SeqCst) <= 4);
        assert_ne!(finished[0].0, 0);
        assert_eq!(
            finished
                .iter()
                .filter(|(_, outcome)| matches!(outcome, CoverOutcome::Loaded(_)))
                .count(),
            121
        );
        assert_eq!(
            finished.iter().find(|(id, _)| *id == 51).unwrap().1,
            CoverOutcome::Unavailable
        );
        for id in [52, 53, 54] {
            assert!(matches!(
                finished.iter().find(|(found, _)| *found == id).unwrap().1,
                CoverOutcome::Failed(_)
            ));
        }
        assert!(
            finished
                .iter()
                .any(|(id, outcome)| *id == 124 && matches!(outcome, CoverOutcome::Loaded(_)))
        );
    }

    #[test]
    fn corrupt_http_success_is_not_published_and_legacy_corrupt_cache_is_repaired() {
        let directory = tempfile::tempdir().unwrap();
        let png = cover_png();
        let server = CoverServer::start(move |route| {
            if route == "/broken" {
                (200, b"invalid-image".to_vec(), Duration::ZERO)
            } else {
                (200, png.clone(), Duration::ZERO)
            }
        });
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .build()
            .unwrap();
        let path = directory.path().join("cover.png");
        assert!(cache_cover_at(&client, &format!("{}/broken", server.url), &path).is_err());
        assert!(!path.exists());
        assert!(!path.with_extension("source.json").exists());
        let url = format!("{}/valid", server.url);
        fs::write(&path, b"previous HTTP200 HTML").unwrap();
        fs::write(
            path.with_extension("source.json"),
            serde_json::to_vec(&AssetManifest {
                source_url: url.clone(),
                etag: None,
                last_modified: None,
                usable: true,
                wordmark_processed: false,
                wordmark_processing_version: 0,
            })
            .unwrap(),
        )
        .unwrap();
        // The old cache predicate accepts this poisoned response forever.
        assert!(asset_is_current(&path, &url));
        assert!(validate_cover_file(&path).is_err());
        cache_cover_at(&client, &url, &path).unwrap();
        validate_cover_file(&path).unwrap();
        let hits = server.hits.load(std::sync::atomic::Ordering::Relaxed);
        cache_cover_at(&client, &url, &path).unwrap();
        assert_eq!(server.hits.load(std::sync::atomic::Ordering::Relaxed), hits);
    }

    #[test]
    fn cancelled_cover_session_and_disconnected_consumer_do_not_strand_bounded_workers() {
        let current = std::sync::atomic::AtomicBool::new(true);
        let mut settled = 0;
        stream_image_work(
            cover_jobs(125),
            || current.load(std::sync::atomic::Ordering::Acquire),
            |_| Ok(None),
            |event| {
                if matches!(event, ImageWork::Finished(..)) {
                    settled += 1;
                    current.store(false, std::sync::atomic::Ordering::Release);
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(settled, 1);
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = stream_image_work(
                cover_jobs(125),
                || true,
                |_| Ok(None),
                |_| anyhow::bail!("consumer closed"),
            );
            sender.send(result.is_err()).unwrap();
        });
        assert!(receiver.recv_timeout(Duration::from_secs(2)).unwrap());
    }

    #[test]
    fn cover_limits_and_safe_errors_never_publish_raw_urls() {
        assert!(validate_cover(&vec![0; COVER_MAX_BYTES as usize + 1]).is_err());
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(8193, 1)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        assert!(validate_cover(&bytes.into_inner()).is_err());
        let server = CoverServer::start(|_| (503, Vec::new(), Duration::ZERO));
        let error = reqwest::blocking::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("{}/cover?token=private", server.url))
            .send()
            .unwrap()
            .error_for_status()
            .unwrap_err();
        let message = sync_error_message(&error.into());
        assert!(message.contains("503"));
        assert!(!message.contains("private"));
        assert!(!message.contains("http"));
    }

    #[test]
    fn old_account_request_is_rejected_before_opening_cache_or_network() {
        // A queued request carries the session captured alongside its token.
        let obsolete = account_session().wrapping_sub(1);
        let error = fetch_product_section(
            &Game::default(),
            DetailSection::Acquisition,
            Some("fixture-token"),
            None,
            obsolete,
        )
        .unwrap_err();
        assert!(error.to_string().contains("Account changed before"));
        assert!(!error.to_string().contains("fixture-token"));
    }

    #[test]
    fn absent_pack_relationships_preserve_cached_entitlement_but_explicit_empty_revokes_it() {
        let directory = tempfile::tempdir().unwrap();
        let store = crate::state::StateStore::open_at(&directory.path().join("state.db")).unwrap();
        store
            .cache_core_library(&[], &[10], &[10, 20], &[(10, vec![20])])
            .unwrap();
        let sparse: Product = serde_json::from_value(
            serde_json::json!({"id":10,"title":"Pack","slug":"pack","game_type":"pack"}),
        )
        .unwrap();
        let sparse_entitlements = core_entitlements(&[sparse], &[10], &store).unwrap();
        assert!(sparse_entitlements.ids.contains(&20));
        assert!(sparse_entitlements.packs.is_empty());
        assert!(
            core_entitlements(&[], &[10], &store)
                .unwrap()
                .ids
                .contains(&20)
        );
        let empty: Product = serde_json::from_value(serde_json::json!({"id":10,"title":"Pack","slug":"pack","game_type":"pack","dlcs":{"products":[]}})).unwrap();
        let empty_entitlements = core_entitlements(&[empty], &[10], &store).unwrap();
        assert!(!empty_entitlements.ids.contains(&20));
        assert_eq!(empty_entitlements.packs, vec![(10, vec![])]);
    }

    #[test]
    fn sparse_core_retains_unobserved_platforms_and_languages() {
        let product: Product = serde_json::from_value(serde_json::json!({"id":1,"title":"New title","slug":"one","content_system_compatibility":{"windows":false}})).unwrap();
        let cached = Game {
            product_id: 1,
            platforms: Platforms {
                windows: true,
                linux: true,
                macos: false,
            },
            languages: vec!["French".into()],
            ..Default::default()
        };
        let merged = merge_core(&product, Some(&cached));
        assert!(!merged.platforms.windows);
        assert!(merged.platforms.linux);
        assert_eq!(merged.languages, ["French"]);
    }

    #[test]
    fn late_core_and_detail_results_preserve_newer_sections_and_ownership() {
        let mut current = Game {
            product_id: 1,
            description: "Loaded description".into(),
            dlcs: vec![Dlc {
                product_id: 2,
                owned: true,
                description: "DLC details".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let incoming = Game {
            product_id: 1,
            title: "Fresh title".into(),
            dlcs: vec![Dlc {
                product_id: 2,
                owned: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        apply_core_product(&mut current, incoming);
        assert_eq!(current.description, "Loaded description");
        assert_eq!(current.dlcs[0].description, "DLC details");
        let mut fetched = current.clone();
        fetched.dlcs[0].owned = false;
        fetched.dlcs.push(Dlc {
            product_id: 3,
            owned: true,
            ..Default::default()
        });
        fetched.description = "Updated".into();
        apply_product_section(&mut current, fetched, DetailSection::Product);
        assert!(current.dlcs[0].owned);
        assert!(!current.dlcs[1].owned);
        let metadata = Game {
            product_id: 1,
            description: "Stale".into(),
            ..Default::default()
        };
        apply_product_section(&mut current, metadata, DetailSection::Metadata);
        assert_eq!(current.description, "Updated");
    }

    fn temporary_png(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ludomere-{name}-{}-{}.png",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ))
    }

    #[test]
    fn trims_transparent_wordmark_canvas_and_rejects_opaque_cards() {
        let wordmark_path = temporary_png("wordmark-crop");
        let wordmark = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 100, 60).unwrap();
        wordmark.fill(0x00000000);
        wordmark.new_subpixbuf(30, 20, 40, 20).fill(0xffffffff);
        wordmark.savev(&wordmark_path, "png", &[]).unwrap();

        assert!(trim_transparent_wordmark(&wordmark_path).unwrap());
        let cropped = Pixbuf::from_file(&wordmark_path).unwrap();
        assert!(cropped.width() < 60);
        assert!(cropped.height() < 40);

        let card_path = temporary_png("wordmark-card");
        let card = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 100, 60).unwrap();
        card.fill(0xffffffff);
        card.savev(&card_path, "png", &[]).unwrap();
        assert!(!trim_transparent_wordmark(&card_path).unwrap());

        let _ = fs::remove_file(wordmark_path);
        let _ = fs::remove_file(card_path);
    }

    #[test]
    fn processed_wordmark_is_reused_without_a_network_request() {
        let path = temporary_png("wordmark-cache");
        let url = format!(
            "http://127.0.0.1:1/should-not-be-requested-{}.png",
            std::process::id()
        );
        let shared = shared_asset_path(&url, &path, true);
        fs::create_dir_all(shared.parent().unwrap()).unwrap();
        fs::write(&shared, cover_png()).unwrap();
        fs::write(
            shared.with_extension("source.json"),
            serde_json::to_vec(&AssetManifest {
                source_url: url.clone(),
                etag: None,
                last_modified: None,
                usable: true,
                wordmark_processed: true,
                wordmark_processing_version: WORDMARK_PROCESSING_VERSION,
            })
            .unwrap(),
        )
        .unwrap();

        let client = reqwest::blocking::Client::new();
        assert_eq!(
            cache_wordmark(&client, Some(&url), &path).unwrap(),
            Some(shared.clone())
        );

        let _ = fs::remove_file(&shared);
        let _ = fs::remove_file(shared.with_extension("source.json"));
        if let Some(parent) = shared.parent() {
            let _ = fs::remove_dir(parent);
        }
    }

    #[test]
    fn identical_product_assets_share_one_source_addressed_file() {
        let url = format!(
            "https://images.example/duplicate-{}.jpg",
            std::process::id()
        );
        let first = temporary_png("shared-first").with_extension("jpg");
        let second = temporary_png("shared-second").with_extension("jpg");
        let manifest = serde_json::to_vec(&AssetManifest {
            source_url: url.clone(),
            etag: None,
            last_modified: None,
            usable: true,
            wordmark_processed: false,
            wordmark_processing_version: 0,
        })
        .unwrap();
        let shared = shared_asset_path(&url, &first, false);
        fs::create_dir_all(shared.parent().unwrap()).unwrap();
        let image = cover_png();
        fs::write(&shared, &image).unwrap();
        fs::write(shared.with_extension("source.json"), &manifest).unwrap();
        let client = reqwest::blocking::Client::new();
        assert_eq!(
            cache_asset(&client, Some(&url), &first).unwrap(),
            Some(shared.clone())
        );
        assert_eq!(
            cache_asset(&client, Some(&url), &second).unwrap(),
            Some(shared.clone())
        );
        assert_eq!(fs::read(&shared).unwrap(), image);
        let _ = fs::remove_file(&shared);
        let _ = fs::remove_file(shared.with_extension("source.json"));
        if let Some(parent) = shared.parent() {
            let _ = fs::remove_dir(parent);
        }
    }

    #[test]
    fn detail_artwork_uses_semantic_provider_order() {
        let mut metadata = ProductMetadata {
            store_galaxy_background_url: Some("galaxy".into()),
            gamesdb_artwork_url: Some("artwork".into()),
            gamesdb_horizontal_artwork_url: Some("horizontal".into()),
            gamesdb_background_url: Some("background".into()),
            ..Default::default()
        };
        assert_eq!(
            detail_artwork_candidates(&metadata, vec!["product".into()]),
            ["galaxy", "artwork", "horizontal", "background", "product"]
        );
        metadata.store_galaxy_background_url = None;
        assert_eq!(
            detail_artwork_candidates(&metadata, vec!["product".into()]),
            ["artwork", "horizontal", "background", "product"]
        );
        metadata.gamesdb_artwork_url = None;
        assert_eq!(
            detail_artwork_candidates(&metadata, vec!["product".into()]),
            ["horizontal", "background", "product"]
        );
        metadata.gamesdb_horizontal_artwork_url = None;
        assert_eq!(
            detail_artwork_candidates(&metadata, vec!["product".into()]),
            ["background", "product"]
        );
        metadata.gamesdb_background_url = None;
        assert_eq!(
            detail_artwork_candidates(&metadata, vec!["product".into()]),
            ["product"]
        );
    }

    #[test]
    fn narrow_hero_is_extended_to_the_standard_ratio() {
        let source = temporary_png("narrow-hero");
        let image = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 160, 90).unwrap();
        image.fill(0xc05020ff);
        image.savev(&source, "png", &[]).unwrap();
        assert!(!hero_aspect_is_suitable(&source));
        let output = process_extended_hero(&source).unwrap();
        let processed = Pixbuf::from_file(&output).unwrap();
        assert_eq!((processed.width(), processed.height()), (2560, 670));
        assert!(hero_aspect_is_suitable(&output));
        let _ = fs::remove_file(source);
        let _ = fs::remove_file(&output);
        if let Some(parent) = output.parent() {
            let _ = fs::remove_dir(parent);
        }
    }

    #[test]
    fn parses_multipart_installer_names() {
        assert_eq!(
            multipart_numbers("Baldur's Gate 3 (Part 12 of 33)"),
            (Some(12), Some(33))
        );
        assert_eq!(multipart_numbers("Bloody Hell"), (None, None));
    }

    #[test]
    fn parses_gog_display_sizes() {
        assert_eq!(parse_size_label("2 MB"), Some(2_000_000));
        assert_eq!(parse_size_label("4 GB"), Some(4_000_000_000));
        assert_eq!(parse_size_label("1.5 GB"), Some(1_500_000_000));
    }

    #[test]
    fn gamesdb_observations_avoid_repeated_refresh_requests() {
        let now = 10_000_000;
        let available = ("available".to_owned(), now - 60);
        let missing = ("not_found".to_owned(), now - 60);
        assert!(!gamesdb_refresh_due(Some(&available), true, now, false));
        assert!(!gamesdb_refresh_due(Some(&missing), false, now, false));
        assert!(gamesdb_refresh_due(Some(&available), true, now, true));
        assert!(gamesdb_refresh_due(Some(&missing), false, now, true));
    }

    #[test]
    fn gamesdb_observations_expire_at_separate_intervals() {
        let now = 10_000_000;
        let stale_available = (
            "available".to_owned(),
            now - GAMESDB_AVAILABLE_REFRESH_SECONDS,
        );
        let stale_missing = (
            "not_found".to_owned(),
            now - GAMESDB_NOT_FOUND_REFRESH_SECONDS,
        );
        assert!(gamesdb_refresh_due(
            Some(&stale_available),
            true,
            now,
            false
        ));
        assert!(gamesdb_refresh_due(Some(&stale_missing), false, now, false));
        assert!(gamesdb_refresh_due(None, false, now, false));
    }

    #[test]
    fn catalog_expansion_does_not_grant_dlc_ownership() {
        let game = Game {
            product_id: 3,
            title: "Store-only DLC".into(),
            ..Default::default()
        };

        let dlc = game_into_dlc(game, false);

        assert!(!dlc.owned);
    }

    #[test]
    fn associates_nested_dlc_downloads_with_catalog_products() {
        let manifest = serde_json::json!({
            "dlcs": [{
                "title": "Example Game: First DLC™",
                "downloads": [["English", {"windows": [{
                    "manualUrl": "/downloads/example_dlc/en1installer0",
                    "name": "Example Game - First DLC",
                    "size": "2 MB"
                }]}]],
                "extras": [],
                "dlcs": [{
                    "title": "Example Game: Nested DLC",
                    "downloads": [["English", {"linux": [{
                        "manualUrl": "/downloads/nested_dlc/en1installer0",
                        "name": "Example Game - Nested DLC",
                        "size": "4 MB"
                    }]}]],
                    "extras": [],
                    "dlcs": []
                }]
            }]
        });
        let known = vec![
            (101, "Example Game: First DLC".to_owned()),
            (102, "Example Game: Nested DLC".to_owned()),
        ];

        let results = normalize_dlc_download_artifacts(&manifest, &known);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, 101);
        assert_eq!(results[0].1[0].operating_system.as_deref(), Some("windows"));
        assert_eq!(results[1].0, 102);
        assert_eq!(results[1].1[0].operating_system.as_deref(), Some("linux"));
    }
}

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

struct AssetRequest {
    product_id: i64,
    artwork_url: Option<String>,
    artwork_fallback_url: Option<String>,
    detail_artwork_urls: Vec<String>,
    hero_logo_url: Option<String>,
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
    downloads: Option<serde_json::Value>,
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

pub fn stream_owned_games(
    product_ids: &[i64],
    access_token: &str,
    sender: &mpsc::Sender<Result<SyncEvent>>,
    force_gamesdb_refresh: bool,
) -> Result<()> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(45))
        .user_agent(crate::identity::USER_AGENT)
        .build()?;
    let mut products = Vec::with_capacity(product_ids.len());
    for (batch_index, ids) in product_ids.chunks(10).enumerate() {
        let joined = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        let response = client
            .get("https://api.gog.com/products")
            .query(&[
                ("ids", joined.as_str()),
                ("expand", crate::gog::product::EXPANSIONS),
            ])
            .send()?
            .error_for_status()?
            .text()?;
        let values: Vec<serde_json::Value> = serde_json::from_str(&response)?;
        let mut basic_batch = Vec::new();
        for value in values {
            let id = value.get("id").and_then(serde_json::Value::as_i64);
            let product = serde_json::from_value::<Product>(value.clone())
                .with_context(|| format!("parsing GOG product {id:?}"))?;
            if product.game_type == "game" && product.is_installable {
                basic_batch.push(normalize_product(&product));
            }
            products.push(product);
        }
        sender.send(Ok(SyncEvent::BasicBatch {
            games: basic_batch,
            current: ((batch_index + 1) * 10).min(product_ids.len()),
            total: product_ids.len(),
        }))?;
    }

    // Some purchases grant DLC through a pack without adding the child DLC IDs to
    // /user/data/games. Expand those pack relationships so the normalized library
    // reflects the complete entitlement rather than only directly owned products.
    let fetched_ids = products
        .iter()
        .map(|product| product.id)
        .collect::<std::collections::HashSet<_>>();
    let mut inherited_dlc_ids = products
        .iter()
        .filter(|product| product.game_type == "pack")
        .flat_map(|product| {
            product
                .dlcs
                .as_ref()
                .into_iter()
                .flat_map(|dlcs| dlcs.products.iter().map(|dlc| dlc.id))
        })
        .filter(|id| !fetched_ids.contains(id))
        .collect::<Vec<_>>();
    inherited_dlc_ids.sort_unstable();
    inherited_dlc_ids.dedup();
    let entitled_product_ids = product_ids
        .iter()
        .copied()
        .chain(inherited_dlc_ids.iter().copied())
        .collect::<std::collections::HashSet<_>>();
    let expanded_total = product_ids.len() + inherited_dlc_ids.len();
    for (batch_index, ids) in inherited_dlc_ids.chunks(10).enumerate() {
        let joined = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        let response = client
            .get("https://api.gog.com/products")
            .query(&[
                ("ids", joined.as_str()),
                ("expand", crate::gog::product::EXPANSIONS),
            ])
            .send()?
            .error_for_status()?
            .text()?;
        let values: Vec<serde_json::Value> = serde_json::from_str(&response)?;
        for value in values {
            let id = value.get("id").and_then(serde_json::Value::as_i64);
            products.push(
                serde_json::from_value::<Product>(value.clone())
                    .with_context(|| format!("parsing inherited GOG DLC {id:?}"))?,
            );
        }
        sender.send(Ok(SyncEvent::BasicBatch {
            games: Vec::new(),
            current: (product_ids.len() + (batch_index + 1) * 10).min(expanded_total),
            total: expanded_total,
        }))?;
    }
    let fetched_ids = products
        .iter()
        .map(|product| product.id)
        .collect::<std::collections::HashSet<_>>();
    let unresolved_ids = inherited_dlc_ids
        .into_iter()
        .filter(|id| !fetched_ids.contains(id))
        .collect::<std::collections::HashSet<_>>();
    products.extend(fetch_account_dlc_fallbacks(
        &client,
        access_token,
        &products,
        &unresolved_ids,
    )?);

    let known_ids = products
        .iter()
        .map(|product| product.id)
        .collect::<std::collections::HashSet<_>>();
    let expanded_products = products
        .iter()
        .flat_map(|product| product.expanded_dlcs.clone().unwrap_or_default())
        .filter(|product| !known_ids.contains(&product.id))
        .collect::<Vec<_>>();
    products.extend(expanded_products);

    let mut parent_dlcs = std::collections::HashMap::<i64, Vec<i64>>::new();
    let mut product_types = std::collections::HashMap::<i64, String>::new();
    let mut installable_products = std::collections::HashSet::<i64>::new();
    for product in &products {
        product_types.insert(product.id, product.game_type.clone());
        if product.is_installable {
            installable_products.insert(product.id);
        }
        if product.game_type == "game" {
            parent_dlcs.insert(
                product.id,
                product
                    .dlcs
                    .as_ref()
                    .map(|dlcs| dlcs.products.iter().map(|dlc| dlc.id).collect())
                    .unwrap_or_default(),
            );
        }
    }

    let mut assets = products.iter().map(asset_request).collect::<Vec<_>>();
    let mut normalized = std::collections::HashMap::with_capacity(products.len());
    for product in &products {
        let id = product.id;
        normalized.insert(id, normalize_product(product));
    }

    let mut attached_dlc_ids = std::collections::HashSet::new();
    for (parent_id, dlc_ids) in parent_dlcs {
        let dlcs = dlc_ids
            .into_iter()
            .filter(|id| product_types.get(id).is_some_and(|kind| kind == "dlc"))
            .filter_map(|id| {
                let game = normalized.get(&id)?.clone();
                attached_dlc_ids.insert(id);
                Some(game_into_dlc(game, entitled_product_ids.contains(&id)))
            })
            .collect::<Vec<_>>();
        if let Some(parent) = normalized.get_mut(&parent_id) {
            parent.dlc_count = dlcs.len();
            parent.dlcs = dlcs;
        }
    }

    let mut games = normalized
        .into_iter()
        .filter_map(|(id, game)| {
            let is_playable_game = product_types.get(&id).is_some_and(|kind| kind == "game")
                && installable_products.contains(&id);
            (is_playable_game && !attached_dlc_ids.contains(&id)).then_some(game)
        })
        .collect::<Vec<_>>();
    games.sort_by_key(|game| game.title.to_lowercase());
    let relevant_ids = games
        .iter()
        .flat_map(|game| {
            std::iter::once(game.product_id).chain(game.dlcs.iter().map(|dlc| dlc.product_id))
        })
        .collect::<std::collections::HashSet<_>>();
    assets.retain(|asset| relevant_ids.contains(&asset.product_id));
    for asset in &assets {
        let artwork = existing_asset_with_fallback(
            asset.artwork_url.as_deref(),
            asset.artwork_fallback_url.as_deref(),
            std::path::Path::new("tile.jpg"),
        );
        let detail_artwork = existing_detail_artwork(
            &asset.detail_artwork_urls,
            std::path::Path::new("detail.png"),
        );
        let hero_logo = existing_wordmark(
            asset.hero_logo_url.as_deref(),
            std::path::Path::new("hero-logo.png"),
        );
        let icon = existing_asset(asset.icon_url.as_deref(), std::path::Path::new("icon.png"));
        apply_media(
            &mut games,
            asset.product_id,
            artwork,
            detail_artwork,
            hero_logo,
            icon,
        );
    }
    sender.send(Ok(SyncEvent::Catalog {
        games: games.clone(),
    }))?;

    let structured_manifests = products
        .iter()
        .filter(|product| relevant_ids.contains(&product.id))
        .filter(|product| entitled_product_ids.contains(&product.id))
        .filter_map(|product| {
            if product.downloads.is_some() {
                let wrapper = serde_json::json!({ "downloads": product.downloads });
                return Some((
                    product.id,
                    crate::gog::product::download_artifacts(product.id, &wrapper),
                ));
            }
            match fetch_download_manifest(&client, access_token, product.id, &[]) {
                Ok((artifacts, _, _)) => {
                    tracing::debug!(product_id = product.id, "structured Product API omitted downloads; using compatibility provider");
                    Some((product.id, artifacts))
                }
                Err(error) => {
                    tracing::warn!(product_id = product.id, %error, "structured downloads unavailable; preserving cached manifest");
                    None
                }
            }
        })
        .collect::<Vec<_>>();
    let manifest_total = structured_manifests.len();
    for (index, (product_id, artifacts)) in structured_manifests.into_iter().enumerate() {
        apply_remote_artifacts(&mut games, product_id, artifacts.clone());
        sender.send(Ok(SyncEvent::FileMetadata {
            product_id,
            artifacts,
            current: index + 1,
            total: manifest_total,
        }))?;
    }

    let enrichment_store = crate::state::StateStore::open()?;
    let now = chrono::Utc::now().timestamp();
    let enrichment_ids = products
        .iter()
        .filter(|product| relevant_ids.contains(&product.id))
        .map(|product| {
            let compatibility = product.content_system_compatibility.as_ref();
            let observation = enrichment_store
                .enrichment_observation(product.id, "gamesdb")
                .unwrap_or_else(|error| {
                    tracing::warn!(product_id = product.id, %error, "could not read GamesDB cache state");
                    None
                });
            let mut cached_gamesdb = enrichment_store
                .cached_product_metadata(product.id)
                .unwrap_or_else(|error| {
                    tracing::warn!(product_id = product.id, %error, "could not read cached product metadata");
                    None
                });
            let gamesdb_due = gamesdb_refresh_due(
                observation.as_ref(),
                cached_gamesdb
                    .as_ref()
                    .is_some_and(|metadata| {
                        metadata.gamesdb_media_checked && metadata.gamesdb_media_version >= 2
                    }),
                now,
                force_gamesdb_refresh,
            );
            let cached_wordmark = cached_gamesdb.as_ref().and_then(|metadata| {
                (!gamesdb_due && metadata.store_wordmark_checked)
                    .then(|| metadata.store_wordmark_url.clone())
            });
            if observation
                .as_ref()
                .is_some_and(|(status, _)| status == "not_found")
            {
                cached_gamesdb = None;
            }
            (
                product.id,
                product.slug.clone(),
                compatibility
                    .and_then(|value| value.windows)
                    .unwrap_or(false),
                compatibility.and_then(|value| value.osx).unwrap_or(false),
                gamesdb_due,
                cached_gamesdb,
                cached_wordmark,
            )
        })
        .collect::<Vec<_>>();
    let enrichment_total = enrichment_ids.len();
    let jobs = std::sync::Arc::new(std::sync::Mutex::new(
        enrichment_ids
            .into_iter()
            .collect::<std::collections::VecDeque<_>>(),
    ));
    let (enrichment_sender, enrichment_receiver) = mpsc::channel();
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let jobs = jobs.clone();
            let client = client.clone();
            let enrichment_sender = enrichment_sender.clone();
            scope.spawn(move || {
                loop {
                    let job = jobs.lock().ok().and_then(|mut jobs| jobs.pop_front());
                    let Some((
                        product_id,
                        product_slug,
                        supports_windows,
                        supports_macos,
                        fetch_gamesdb,
                        cached_gamesdb,
                        cached_wordmark,
                    )) = job
                    else {
                        break;
                    };
                    let mut store = crate::gog::store::fetch(&client, product_id);
                    if let Ok(metadata) = &mut store {
                        match cached_wordmark {
                            Some(url) => {
                                metadata.store_wordmark_url = url;
                                metadata.store_wordmark_checked = true;
                            }
                            None => match crate::gog::store::fetch_wordmark(&client, &product_slug)
                            {
                                Ok(url) => {
                                    metadata.store_wordmark_url = url;
                                    metadata.store_wordmark_checked = true;
                                }
                                Err(error) => tracing::warn!(
                                    product_id,
                                    %error,
                                    "could not inspect GOG store page for a hero wordmark"
                                ),
                            },
                        }
                    }
                    let gamesdb =
                        fetch_gamesdb.then(|| crate::gog::gamesdb::fetch(&client, product_id));
                    let windows = supports_windows
                        .then(|| crate::gog::builds::fetch(&client, product_id, "windows"));
                    let macos = supports_macos
                        .then(|| crate::gog::builds::fetch(&client, product_id, "osx"));
                    if enrichment_sender
                        .send((product_id, store, gamesdb, cached_gamesdb, windows, macos))
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
        drop(enrichment_sender);
        for current in 1..=enrichment_total {
            let Ok((product_id, store, gamesdb, cached_gamesdb, windows, macos)) =
                enrichment_receiver.recv()
            else {
                break;
            };
            let mut metadata = match store {
                Ok(metadata) => metadata,
                Err(error) if is_http_not_found(&error) => {
                    tracing::debug!(product_id, "product has no Store API v2 metadata");
                    ProductMetadata::default()
                }
                Err(error) => {
                    tracing::warn!(product_id, %error, "could not fetch Store API metadata");
                    ProductMetadata::default()
                }
            };
            match gamesdb {
                Some(Ok(Some(gamesdb))) => {
                    merge_metadata(&mut metadata, gamesdb);
                    if let Err(error) = enrichment_store.record_enrichment_observation(
                        product_id,
                        "gamesdb",
                        "available",
                    ) {
                        tracing::warn!(product_id, %error, "could not cache GamesDB availability");
                    }
                }
                Some(Ok(None)) => {
                    tracing::debug!(product_id, "product has no GamesDB metadata mapping");
                    if let Err(error) = enrichment_store.record_enrichment_observation(
                        product_id,
                        "gamesdb",
                        "not_found",
                    ) {
                        tracing::warn!(product_id, %error, "could not cache missing GamesDB mapping");
                    }
                }
                Some(Err(error)) => {
                    if let Some(cached) = cached_gamesdb {
                        merge_metadata(&mut metadata, cached);
                    }
                    tracing::warn!(product_id, %error, "could not fetch GamesDB metadata");
                }
                None => {
                    if let Some(cached) = cached_gamesdb {
                        merge_metadata(&mut metadata, cached);
                    }
                }
            }
            apply_metadata(&mut games, product_id, metadata.clone());
            if let Some(asset) = assets
                .iter_mut()
                .find(|asset| asset.product_id == product_id)
            {
                asset.detail_artwork_urls = detail_artwork_candidates(
                    &metadata,
                    std::mem::take(&mut asset.detail_artwork_urls),
                );
                asset.hero_logo_url = metadata.store_wordmark_url.clone();
            }
            let _ = sender.send(Ok(SyncEvent::Enrichment {
                product_id,
                metadata: Box::new(metadata),
                current,
                total: enrichment_total,
            }));
            let windows_observed = matches!(&windows, Some(Ok(_)));
            let macos_observed = matches!(&macos, Some(Ok(_)));
            let mut builds = match windows {
                Some(Ok(values)) => values,
                Some(Err(error)) => {
                    tracing::warn!(product_id, %error, "could not fetch Windows Galaxy builds");
                    Vec::new()
                }
                None => Vec::new(),
            };
            match macos {
                Some(Ok(mut values)) => builds.append(&mut values),
                Some(Err(error)) => {
                    tracing::warn!(product_id, %error, "could not fetch macOS Galaxy builds")
                }
                None => {}
            }
            apply_builds(&mut games, product_id, builds.clone());
            let _ = sender.send(Ok(SyncEvent::Builds {
                product_id,
                builds,
                windows_observed,
                macos_observed,
                current,
                total: enrichment_total,
            }));
        }
    });

    let media_total = assets.len();
    for (media_index, asset) in assets.into_iter().enumerate() {
        let artwork = cache_asset_with_fallback(
            &client,
            asset.artwork_url.as_deref(),
            asset.artwork_fallback_url.as_deref(),
            std::path::Path::new("tile.jpg"),
        )
        .unwrap_or_else(|error| {
            if is_http_not_found(&error) {
                tracing::debug!(product_id = asset.product_id, "product artwork is unavailable");
            } else {
                tracing::warn!(product_id = asset.product_id, %error, "could not cache product artwork");
            }
            None
        });
        apply_media(
            &mut games,
            asset.product_id,
            artwork.clone(),
            None,
            None,
            None,
        );
        sender.send(Ok(SyncEvent::Media {
            product_id: asset.product_id,
            artwork: artwork.clone(),
            detail_artwork: None,
            hero_logo: None,
            icon: None,
            current: media_index * 4 + 1,
            total: media_total * 4,
        }))?;
        let detail_artwork = cache_detail_artwork(
            &client,
            &asset.detail_artwork_urls,
            std::path::Path::new("detail.png"),
        )
        .unwrap_or_else(|error| {
            if is_http_not_found(&error) {
                tracing::debug!(product_id = asset.product_id, "detail artwork is unavailable");
            } else {
                tracing::warn!(product_id = asset.product_id, %error, "could not cache detail artwork");
            }
            None
        });
        apply_media(
            &mut games,
            asset.product_id,
            None,
            detail_artwork.clone(),
            None,
            None,
        );
        sender.send(Ok(SyncEvent::Media {
            product_id: asset.product_id,
            artwork: None,
            detail_artwork: detail_artwork.clone(),
            hero_logo: None,
            icon: None,
            current: media_index * 4 + 2,
            total: media_total * 4,
        }))?;
        let hero_logo = cache_wordmark(
            &client,
            asset.hero_logo_url.as_deref(),
            std::path::Path::new("hero-logo.png"),
        )
        .unwrap_or_else(|error| {
            tracing::warn!(product_id = asset.product_id, %error, "could not cache GamesDB hero logo");
            None
        });
        apply_media(
            &mut games,
            asset.product_id,
            None,
            None,
            hero_logo.clone(),
            None,
        );
        sender.send(Ok(SyncEvent::Media {
            product_id: asset.product_id,
            artwork: None,
            detail_artwork: None,
            hero_logo: hero_logo.clone(),
            icon: None,
            current: media_index * 4 + 3,
            total: media_total * 4,
        }))?;
        let icon = cache_asset(
            &client,
            asset.icon_url.as_deref(),
            std::path::Path::new("icon.png"),
        )
        .unwrap_or_else(|error| {
            tracing::warn!(product_id = asset.product_id, %error, "could not cache product icon");
            None
        });
        apply_media(&mut games, asset.product_id, None, None, None, icon.clone());
        sender.send(Ok(SyncEvent::Media {
            product_id: asset.product_id,
            artwork: None,
            detail_artwork: None,
            hero_logo: None,
            icon,
            current: media_index * 4 + 4,
            total: media_total * 4,
        }))?;
    }
    for game in &mut games {
        game.dlcs.retain(Dlc::is_catalog_visible);
        game.dlc_count = game.dlcs.len();
    }
    sender.send(Ok(SyncEvent::Complete { games }))?;
    Ok(())
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

fn apply_metadata(games: &mut [Game], product_id: i64, metadata: ProductMetadata) {
    for game in games {
        if game.product_id == product_id {
            game.features = metadata
                .features
                .iter()
                .map(|term| term.name.clone())
                .collect();
            game.languages = metadata
                .localizations
                .iter()
                .map(|item| item.name.clone())
                .collect();
            game.metadata = metadata;
            return;
        }
        if let Some(dlc) = game
            .dlcs
            .iter_mut()
            .find(|dlc| dlc.product_id == product_id)
        {
            if dlc.description.trim().is_empty()
                && let Some(description) = metadata.store_description.as_ref()
            {
                dlc.description = description.clone();
            }
            dlc.languages = metadata
                .localizations
                .iter()
                .map(|item| item.name.clone())
                .collect();
            dlc.metadata = metadata;
            return;
        }
    }
}

fn apply_builds(games: &mut [Game], product_id: i64, builds: Vec<GalaxyBuild>) {
    for game in games {
        if game.product_id == product_id {
            game.galaxy_builds = builds;
            return;
        }
        if let Some(dlc) = game
            .dlcs
            .iter_mut()
            .find(|dlc| dlc.product_id == product_id)
        {
            dlc.galaxy_builds = builds;
            return;
        }
    }
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
        .error_for_status()?
        .text()?;
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
        .user_agent(crate::identity::USER_AGENT)
        .build()?;
    if let Ok(product) = crate::gog::product::fetch(&client, product_id) {
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
                downloads: None,
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
        metadata: Default::default(),
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
        product_id: product.id,
        artwork_url: tile_url,
        artwork_fallback_url,
        detail_artwork_urls,
        hero_logo_url: None,
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
        if asset_is_current(path, &normalized) {
            return Ok(Some(path.to_owned()));
        }
    }
    match cache_asset(client, primary_url, path) {
        Ok(asset) => Ok(asset),
        Err(primary_error) if fallback_url.is_some() => cache_asset(client, fallback_url, path)
            .with_context(|| {
                format!("primary artwork failed ({primary_error}); fallback artwork also failed")
            }),
        Err(error) => Err(error),
    }
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

fn existing_detail_artwork(urls: &[String], suggested_path: &std::path::Path) -> Option<PathBuf> {
    let mut first = None;
    for url in urls {
        let Some(path) = existing_asset(Some(url), suggested_path) else {
            continue;
        };
        first.get_or_insert_with(|| path.clone());
        if hero_aspect_is_suitable(&path) {
            return Some(path);
        }
    }
    first.and_then(|path| process_extended_hero(&path).ok())
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
    if asset_is_current(&path, &url) {
        return Ok(Some(path));
    }
    fs::create_dir_all(path.parent().context("shared media path has no parent")?)?;
    let response = client
        .get(&url)
        .send()
        .with_context(|| format!("downloading {url}"))?
        .error_for_status()?;
    let manifest = AssetManifest {
        source_url: url,
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
    let bytes = response.bytes()?;
    let temporary = path.with_extension("part");
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, &path)?;
    let manifest_path = path.with_extension("source.json");
    let manifest_temporary = path.with_extension("source.part");
    fs::write(&manifest_temporary, serde_json::to_vec(&manifest)?)?;
    fs::rename(manifest_temporary, manifest_path)?;
    Ok(Some(path))
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
        {
            return Ok(Some(path.to_owned()));
        }
    }

    if cached_manifest.is_none() || !path.is_file() {
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

fn existing_asset(url: Option<&str>, path: &std::path::Path) -> Option<PathBuf> {
    let url = normalize_asset_url(url?);
    let shared = shared_asset_path(&url, path, false);
    asset_is_current(&shared, &url).then_some(shared)
}

fn existing_wordmark(url: Option<&str>, path: &std::path::Path) -> Option<PathBuf> {
    let url = normalize_asset_url(url?);
    let shared = shared_asset_path(&url, path, true);
    let current = asset_is_current(&shared, &url)
        && fs::read(shared.with_extension("source.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<AssetManifest>(&bytes).ok())
            .is_some_and(|manifest| {
                manifest.wordmark_processed
                    && manifest.wordmark_processing_version == WORDMARK_PROCESSING_VERSION
            });
    current.then_some(shared)
}

fn existing_asset_with_fallback(
    primary_url: Option<&str>,
    fallback_url: Option<&str>,
    path: &std::path::Path,
) -> Option<PathBuf> {
    [primary_url, fallback_url]
        .into_iter()
        .flatten()
        .find_map(|url| existing_asset(Some(url), path))
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
    fs::write(&temporary, response.bytes()?)?;
    fs::rename(temporary, path)?;
    let manifest_path = path.with_extension("source.json");
    let manifest_temporary = path.with_extension("source.part");
    fs::write(&manifest_temporary, serde_json::to_vec(&manifest)?)?;
    fs::rename(manifest_temporary, manifest_path)?;
    Ok(())
}

fn normalize_asset_url(url: &str) -> String {
    if url.starts_with("//") {
        format!("https:{url}")
    } else {
        url.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        fs::write(&shared, b"cached-wordmark").unwrap();
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
        fs::write(&shared, b"same-image").unwrap();
        fs::write(shared.with_extension("source.json"), &manifest).unwrap();
        assert_eq!(existing_asset(Some(&url), &first), Some(shared.clone()));
        assert_eq!(existing_asset(Some(&url), &second), Some(shared.clone()));
        assert_eq!(fs::read(&shared).unwrap(), b"same-image");
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

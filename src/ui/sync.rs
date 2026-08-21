use super::*;

enum SyncPersistence {
    Catalog(Vec<Game>),
    Manifest(i64, Vec<RemoteArtifact>),
    Builds {
        product_id: i64,
        builds: Vec<crate::domain::GalaxyBuild>,
        windows_observed: bool,
        macos_observed: bool,
    },
}

fn start_sync_persistence_worker() -> mpsc::Sender<SyncPersistence> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let Ok(store) = StateStore::open() else {
            return;
        };
        while let Ok(task) = receiver.recv() {
            match task {
                SyncPersistence::Catalog(games) => {
                    if let Err(error) = store.cache_online_games(&games) {
                        tracing::warn!(%error, "could not cache online catalog");
                    }
                    if let Err(error) = store.upsert_normalized_library(&games) {
                        tracing::warn!(%error, "could not persist normalized product catalog");
                    }
                }
                SyncPersistence::Manifest(product_id, artifacts) => {
                    if let Err(error) = store.observe_download_manifest(product_id, &artifacts) {
                        tracing::warn!(product_id, %error, "could not observe structured manifest");
                    }
                    if let Err(error) = store.cache_download_manifest(product_id, &artifacts) {
                        tracing::warn!(product_id, %error, "could not update compatibility manifest cache");
                    }
                }
                SyncPersistence::Builds {
                    product_id,
                    builds,
                    windows_observed,
                    macos_observed,
                } => {
                    for (os, observed) in [("windows", windows_observed), ("osx", macos_observed)] {
                        if !observed {
                            continue;
                        }
                        let values = builds
                            .iter()
                            .filter(|build| build.operating_system == os)
                            .cloned()
                            .collect::<Vec<_>>();
                        if let Err(error) = store.observe_galaxy_builds(product_id, os, &values) {
                            tracing::warn!(product_id, os, %error, "could not persist Galaxy builds");
                        }
                    }
                }
            }
        }
    });
    sender
}

pub(super) fn update_streamed_media(
    w: &Widgets,
    model: &Rc<RefCell<AppModel>>,
    product_id: i64,
    mut artwork: Option<std::path::PathBuf>,
    mut detail_artwork: Option<std::path::PathBuf>,
    hero_logo: Option<std::path::PathBuf>,
    icon: Option<std::path::PathBuf>,
) {
    artwork =
        crate::custom_artwork::override_path(product_id, crate::custom_artwork::ArtworkKind::Cover)
            .or(artwork);
    detail_artwork = crate::custom_artwork::override_path(
        product_id,
        crate::custom_artwork::ArtworkKind::Background,
    )
    .or(detail_artwork);
    let selected_product = model.borrow().selected;
    let mut state = model.borrow_mut();
    let Some(game) = state
        .games
        .iter_mut()
        .find(|game| game.product_id == product_id)
    else {
        for game in &mut state.games {
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
                if let Some(path) = detail_artwork
                    .clone()
                    .or(parent_detail_artwork.filter(|_| dlc.detail_artwork.is_none()))
                {
                    dlc.detail_artwork = Some(path);
                }
                if let Some(path) = hero_logo
                    .clone()
                    .or(parent_hero_logo.filter(|_| dlc.hero_logo.is_none()))
                {
                    dlc.hero_logo = Some(path);
                }
                if let Some(path) = icon {
                    dlc.icon = Some(path);
                }
                drop(state);
                if selected_product == Some(product_id)
                    && let Some(path) = detail_artwork
                    && let Some(picture) = find_named_descendant(
                        w.details.upcast_ref::<gtk::Widget>(),
                        "detail-hero-image",
                    )
                    .and_downcast::<gtk::Picture>()
                {
                    picture.set_file(Some(&gio::File::for_path(path)));
                }
                return;
            }
        }
        return;
    };
    if let Some(path) = &artwork {
        game.artwork = Some(path.clone());
    }
    if let Some(path) = &detail_artwork {
        game.detail_artwork = Some(path.clone());
    }
    if let Some(path) = &hero_logo {
        game.hero_logo = Some(path.clone());
    }
    if let Some(path) = &icon {
        game.icon = Some(path.clone());
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
    let card_width = state.card_width;
    drop(state);

    if selected_product == Some(product_id)
        && let Some(picture) =
            find_named_descendant(w.details.upcast_ref::<gtk::Widget>(), "detail-hero-image")
                .and_downcast::<gtk::Picture>()
        && let Some(path) = &detail_artwork
    {
        picture.set_file(Some(&gio::File::for_path(path)));
    }
    if selected_product == Some(product_id) {
        let root = w.details.upcast_ref::<gtk::Widget>();
        if let Some(picture) =
            find_named_descendant(root, "detail-hero-logo").and_downcast::<gtk::Picture>()
            && let Some(path) = &hero_logo
        {
            picture.set_file(Some(&gio::File::for_path(path)));
            picture.set_visible(true);
        }
        if hero_logo.is_some()
            && let Some(title) = find_named_descendant(root, "detail-hero-text-title")
        {
            title.set_visible(false);
        }
    }

    let id_text = product_id.to_string();
    let mut row = w.game_list.first_child();
    while let Some(widget) = row {
        if widget.widget_name() == id_text {
            if let Some(picture) =
                find_named_descendant(&widget, "game-icon").and_downcast::<gtk::Picture>()
                && let Some(path) = &icon
                && let Some(texture) = scaled_card_texture(path, 23, 23)
            {
                picture.set_size_request(23, 23);
                picture.set_paintable(Some(&texture));
            }
            break;
        }
        row = widget.next_sibling();
    }

    let mut child = w.home_grid.first_child();
    while let Some(wrapper) = child {
        if let Some(card) = wrapper.first_child()
            && card.widget_name() == id_text
        {
            if let Some(picture) =
                find_named_descendant(&card, "card-art").and_downcast::<gtk::Picture>()
                && let Some(path) = &artwork
                && let Some(texture) = scaled_card_texture(path, card_width, card_width * 9 / 16)
            {
                picture.set_size_request(card_width, card_width * 9 / 16);
                picture.set_paintable(Some(&texture));
            }
            break;
        }
        child = wrapper.next_sibling();
    }
}

pub(super) fn apply_remote_artifacts_to_model(
    games: &mut [Game],
    product_id: i64,
    artifacts: Vec<crate::domain::RemoteArtifact>,
) {
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

pub(super) fn apply_metadata_to_model(
    games: &mut [Game],
    product_id: i64,
    metadata: crate::domain::ProductMetadata,
) {
    for game in games {
        if game.product_id == product_id {
            merge_product_metadata(&mut game.metadata, metadata);
            game.features = game
                .metadata
                .features
                .iter()
                .map(|term| term.name.clone())
                .collect();
            game.languages = game
                .metadata
                .localizations
                .iter()
                .map(|item| item.name.clone())
                .collect();
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
            merge_product_metadata(&mut dlc.metadata, metadata);
            dlc.languages = dlc
                .metadata
                .localizations
                .iter()
                .map(|item| item.name.clone())
                .collect();
            return;
        }
    }
}

pub(super) fn merge_product_metadata(
    current: &mut crate::domain::ProductMetadata,
    update: crate::domain::ProductMetadata,
) {
    if !update.tags.is_empty() {
        current.tags = update.tags;
    }
    if !update.properties.is_empty() {
        current.properties = update.properties;
    }
    if !update.features.is_empty() {
        current.features = update.features;
    }
    if !update.genres.is_empty() {
        current.genres = update.genres;
    }
    if !update.themes.is_empty() {
        current.themes = update.themes;
    }
    if !update.game_modes.is_empty() {
        current.game_modes = update.game_modes;
    }
    if !update.localizations.is_empty() {
        current.localizations = update.localizations;
    }
    if !update.developers.is_empty() {
        current.developers = update.developers;
    }
    if !update.publishers.is_empty() {
        current.publishers = update.publishers;
    }
    if update.series.is_some() {
        current.series = update.series;
    }
    if !update.editions.is_empty() {
        current.editions = update.editions;
    }
    if !update.system_requirements.is_empty() {
        current.system_requirements = update.system_requirements;
    }
    if update.copyright.is_some() {
        current.copyright = update.copyright;
    }
    if update.gamesdb_summary.is_some() {
        current.gamesdb_summary = update.gamesdb_summary;
    }
    if update.store_release_status.is_some() {
        current.store_release_status = update.store_release_status;
    }
    if update.store_description.is_some() {
        current.store_description = update.store_description;
    }
}

pub(super) fn apply_builds_to_model(
    games: &mut [Game],
    product_id: i64,
    builds: Vec<crate::domain::GalaxyBuild>,
) {
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

pub(super) fn merge_remote_artifacts(source: &[Game], target: &mut [Game]) {
    for target_game in target {
        if target_game.remote_artifacts.is_empty()
            && let Some(source_game) = source
                .iter()
                .find(|game| game.product_id == target_game.product_id)
        {
            target_game.remote_artifacts = source_game.remote_artifacts.clone();
        }
        if let Some(source_game) = source
            .iter()
            .find(|game| game.product_id == target_game.product_id)
        {
            target_game.location = source_game.location.clone();
            target_game.installers = source_game.installers.clone();
            target_game.patches = source_game.patches.clone();
            target_game.extras = source_game.extras.clone();
            target_game.disk_usage = source_game.disk_usage;
            merge_product_metadata(&mut target_game.metadata, source_game.metadata.clone());
            if target_game.galaxy_builds.is_empty() {
                target_game.galaxy_builds = source_game.galaxy_builds.clone();
            }
        }
        for target_dlc in &mut target_game.dlcs {
            if target_dlc.owned
                && target_dlc.remote_artifacts.is_empty()
                && let Some(source_dlc) = source
                    .iter()
                    .flat_map(|game| game.dlcs.iter())
                    .find(|dlc| dlc.product_id == target_dlc.product_id)
            {
                target_dlc.remote_artifacts = source_dlc.remote_artifacts.clone();
            }
            if let Some(source_dlc) = source
                .iter()
                .flat_map(|game| game.dlcs.iter())
                .find(|dlc| dlc.product_id == target_dlc.product_id)
            {
                target_dlc.location = source_dlc.location.clone();
                target_dlc.installers = source_dlc.installers.clone();
                target_dlc.extras = source_dlc.extras.clone();
                target_dlc.disk_usage = source_dlc.disk_usage;
                merge_product_metadata(&mut target_dlc.metadata, source_dlc.metadata.clone());
                if target_dlc.galaxy_builds.is_empty() {
                    target_dlc.galaxy_builds = source_dlc.galaxy_builds.clone();
                }
            }
        }
    }
}

pub(super) fn merge_cached_media(source: &[Game], target: &mut [Game]) {
    for target_game in target {
        if let Some(source_game) = source
            .iter()
            .find(|game| game.product_id == target_game.product_id)
        {
            retain_cached_path(&mut target_game.artwork, &source_game.artwork);
            retain_cached_path(&mut target_game.detail_artwork, &source_game.detail_artwork);
            retain_cached_path(&mut target_game.hero_logo, &source_game.hero_logo);
            retain_cached_path(&mut target_game.icon, &source_game.icon);
        }
        for target_dlc in &mut target_game.dlcs {
            if let Some(source_dlc) = source
                .iter()
                .flat_map(|game| game.dlcs.iter())
                .find(|dlc| dlc.product_id == target_dlc.product_id)
            {
                retain_cached_path(&mut target_dlc.artwork, &source_dlc.artwork);
                retain_cached_path(&mut target_dlc.detail_artwork, &source_dlc.detail_artwork);
                retain_cached_path(&mut target_dlc.hero_logo, &source_dlc.hero_logo);
                retain_cached_path(&mut target_dlc.icon, &source_dlc.icon);
            }
        }
    }
}

fn retain_patch_note_cache(
    mut cache: HashMap<i64, Rc<Vec<PatchNote>>>,
    previous: &[Game],
    current: &[Game],
) -> HashMap<i64, Rc<Vec<PatchNote>>> {
    cache.retain(|product_id, _| {
        let previous = previous.iter().find(|game| game.product_id == *product_id);
        let current = current.iter().find(|game| game.product_id == *product_id);
        matches!((previous, current), (Some(previous), Some(current)) if previous.changelog == current.changelog)
    });
    cache
}

pub(super) fn retain_cached_path(
    target: &mut Option<std::path::PathBuf>,
    cached: &Option<std::path::PathBuf>,
) {
    if cached.as_ref().is_some_and(|path| path.is_file()) {
        *target = cached.clone();
    }
}

pub(super) fn start_owned_library_sync(
    w: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
    token: auth::Token,
    announce: bool,
    force_gamesdb_refresh: bool,
) {
    w.sync_spinner.set_visible(true);
    w.sync_spinner.set_spinning(true);
    w.sync_status.set_visible(true);
    w.sync_status.set_label("Updating library");
    w.sync_progress.set_fraction(0.01);
    w.sync_progress.set_visible(true);
    w.account_library_status.set_label("Synchronizing…");
    let persistence = start_sync_persistence_worker();
    let installer_language = model.borrow().config.installer_language.clone();
    let (sender, receiver) = mpsc::channel::<anyhow::Result<online::SyncEvent>>();
    std::thread::spawn(move || {
        let result = (|| {
            let ids = auth::fetch_owned_product_ids(&token)?;
            let mut store = StateStore::open()?;
            for stage in [
                "ownership",
                "products",
                "manifests",
                "metadata",
                "builds",
                "artwork",
                "library_sync",
            ] {
                store.mark_sync_stage_started(stage)?;
            }
            store.replace_owned_products(&ids)?;
            store.mark_sync_stage_finished("ownership", true, None)?;
            sender.send(Ok(online::SyncEvent::Ownership(ids.len())))?;
            online::stream_owned_games(
                &ids,
                &token.access_token,
                &sender,
                force_gamesdb_refresh,
                installer_language.as_deref(),
            )
        })();
        if let Err(error) = result {
            let _ = sender.send(Err(error));
        }
    });
    let w = w.clone();
    let model = model.clone();
    let sync_fraction = Rc::new(std::cell::Cell::new(0.01_f64));
    glib::timeout_add_local(Duration::from_millis(50), move || {
        match receiver.try_recv() {
            Ok(Ok(online::SyncEvent::Ownership(count))) => {
                let mut state = model.borrow_mut();
                state.owned_product_count = count;
                update_account_library_status(&w, &state);
                update_sync_progress(&w, &sync_fraction, 0.04);
                glib::ControlFlow::Continue
            }
            Ok(Ok(online::SyncEvent::BasicBatch {
                games,
                current,
                total,
            })) => {
                let mut state = model.borrow_mut();
                let existing = state
                    .games
                    .iter()
                    .map(|game| game.product_id)
                    .collect::<HashSet<_>>();
                let additions = games
                    .into_iter()
                    .filter(|game| !existing.contains(&game.product_id))
                    .collect::<Vec<_>>();
                let changed = !additions.is_empty();
                let added_at = chrono::Utc::now().timestamp();
                for game in &additions {
                    state
                        .product_activity
                        .entry(game.product_id)
                        .or_default()
                        .last_activity_at = Some(added_at);
                }
                state.games.extend(additions);
                state.games.sort_by_key(|game| game.title.to_lowercase());
                drop(state);
                if changed {
                    rebuild_library(&w, &model);
                }
                update_sync_stage_progress(&w, &sync_fraction, 0.05, 0.15, current, total);
                glib::ControlFlow::Continue
            }
            Ok(Ok(online::SyncEvent::Catalog { mut games })) => {
                merge_remote_artifacts(&model.borrow().games, &mut games);
                merge_cached_media(&model.borrow().games, &mut games);
                let mut state = model.borrow_mut();
                let cache = std::mem::take(&mut state.patch_notes);
                let cache = retain_patch_note_cache(cache, &state.games, &games);
                state.patch_notes = cache;
                let needs_initial_render = state.games.is_empty();
                let existing = state
                    .games
                    .iter()
                    .map(|game| game.product_id)
                    .collect::<HashSet<_>>();
                let added_at = chrono::Utc::now().timestamp();
                for game in &games {
                    if !existing.contains(&game.product_id) {
                        state
                            .product_activity
                            .entry(game.product_id)
                            .or_default()
                            .last_activity_at = Some(added_at);
                    }
                }
                state.games = games;
                let games_to_persist = state.games.clone();
                drop(state);
                if needs_initial_render {
                    rebuild_library(&w, &model);
                }
                let _ = persistence.send(SyncPersistence::Catalog(games_to_persist));
                update_sync_progress(&w, &sync_fraction, 0.20);
                record_sync_stage_finished("products", true);
                glib::ControlFlow::Continue
            }
            Ok(Ok(online::SyncEvent::FileMetadata {
                product_id,
                artifacts,
                current,
                total,
            })) => {
                apply_remote_artifacts_to_model(
                    &mut model.borrow_mut().games,
                    product_id,
                    artifacts.clone(),
                );
                let _ = persistence.send(SyncPersistence::Manifest(product_id, artifacts));
                update_sync_stage_progress(&w, &sync_fraction, 0.20, 0.30, current, total);
                finish_sync_stage_at_end("manifests", current, total);
                glib::ControlFlow::Continue
            }
            Ok(Ok(online::SyncEvent::Enrichment {
                product_id,
                metadata,
                current,
                total,
            })) => {
                apply_metadata_to_model(&mut model.borrow_mut().games, product_id, *metadata);
                update_sync_stage_progress(&w, &sync_fraction, 0.50, 0.18, current, total);
                finish_sync_stage_at_end("metadata", current, total);
                glib::ControlFlow::Continue
            }
            Ok(Ok(online::SyncEvent::Builds {
                product_id,
                builds,
                windows_observed,
                macos_observed,
                current,
                total,
            })) => {
                apply_builds_to_model(&mut model.borrow_mut().games, product_id, builds.clone());
                let _ = persistence.send(SyncPersistence::Builds {
                    product_id,
                    builds,
                    windows_observed,
                    macos_observed,
                });
                update_sync_stage_progress(&w, &sync_fraction, 0.68, 0.14, current, total);
                finish_sync_stage_at_end("builds", current, total);
                glib::ControlFlow::Continue
            }
            Ok(Ok(online::SyncEvent::Media {
                product_id,
                artwork,
                detail_artwork,
                hero_logo,
                icon,
                current,
                total,
            })) => {
                update_streamed_media(
                    &w,
                    &model,
                    product_id,
                    artwork,
                    detail_artwork,
                    hero_logo,
                    icon,
                );
                update_sync_stage_progress(&w, &sync_fraction, 0.82, 0.17, current, total);
                finish_sync_stage_at_end("artwork", current, total);
                glib::ControlFlow::Continue
            }
            Ok(Ok(online::SyncEvent::Complete { mut games })) => {
                let count = model.borrow().owned_product_count;
                tracing::info!(count, "owned GOG library synchronization complete");
                merge_remote_artifacts(&model.borrow().games, &mut games);
                merge_cached_media(&model.borrow().games, &mut games);
                let games_to_persist = games.clone();
                let _ = persistence.send(SyncPersistence::Catalog(games_to_persist));
                let mut state = model.borrow_mut();
                let cache = std::mem::take(&mut state.patch_notes);
                let cache = retain_patch_note_cache(cache, &state.games, &games);
                state.patch_notes = cache;
                state.online_synced_at = Some(chrono::Utc::now().timestamp());
                let existing = state
                    .games
                    .iter()
                    .map(|game| game.product_id)
                    .collect::<HashSet<_>>();
                let added_at = chrono::Utc::now().timestamp();
                for game in &games {
                    if !existing.contains(&game.product_id) {
                        state
                            .product_activity
                            .entry(game.product_id)
                            .or_default()
                            .last_activity_at = Some(added_at);
                    }
                }
                state.games = games;
                update_account_library_status(&w, &state);
                drop(state);
                super::window::start_managed_reconciliation(&w, &model);
                update_metadata_filter_options(&w, &model);
                refresh_filters(&w, &model.borrow());
                w.sync_spinner.set_spinning(false);
                w.sync_spinner.set_visible(false);
                w.sync_progress.set_fraction(1.0);
                w.sync_progress.set_visible(false);
                w.sync_status.set_visible(false);
                record_sync_success();
                super::window::start_update_check(
                    &w,
                    &model,
                    crate::updates::CheckMode::Automatic,
                    false,
                );
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                tracing::warn!(%error, "owned GOG library synchronization failed");
                w.sync_spinner.set_spinning(false);
                w.sync_spinner.set_visible(false);
                w.sync_progress.set_visible(false);
                w.sync_status.set_visible(false);
                update_account_library_status(&w, &model.borrow());
                tracing::debug!(announce, "library synchronization status cleared");
                record_sync_failure();
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => {
                w.sync_spinner.set_spinning(false);
                w.sync_spinner.set_visible(false);
                w.sync_progress.set_visible(false);
                w.sync_status.set_visible(false);
                glib::ControlFlow::Break
            }
        }
    });
}

fn update_sync_stage_progress(
    w: &Widgets,
    current_fraction: &std::cell::Cell<f64>,
    stage_start: f64,
    stage_weight: f64,
    current: usize,
    total: usize,
) {
    let stage_fraction = if total == 0 {
        1.0
    } else {
        (current as f64 / total as f64).clamp(0.0, 1.0)
    };
    update_sync_progress(
        w,
        current_fraction,
        stage_start + stage_weight * stage_fraction,
    );
}

fn update_sync_progress(w: &Widgets, current_fraction: &std::cell::Cell<f64>, estimate: f64) {
    let estimate = estimate.clamp(current_fraction.get(), 0.99);
    current_fraction.set(estimate);
    w.sync_progress.set_fraction(estimate);
}

fn finish_sync_stage_at_end(stage: &'static str, current: usize, total: usize) {
    if current >= total {
        record_sync_stage_finished(stage, true);
    }
}

fn record_sync_stage_finished(stage: &'static str, succeeded: bool) {
    std::thread::spawn(move || {
        if let Ok(store) = StateStore::open() {
            let message = (!succeeded).then_some("Synchronization stage failed");
            let _ = store.mark_sync_stage_finished(stage, succeeded, message);
        }
    });
}

fn record_sync_failure() {
    std::thread::spawn(|| {
        if let Ok(store) = StateStore::open() {
            for stage in [
                "ownership",
                "products",
                "manifests",
                "metadata",
                "builds",
                "artwork",
                "library_sync",
            ] {
                let _ = store.mark_sync_stage_finished(
                    stage,
                    false,
                    Some("Synchronization did not complete"),
                );
            }
        }
    });
}

fn record_sync_success() {
    std::thread::spawn(|| {
        if let Ok(store) = StateStore::open() {
            for stage in [
                "ownership",
                "products",
                "manifests",
                "metadata",
                "builds",
                "artwork",
                "library_sync",
            ] {
                let _ = store.mark_sync_stage_finished(stage, true, None);
            }
        }
    });
}

pub(super) fn start_product_file_refresh(
    w: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
    target_id: i64,
) {
    let Some(token) = model.borrow().account_token.clone() else {
        show_status(w, "Sign in to refresh GOG files");
        return;
    };
    let request = model.borrow().games.iter().find_map(|game| {
        let is_target =
            game.product_id == target_id || game.dlcs.iter().any(|dlc| dlc.product_id == target_id);
        is_target.then(|| {
            (
                game.product_id,
                game.title.clone(),
                game.dlcs
                    .iter()
                    .filter(|dlc| dlc.owned)
                    .map(|dlc| (dlc.product_id, dlc.title.clone()))
                    .collect::<Vec<_>>(),
            )
        })
    });
    let Some((base_product_id, title, dlcs)) = request else {
        show_status(w, "Could not find this product in the library");
        return;
    };
    show_status(w, &format!("Refreshing files for {title}…"));
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result =
            online::fetch_product_file_metadata(&token.access_token, base_product_id, &dlcs);
        let _ = sender.send(result);
    });
    let w = w.clone();
    let model = model.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        match receiver.try_recv() {
            Ok(Ok(manifests)) => {
                let mut persistence = Vec::new();
                for (product_id, artifacts, _raw_json) in manifests {
                    apply_remote_artifacts_to_model(
                        &mut model.borrow_mut().games,
                        product_id,
                        artifacts.clone(),
                    );
                    persistence.push((product_id, artifacts));
                }
                std::thread::spawn(move || {
                    let Ok(store) = StateStore::open() else {
                        return;
                    };
                    for (product_id, artifacts) in persistence {
                        if let Err(error) = store.observe_download_manifest(product_id, &artifacts)
                        {
                            tracing::warn!(product_id, %error, "could not observe targeted file manifest");
                        }
                        if let Err(error) = store.cache_download_manifest(product_id, &artifacts) {
                            tracing::warn!(product_id, %error, "could not cache targeted file manifest");
                        }
                    }
                });
                if model.borrow().selected == Some(target_id) {
                    render_product_details(&w, &model, target_id);
                }
                show_status(&w, "GOG file list updated");
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                tracing::warn!(target_id, %error, "targeted GOG file refresh failed");
                show_status(&w, &format!("Could not refresh this game’s files: {error}"));
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

pub(super) fn render_product_details(w: &Widgets, model: &Rc<RefCell<AppModel>>, product_id: i64) {
    let product = model.borrow().games.iter().find_map(|game| {
        if game.product_id == product_id {
            Some((game.clone(), None))
        } else {
            game.dlcs
                .iter()
                .find(|dlc| dlc.product_id == product_id)
                .cloned()
                .map(|dlc| (game.clone(), Some(dlc)))
        }
    });
    let Some((game, dlc)) = product else {
        return;
    };
    if let Some(dlc) = dlc {
        show_dlc_page(w, model, game.product_id, &dlc);
    } else {
        show_game(w, model, product_id, Some(false));
    }
}

pub(super) fn update_account_widgets(w: &Widgets, profile: Option<&auth::Profile>) {
    if let Some(profile) = profile {
        w.account_button_name.set_label(&profile.username);
        w.account_name.set_label(&profile.username);
        let member_since = profile
            .member_since
            .and_then(|timestamp| chrono::DateTime::from_timestamp(timestamp, 0))
            .map(|date| date.format("Member since %Y").to_string());
        let details = [
            (!profile.email.is_empty()).then(|| profile.email.clone()),
            (!profile.country.is_empty()).then(|| format!("Country: {}", profile.country)),
            (!profile.preferred_language.is_empty())
                .then(|| format!("Language: {}", profile.preferred_language)),
            (!profile.selected_currency.is_empty())
                .then(|| format!("Currency: {}", profile.selected_currency)),
            member_since,
            Some(format!("GOG ID: {}", profile.user_id)),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n");
        w.account_details.set_label(&details);
        if let Some(path) = &profile.avatar_path {
            let file = gio::File::for_path(path);
            if let Ok(texture) = gdk::Texture::from_file(&file) {
                w.account_avatar.set_paintable(Some(&texture));
                w.account_button_avatar.set_custom_image(Some(&texture));
            }
        }
        w.sign_in.set_visible(false);
        w.reconnect.set_visible(false);
        w.sign_out.set_visible(true);
    } else {
        w.account_button_name.set_label("Sign in");
        w.account_name.set_label("Not signed in");
        w.account_details
            .set_label("Connect your GOG account to synchronize your library.");
        w.account_avatar.set_paintable(None::<&gdk::Paintable>);
        w.account_button_avatar
            .set_custom_image(None::<&gdk::Paintable>);
        w.sign_in.set_visible(true);
        w.reconnect.set_visible(false);
        w.sign_out.set_visible(false);
    }
}

pub(super) fn start_network_monitor(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>) {
    let monitor = gio::NetworkMonitor::default();
    update_network_status(w, model, monitor.is_network_available());
    let w = w.clone();
    let model = model.clone();
    monitor.connect_network_changed(move |_, available| {
        update_network_status(&w, &model, available);
    });
}

pub(super) fn update_network_status(w: &Widgets, model: &Rc<RefCell<AppModel>>, available: bool) {
    download::set_network_available(available);
    model.borrow_mut().network_available = available;
    w.account_offline_indicator.set_visible(!available);
    w.account_connection_status.set_label(if available {
        "Network: Online"
    } else {
        "Network: Offline — cached library remains available"
    });
    w.account_connection_status
        .remove_css_class(if available { "error" } else { "success" });
    w.account_connection_status
        .add_css_class(if available { "success" } else { "error" });
    update_header_network_indicator(w, &model.borrow());
}

pub(super) fn update_header_network_indicator(w: &Widgets, model: &AppModel) {
    for class in ["success", "warning", "error"] {
        w.header_network_button.remove_css_class(class);
        w.header_network_icon.remove_css_class(class);
        w.header_network_slash.remove_css_class(class);
    }
    let session_valid = model
        .account_token
        .as_ref()
        .is_some_and(|token| token.expires_at > chrono::Utc::now().timestamp());
    let (class, tooltip, slashed) = if !model.network_available {
        ("error", "Offline — cached library only", true)
    } else if !session_valid {
        ("warning", "Online, but the GOG session needs renewal", true)
    } else {
        ("success", "Online and connected to GOG", false)
    };
    w.header_network_icon.add_css_class(class);
    w.header_network_button.add_css_class(class);
    w.header_network_slash.add_css_class(class);
    w.header_network_slash.set_visible(slashed);
    w.header_network_button
        .set_tooltip_text(Some(if !session_valid && model.network_available {
            "GOG session unavailable — click to sign in again"
        } else {
            tooltip
        }));
    w.reconnect
        .set_visible(model.account_profile.is_some() && !session_valid);
}

pub(super) fn update_account_library_status(w: &Widgets, model: &AppModel) {
    let synchronized = model
        .online_synced_at
        .and_then(|timestamp| chrono::DateTime::from_timestamp(timestamp, 0))
        .map(|date| {
            date.with_timezone(&chrono::Local)
                .format("%b %-d, %-I:%M %p")
                .to_string()
        });
    let text = if model.owned_product_count == 0 {
        "Online library not synchronized".to_owned()
    } else if let Some(synchronized) = synchronized {
        format!(
            "{} games owned on GOG\nLast synchronized {synchronized}",
            model.owned_product_count
        )
    } else {
        format!("{} games owned on GOG", model.owned_product_count)
    };
    w.account_library_status.set_label(&text);
}

pub(super) fn show_status(w: &Widgets, message: &str) {
    w.status.set_label(message);
    w.status.set_visible(true);
}

pub(super) fn downloaded_product_ids(_jobs: &[DownloadJobRecord]) -> HashSet<i64> {
    let mut products = HashSet::new();
    if let Ok(store) = StateStore::open()
        && let Ok(files) = store.managed_files()
    {
        products.extend(
            files
                .into_iter()
                .filter(|file| file.present)
                .map(|file| file.product_id),
        );
    }
    products
}

pub(super) fn downloaded_installer_product_ids(_jobs: &[DownloadJobRecord]) -> HashSet<i64> {
    let mut products = HashSet::new();
    if let Ok(store) = StateStore::open()
        && let Ok(files) = store.managed_files()
    {
        products.extend(
            files
                .into_iter()
                .filter(|file| file.present && file.kind == ArtifactKind::Installer)
                .map(|file| file.product_id),
        );
    }
    products
}

pub(super) fn reconcile_managed_directory(model: &mut AppModel) -> String {
    let result = (|| -> anyhow::Result<managed::RebuildSummary> {
        let mut store = StateStore::open()?;
        let summary = managed::rebuild(&mut store, &model.config.download_directory, &model.games)?;
        let files = store.managed_files()?;
        managed::apply_to_games(&mut model.games, &files);
        managed::set_locations(&mut model.games, &model.config.download_directory);
        model.download_jobs = store.download_jobs()?;
        model.downloaded_products = downloaded_product_ids(&model.download_jobs);
        model.downloaded_installer_products =
            downloaded_installer_product_ids(&model.download_jobs);
        Ok(summary)
    })();
    match result {
        Ok(summary) => format!(
            "Indexed {} files ({} matched, {} unmatched; {} partial downloads retained)",
            summary.files, summary.matched, summary.unmatched, summary.partials
        ),
        Err(error) => {
            tracing::warn!(%error, "could not rebuild managed-file index");
            format!("Could not rebuild downloaded-file index: {error}")
        }
    }
}

pub(super) fn local_files_exist(files: &[LibraryFile]) -> bool {
    files.iter().any(|file| file.path.is_file())
}

#[cfg(test)]
mod media_cache_tests {
    use super::*;

    #[test]
    fn library_scan_keeps_only_unchanged_patch_note_caches() {
        let unchanged = Rc::new(vec![PatchNote {
            title: "Patch 1".into(),
            version: Some("1".into()),
            date: None,
            body_markup: "Cached".into(),
        }]);
        let changed = Rc::new(Vec::new());
        let cache = HashMap::from([(1, unchanged.clone()), (2, changed)]);
        let previous = vec![
            Game {
                product_id: 1,
                changelog: "same".into(),
                ..Game::default()
            },
            Game {
                product_id: 2,
                changelog: "old".into(),
                ..Game::default()
            },
        ];
        let current = vec![
            Game {
                product_id: 1,
                changelog: "same".into(),
                ..Game::default()
            },
            Game {
                product_id: 2,
                changelog: "new".into(),
                ..Game::default()
            },
        ];

        let cache = retain_patch_note_cache(cache, &previous, &current);

        assert!(Rc::ptr_eq(cache.get(&1).unwrap(), &unchanged));
        assert!(!cache.contains_key(&2));
    }

    #[test]
    fn catalog_refresh_preserves_local_managed_content() {
        let local = LibraryFile {
            name: "setup_1.3.0.5.exe".into(),
            path: "/games/grim-dawn/setup_1.3.0.5.exe".into(),
            size: 42,
        };
        let source = vec![Game {
            product_id: 42,
            location: "/games/grim-dawn".into(),
            installers: vec![local.clone()],
            disk_usage: 42,
            ..Default::default()
        }];
        let mut target = vec![Game {
            product_id: 42,
            title: "Grim Dawn".into(),
            ..Default::default()
        }];

        merge_remote_artifacts(&source, &mut target);

        assert_eq!(target[0].installers, vec![local]);
        assert_eq!(
            target[0].location,
            std::path::PathBuf::from("/games/grim-dawn")
        );
        assert_eq!(target[0].disk_usage, 42);
    }

    #[test]
    fn catalog_refresh_retains_existing_cached_header_media() {
        let path = std::env::temp_dir().join(format!(
            "ludomere-media-cache-test-{}.png",
            std::process::id()
        ));
        std::fs::write(&path, b"cached").unwrap();
        let source = vec![Game {
            product_id: 42,
            detail_artwork: Some(path.clone()),
            hero_logo: Some(path.clone()),
            ..Default::default()
        }];
        let mut target = vec![Game {
            product_id: 42,
            detail_artwork: Some("new-api-background.jpg".into()),
            ..Default::default()
        }];

        merge_cached_media(&source, &mut target);

        assert_eq!(target[0].detail_artwork.as_ref(), Some(&path));
        assert_eq!(target[0].hero_logo.as_ref(), Some(&path));
        let _ = std::fs::remove_file(path);
    }
}

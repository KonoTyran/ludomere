use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CoverState {
    Pending,
    Loading,
    Loaded,
    Unavailable,
    Failed(String),
}

pub(super) fn sync_error_fingerprint(model: &AppModel) -> String {
    let mut failures = Vec::new();
    for (kind, states) in [("cover", &model.cover_states), ("icon", &model.icon_states)] {
        failures.extend(states.iter().filter_map(|(id, state)| match state {
            CoverState::Failed(error) => Some(format!("{kind}:{id}:{error}")),
            _ => None,
        }));
    }
    failures.extend(
        model
            .section_states
            .iter()
            .filter_map(|(key, state)| match state {
                SectionState::Failed(error) => Some(format!("{key:?}:{error}")),
                _ => None,
            }),
    );
    if model.sync_failed {
        failures.push(format!("sync:{:?}", model.sync_message));
    }
    failures.sort();
    failures.join("\n")
}

pub(super) fn refresh_sync_status(w: &Widgets, model: &AppModel) {
    let pending = model
        .section_states
        .values()
        .filter(|state| matches!(state, SectionState::Loading))
        .count();
    let metadata_pending = model
        .section_states
        .iter()
        .filter(|((_, section), state)| {
            *section == online::DetailSection::Metadata && matches!(state, SectionState::Loading)
        })
        .count();
    let failed = model
        .section_states
        .values()
        .filter(|state| matches!(state, SectionState::Failed(_)))
        .count();
    let cover_failed = model
        .cover_states
        .values()
        .chain(model.icon_states.values())
        .filter(|state| matches!(state, CoverState::Failed(_)))
        .count();
    let errors_visible =
        model.dismissed_sync_error.as_ref() != Some(&sync_error_fingerprint(model));
    let message = if model.sync_running {
        model.sync_message.clone()
    } else if pending > 0 {
        let stage = if metadata_pending > 0 {
            "Metadata and game details"
        } else {
            "Game details"
        };
        Some(
            if errors_visible && (model.sync_failed || cover_failed > 0) {
                format!("{stage} · {pending} pending · some sync work failed")
            } else {
                format!("{stage} · {pending} pending")
            },
        )
    } else if model.sync_failed && errors_visible {
        model.sync_message.clone()
    } else if cover_failed > 0 && errors_visible {
        Some(format!("Library images · {cover_failed} failed"))
    } else if failed > 0 && errors_visible {
        Some(format!("Game details · {failed} failed"))
    } else {
        None
    };
    let active = model.sync_running || pending > 0;
    w.sync_spinner.set_spinning(active);
    w.sync_spinner.set_visible(active);
    w.sync_progress.set_visible(model.sync_running);
    w.sync_status.set_visible(message.is_some());
    w.sync_status
        .set_label(message.as_deref().unwrap_or_default());
    w.sync_status.set_tooltip_text(message.as_deref());
    w.sync_retry.set_visible(
        !model.sync_running
            && errors_visible
            && cover_failed > 0
            && model.sync_session.is_some_and(online::has_failed_images),
    );
    w.sync_options
        .set_visible(!model.sync_running && errors_visible && model.sync_failed);
    w.sync_dismiss.set_visible(
        !model.sync_running
            && errors_visible
            && (model.sync_failed || failed > 0 || cover_failed > 0),
    );
}

pub(super) fn update_cover_state(
    w: &Widgets,
    model: &Rc<RefCell<AppModel>>,
    id: i64,
    state: CoverState,
) {
    model.borrow_mut().cover_states.insert(id, state.clone());
    let mut child = w.home_grid.first_child();
    while let Some(wrapper) = child {
        if let Some(card) = wrapper.first_child()
            && card.widget_name() == id.to_string()
        {
            apply_card_cover_state(&card, Some(&state));
            break;
        }
        child = wrapper.next_sibling();
    }
}

pub(super) fn cancel_cover_indicators(w: &Widgets) {
    let mut child = w.home_grid.first_child();
    while let Some(wrapper) = child {
        if let Some(art) =
            find_named_descendant(&wrapper, "card-art").and_downcast::<gtk::Picture>()
            && (art.has_css_class("image-pending") || art.has_css_class("image-loading"))
        {
            if art.paintable().is_some() {
                set_picture_status(&art, "image-ready", "Image loaded");
            } else {
                set_picture_status(&art, "image-unavailable", "Image loading cancelled");
            }
        }
        child = wrapper.next_sibling();
    }
}

pub(super) fn update_streamed_media(
    w: &Widgets,
    model: &Rc<RefCell<AppModel>>,
    product_id: i64,
    artwork: Option<std::path::PathBuf>,
    detail_artwork: Option<std::path::PathBuf>,
    hero_logo: Option<std::path::PathBuf>,
    icon: Option<std::path::PathBuf>,
) {
    let selected_product = model.borrow().detail_target.map(|(id, _)| id);
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
            {
                set_card_picture(&picture, path, 23, 23);
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
            {
                set_card_picture(&picture, path, card_width, card_width * 9 / 16);
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
    w.sync_status.set_label("Game list · connecting…");
    w.sync_progress.set_fraction(0.01);
    w.sync_progress.set_visible(true);
    w.account_library_status.set_label("Synchronizing…");
    let (epoch, generation, language) = {
        let mut state = model.borrow_mut();
        state.sync_generation = state.sync_generation.wrapping_add(1);
        if force_gamesdb_refresh {
            state
                .section_states
                .retain(|_, state| matches!(state, SectionState::Loading));
        }
        state.core_loading = true;
        state.sync_running = true;
        state.sync_failed = false;
        state.dismissed_sync_error = None;
        state.sync_message = Some("Game list · connecting…".into());
        state.cover_states.clear();
        state.icon_states.clear();
        (
            state.account_epoch,
            state.sync_generation,
            state.config.installer_language.clone(),
        )
    };
    let session = online::begin_library_session();
    model.borrow_mut().sync_session = Some(session);
    let (sender, receiver) = mpsc::channel::<anyhow::Result<online::SyncEvent>>();
    std::thread::spawn(move || {
        let result = (|| {
            let ids = auth::fetch_owned_product_ids(&token)?;
            sender.send(Ok(online::SyncEvent::Ownership(ids.len())))?;
            online::stream_owned_games(
                &ids,
                &token.access_token,
                &sender,
                force_gamesdb_refresh,
                language.as_deref(),
                session,
            )
        })();
        if let Err(error) = result {
            let _ = sender.send(Err(error));
        }
    });
    tracing::debug!(announce, "starting library synchronization");
    monitor_library_sync(w, model, receiver, epoch, generation, false, None);
}

pub(super) fn retry_failed_images(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>) {
    let (session, epoch, generation, prior_error) = {
        let mut state = model.borrow_mut();
        if state.sync_running || state.logout_pending {
            return;
        }
        let Some(session) = state.sync_session else {
            state.sync_failed = true;
            state.sync_message = Some("Open Options to synchronize this library first".into());
            state.dismissed_sync_error = None;
            drop(state);
            refresh_sync_status(w, &model.borrow());
            return;
        };
        state.sync_generation = state.sync_generation.wrapping_add(1);
        let prior_error = state
            .sync_failed
            .then(|| state.sync_message.clone())
            .flatten();
        state.sync_running = true;
        state.sync_failed = false;
        state.dismissed_sync_error = None;
        state.sync_message = Some("Retrying failed images…".into());
        (
            session,
            state.account_epoch,
            state.sync_generation,
            prior_error,
        )
    };
    refresh_sync_status(w, &model.borrow());
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        if let Err(error) = online::retry_failed_images(&sender, session) {
            let _ = sender.send(Err(error));
        }
    });
    monitor_library_sync(w, model, receiver, epoch, generation, true, prior_error);
}

fn monitor_library_sync(
    w: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
    receiver: mpsc::Receiver<anyhow::Result<online::SyncEvent>>,
    epoch: u64,
    generation: u64,
    image_retry: bool,
    prior_error: Option<String>,
) {
    let w = w.clone();
    let model = model.clone();
    let sync_fraction = Rc::new(std::cell::Cell::new(0.01_f64));
    glib::timeout_add_local(Duration::from_millis(16), move || {
        if model.borrow().account_epoch != epoch || model.borrow().sync_generation != generation {
            return glib::ControlFlow::Break;
        }
        let started = std::time::Instant::now();
        let mut changed = false;
        let mut terminal = false;
        let mut complete = false;
        // Bound both backlog work and presentation work per GTK turn.
        for _ in 0..32 {
            if started.elapsed() >= Duration::from_millis(4) {
                break;
            }
            match receiver.try_recv() {
                Ok(Ok(online::SyncEvent::Ownership(count))) => {
                    model.borrow_mut().owned_product_count = count;
                    model.borrow_mut().sync_message = Some(format!("Game list · 0/{count}"));
                    update_account_library_status(&w, &model.borrow());
                }
                Ok(Ok(online::SyncEvent::BasicBatch {
                    games,
                    current,
                    total,
                })) => {
                    for game in &games {
                        if game.artwork.is_none() {
                            model
                                .borrow_mut()
                                .cover_states
                                .insert(game.product_id, CoverState::Pending);
                        }
                    }
                    merge_core_catalog(&mut model.borrow_mut().games, games, false);
                    model.borrow_mut().sync_message =
                        Some(format!("Game list · {current}/{total}"));
                    changed = true;
                    update_sync_stage_progress(&w, &sync_fraction, 0.04, 0.40, current, total);
                }
                Ok(Ok(online::SyncEvent::Catalog { games })) => {
                    model.borrow_mut().core_loading = false;
                    merge_core_catalog(&mut model.borrow_mut().games, games, true);
                    changed = true;
                    model.borrow_mut().sync_message = Some("Grid images · preparing…".into());
                    update_sync_progress(&w, &sync_fraction, 0.45);
                }
                Ok(Ok(online::SyncEvent::CoversQueued { product_ids })) => {
                    model
                        .borrow_mut()
                        .cover_states
                        .extend(product_ids.into_iter().map(|id| (id, CoverState::Pending)));
                    let mut child = w.home_grid.first_child();
                    while let Some(wrapper) = child {
                        if let Some(card) = wrapper.first_child()
                            && let Ok(id) = card.widget_name().parse::<i64>()
                        {
                            apply_card_cover_state(&card, model.borrow().cover_states.get(&id));
                        }
                        child = wrapper.next_sibling();
                    }
                    update_image_sync_progress(&w, &model, &sync_fraction);
                }
                Ok(Ok(online::SyncEvent::IconsQueued { product_ids })) => {
                    model
                        .borrow_mut()
                        .icon_states
                        .extend(product_ids.into_iter().map(|id| (id, CoverState::Pending)));
                    update_image_sync_progress(&w, &model, &sync_fraction);
                }
                Ok(Ok(online::SyncEvent::IconStarted { product_id })) => {
                    model
                        .borrow_mut()
                        .icon_states
                        .insert(product_id, CoverState::Loading);
                }
                Ok(Ok(online::SyncEvent::IconFinished {
                    product_id,
                    outcome,
                    ..
                })) => {
                    let state = match outcome {
                        online::CoverOutcome::Loaded(path) => {
                            update_streamed_media(
                                &w,
                                &model,
                                product_id,
                                None,
                                None,
                                None,
                                Some(path),
                            );
                            CoverState::Loaded
                        }
                        online::CoverOutcome::Unavailable => CoverState::Unavailable,
                        online::CoverOutcome::Failed(error) => CoverState::Failed(error),
                    };
                    model.borrow_mut().icon_states.insert(product_id, state);
                    update_image_sync_progress(&w, &model, &sync_fraction);
                }
                Ok(Ok(online::SyncEvent::CoverStarted { product_id })) => {
                    update_cover_state(&w, &model, product_id, CoverState::Loading);
                }
                Ok(Ok(online::SyncEvent::CoverFinished {
                    product_id,
                    outcome,
                    ..
                })) => {
                    let state = match outcome {
                        online::CoverOutcome::Loaded(path) => {
                            update_streamed_media(
                                &w,
                                &model,
                                product_id,
                                Some(path),
                                None,
                                None,
                                None,
                            );
                            CoverState::Loaded
                        }
                        online::CoverOutcome::Unavailable => CoverState::Unavailable,
                        online::CoverOutcome::Failed(error) => CoverState::Failed(error),
                    };
                    update_cover_state(&w, &model, product_id, state);
                    update_image_sync_progress(&w, &model, &sync_fraction);
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
                    update_sync_stage_progress(&w, &sync_fraction, 0.45, 0.54, current, total);
                }
                Ok(Ok(online::SyncEvent::FileMetadata {
                    product_id,
                    artifacts,
                    ..
                })) => apply_remote_artifacts_to_model(
                    &mut model.borrow_mut().games,
                    product_id,
                    artifacts,
                ),
                Ok(Ok(online::SyncEvent::Enrichment {
                    product_id,
                    metadata,
                    ..
                })) => {
                    apply_metadata_to_model(&mut model.borrow_mut().games, product_id, *metadata)
                }
                Ok(Ok(online::SyncEvent::Builds {
                    product_id, builds, ..
                })) => apply_builds_to_model(&mut model.borrow_mut().games, product_id, builds),
                Ok(Ok(online::SyncEvent::Complete { games })) => {
                    merge_core_catalog(&mut model.borrow_mut().games, games, true);
                    model.borrow_mut().online_synced_at = Some(chrono::Utc::now().timestamp());
                    changed = true;
                    terminal = true;
                    complete = true;
                    break;
                }
                Ok(Ok(online::SyncEvent::ImageRetryComplete)) => {
                    complete = true;
                    terminal = true;
                    break;
                }
                Ok(Err(error)) => {
                    tracing::warn!(message = %online::sync_error_message(&error), "owned GOG library synchronization failed");
                    model.borrow_mut().sync_failed = true;
                    model.borrow_mut().sync_message = Some(format!(
                        "Library sync failed · {}",
                        online::sync_error_message(&error)
                    ));
                    record_sync_failure(image_retry);
                    terminal = true;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    model.borrow_mut().sync_failed = true;
                    model.borrow_mut().sync_message =
                        Some("Library sync interrupted · open Options to refresh".into());
                    record_sync_failure(image_retry);
                    terminal = true;
                    break;
                }
            }
        }
        if changed {
            model
                .borrow_mut()
                .games
                .sort_by_cached_key(|game| game.title.to_lowercase());
            rebuild_library(&w, &model);
            refresh_collection_metadata(&w, &model);
        }
        if terminal {
            model.borrow_mut().core_loading = false;
            model.borrow_mut().sync_running = false;
            if let Some(error) = &prior_error {
                model.borrow_mut().sync_failed = true;
                model.borrow_mut().sync_message = Some(error.clone());
            }
            let unfinished = model
                .borrow()
                .cover_states
                .iter()
                .filter_map(|(id, state)| {
                    matches!(state, CoverState::Pending | CoverState::Loading).then_some(*id)
                })
                .collect::<Vec<_>>();
            for id in unfinished {
                update_cover_state(
                    &w,
                    &model,
                    id,
                    CoverState::Failed("Image loading did not finish".into()),
                );
            }
            for state in model.borrow_mut().icon_states.values_mut() {
                if matches!(state, CoverState::Pending | CoverState::Loading) {
                    *state = CoverState::Failed("Icon loading did not finish".into());
                }
            }
            update_account_library_status(&w, &model.borrow());
            refresh_filters(&w, &model.borrow());
            if complete && !image_retry {
                update_policies::check_updates(&w, &model, false);
                super::window::start_managed_reconciliation(&w, &model);
                refresh_local_action_state(&w, &model);
                update_metadata_filter_options(&w, &model);
                refresh_filters(&w, &model.borrow());
                let state = model.borrow();
                record_sync_completion(
                    state
                        .cover_states
                        .values()
                        .chain(state.icon_states.values())
                        .filter(|state| matches!(state, CoverState::Failed(_)))
                        .count(),
                    false,
                );
            }
            if complete && image_retry {
                let state = model.borrow();
                record_sync_completion(
                    state
                        .cover_states
                        .values()
                        .chain(state.icon_states.values())
                        .filter(|state| matches!(state, CoverState::Failed(_)))
                        .count(),
                    true,
                );
            }
            tracing::debug!(complete, "library synchronization presentation complete");
            refresh_sync_status(&w, &model.borrow());
            let state = model.borrow();
            let failed_images = state
                .cover_states
                .values()
                .chain(state.icon_states.values())
                .filter(|state| matches!(state, CoverState::Failed(_)))
                .count();
            if state.sync_failed {
                show_status(
                    &w,
                    state
                        .sync_message
                        .as_deref()
                        .unwrap_or("Library synchronization failed"),
                );
            } else if failed_images > 0 {
                show_status(
                    &w,
                    &format!(
                        "Library synchronized; {failed_images} images failed. Use the image Retry control to try again."
                    ),
                );
            } else {
                show_status(
                    &w,
                    if image_retry {
                        "Image retry completed"
                    } else {
                        "Library synchronized"
                    },
                );
            }
            glib::ControlFlow::Break
        } else {
            refresh_sync_status(&w, &model.borrow());
            glib::ControlFlow::Continue
        }
    });
}

fn update_image_sync_progress(
    w: &Widgets,
    model: &Rc<RefCell<AppModel>>,
    fraction: &std::cell::Cell<f64>,
) {
    let mut state = model.borrow_mut();
    let total = state.cover_states.len() + state.icon_states.len();
    let finished = state
        .cover_states
        .values()
        .chain(state.icon_states.values())
        .filter(|state| !matches!(state, CoverState::Pending | CoverState::Loading))
        .count();
    let failed = state
        .cover_states
        .values()
        .chain(state.icon_states.values())
        .filter(|state| matches!(state, CoverState::Failed(_)))
        .count();
    state.sync_message = Some(if failed == 0 {
        format!("Grid images + icons · {finished}/{total}")
    } else {
        format!("Grid images + icons · {finished}/{total} · {failed} failed")
    });
    drop(state);
    update_sync_stage_progress(w, fraction, 0.45, 0.54, finished, total);
}

fn merge_core_catalog(current: &mut Vec<Game>, incoming: Vec<Game>, authoritative: bool) {
    let ids = incoming
        .iter()
        .map(|game| game.product_id)
        .collect::<HashSet<_>>();
    for game in incoming {
        if let Some(existing) = current
            .iter_mut()
            .find(|existing| existing.product_id == game.product_id)
        {
            online::apply_core_product(existing, game);
        } else {
            current.push(game);
        }
    }
    if authoritative {
        current.retain(|game| ids.contains(&game.product_id));
    }
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

fn record_sync_failure(images_only: bool) {
    std::thread::spawn(move || {
        if let Ok(store) = StateStore::open() {
            for stage in ["ownership", "products", "artwork", "library_sync"] {
                if images_only && matches!(stage, "ownership" | "products") {
                    continue;
                }
                let _ = store.mark_sync_stage_finished(
                    stage,
                    false,
                    Some("Synchronization did not complete"),
                );
            }
        }
    });
}

fn record_sync_completion(image_failures: usize, images_only: bool) {
    std::thread::spawn(move || {
        if let Ok(store) = StateStore::open() {
            for stage in ["ownership", "products", "artwork", "library_sync"] {
                if images_only && matches!(stage, "ownership" | "products") {
                    continue;
                }
                let failed = image_failures > 0 && matches!(stage, "artwork" | "library_sync");
                let message = failed.then(|| {
                    format!("{image_failures} library images failed; retry synchronization")
                });
                let _ = store.mark_sync_stage_finished(stage, !failed, message.as_deref());
            }
        }
    });
}

pub(super) fn start_product_file_refresh(
    w: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
    target_id: i64,
) {
    let id = model
        .borrow()
        .games
        .iter()
        .find(|game| {
            game.product_id == target_id || game.dlcs.iter().any(|dlc| dlc.product_id == target_id)
        })
        .map(|game| game.product_id);
    if let Some(id) = id {
        request_product_section(w, model, id, online::DetailSection::Acquisition, true);
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
}

pub(super) fn show_progress(w: &Widgets, message: &str) {
    w.live_status.set_label(message);
    w.live_status.set_visible(!message.is_empty());
}

pub(super) fn local_files_exist(files: &[LibraryFile]) -> bool {
    files.iter().any(|file| file.path.is_file())
}

#[cfg(test)]
mod media_cache_tests {
    use super::*;

    #[test]
    fn core_batches_preserve_late_detail_results_and_local_files() {
        let mut current = vec![Game {
            product_id: 1,
            title: "Old title".into(),
            description: "Opened detail".into(),
            detail_artwork: Some("cached.png".into()),
            installers: vec![LibraryFile {
                name: "setup".into(),
                path: "/tmp/setup".into(),
                size: 42,
            }],
            ..Default::default()
        }];
        merge_core_catalog(
            &mut current,
            vec![
                Game {
                    product_id: 1,
                    title: "New title".into(),
                    ..Default::default()
                },
                Game {
                    product_id: 2,
                    title: "Second".into(),
                    ..Default::default()
                },
            ],
            false,
        );
        assert_eq!(current.len(), 2);
        assert_eq!(current[0].title, "New title");
        assert_eq!(current[0].description, "Opened detail");
        assert_eq!(
            current[0].detail_artwork.as_deref(),
            Some(std::path::Path::new("cached.png"))
        );
        assert_eq!(current[0].installers.len(), 1);
        merge_core_catalog(
            &mut current,
            vec![Game {
                product_id: 1,
                title: "New title".into(),
                ..Default::default()
            }],
            true,
        );
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].description, "Opened detail");
    }
}

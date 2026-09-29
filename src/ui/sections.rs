use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SectionState {
    Loading,
    Ready,
    Failed(String),
}

#[derive(Clone, Default)]
pub(super) struct LocalActionState {
    pub installed: Option<crate::domain::InstalledGame>,
    pub installed_update: bool,
    pub backup_update: bool,
    pub downloaded: bool,
    pub dlc: DlcActionState,
}

pub(super) fn current_detail(
    model: &AppModel,
    id: i64,
    parent: Option<i64>,
) -> Option<DetailPageModel> {
    if let Some(parent) = parent {
        let game = model.games.iter().find(|game| game.product_id == parent)?;
        let dlc = game.dlcs.iter().find(|dlc| dlc.product_id == id)?;
        Some(DetailPageModel::dlc(game, dlc.clone()))
    } else {
        model
            .games
            .iter()
            .find(|game| game.product_id == id)
            .map(|game| DetailPageModel::game(game.clone(), model.favorites.contains(&id)))
    }
}

pub(super) fn section_product(id: i64, parent: Option<i64>, scope: online::DetailSection) -> i64 {
    if matches!(
        scope,
        online::DetailSection::Acquisition | online::DetailSection::Builds
    ) {
        parent.unwrap_or(id)
    } else {
        id
    }
}

pub(super) fn current_primary_action(
    model: &AppModel,
    id: i64,
    parent: Option<i64>,
) -> GamePrimaryAction {
    let Some(local) = model.local_actions.get(&id) else {
        return if model.installed_games.contains_key(&id) {
            GamePrimaryAction::Play
        } else {
            GamePrimaryAction::Download
        };
    };
    let acquisition_ready = matches!(
        model
            .section_states
            .get(&(parent.unwrap_or(id), online::DetailSection::Acquisition)),
        Some(SectionState::Ready)
    );
    ready_local_action(local, acquisition_ready)
}

fn ready_local_action(local: &LocalActionState, acquisition_ready: bool) -> GamePrimaryAction {
    primary_action_for_state(
        local.installed.is_some(),
        acquisition_ready && local.installed_update,
        acquisition_ready && local.backup_update,
        local.downloaded,
        if acquisition_ready {
            local.dlc
        } else {
            DlcActionState::default()
        },
    )
}

fn local_action_details(game: Game) -> Vec<DetailPageModel> {
    let mut details = game
        .dlcs
        .iter()
        .filter(|dlc| dlc.owned)
        .map(|dlc| DetailPageModel::dlc(&game, dlc.clone()))
        .collect::<Vec<_>>();
    details.push(DetailPageModel::game(game, false));
    details
}

pub(super) fn invalidate_section_requests(model: &mut AppModel) {
    model.account_epoch = model.account_epoch.wrapping_add(1);
    model.detail_generation = model.detail_generation.wrapping_add(1);
    model.detail_target = None;
    model.section_states.clear();
    model.section_queue.clear();
    model.section_active.clear();
    model.section_forced.clear();
    model.local_refresh_running = false;
    model.local_refresh_pending = false;
    model.sync_running = false;
    model.sync_session = None;
    model.dismissed_sync_error = None;
    model.sync_message = None;
    model.sync_failed = false;
    model.cover_states.clear();
    model.icon_states.clear();
    online::invalidate_library_session();
}

pub(super) fn request_product_section(
    w: &Widgets,
    model: &Rc<RefCell<AppModel>>,
    id: i64,
    section: online::DetailSection,
    retry: bool,
) {
    let key = (id, section);
    {
        let mut state = model.borrow_mut();
        if state.logout_pending {
            return;
        }
        let foreground = state
            .detail_target
            .is_some_and(|(id, parent)| section_product(id, parent, section) == key.0);
        let AppModel {
            section_states,
            section_queue,
            ..
        } = &mut *state;
        if !queue_section(section_states, section_queue, key, foreground, retry) {
            return;
        }
        if retry {
            state.section_forced.insert(key);
        }
    }
    refresh_sync_status(w, &model.borrow());
    pump_sections(w, model);
}

fn queue_section(
    states: &mut HashMap<(i64, online::DetailSection), SectionState>,
    queue: &mut VecDeque<(i64, online::DetailSection)>,
    key: (i64, online::DetailSection),
    foreground: bool,
    retry: bool,
) -> bool {
    if matches!(states.get(&key), Some(SectionState::Loading)) {
        if foreground && let Some(index) = queue.iter().position(|queued| *queued == key) {
            queue.remove(index);
            queue.push_front(key);
        }
        return false;
    }
    if !retry
        && matches!(
            states.get(&key),
            Some(SectionState::Ready | SectionState::Failed(_))
        )
    {
        return false;
    }
    states.insert(key, SectionState::Loading);
    if key.1 == online::DetailSection::Metadata && !foreground {
        queue.push_back(key);
    } else {
        queue.push_front(key);
    }
    true
}

fn pump_sections(w: &Widgets, model: &Rc<RefCell<AppModel>>) {
    loop {
        let request = {
            let mut state = model.borrow_mut();
            if state.section_active.len() >= 4 {
                return;
            }
            let metadata_active = state
                .section_active
                .iter()
                .filter(|(_, kind)| *kind == online::DetailSection::Metadata)
                .count();
            let Some(index) = state.section_queue.iter().position(|(_, kind)| {
                *kind != online::DetailSection::Metadata || metadata_active < 2
            }) else {
                return;
            };
            let key = state.section_queue.remove(index).unwrap();
            let Some(game) = state
                .games
                .iter()
                .find(|game| game.product_id == key.0)
                .cloned()
                .or_else(|| {
                    state
                        .games
                        .iter()
                        .flat_map(|game| &game.dlcs)
                        .find(|dlc| dlc.product_id == key.0)
                        .cloned()
                        .map(Game::from)
                })
            else {
                state.section_states.remove(&key);
                continue;
            };
            state.section_active.insert(key);
            let force = state.section_forced.remove(&key);
            (
                key,
                game,
                state
                    .account_token
                    .as_ref()
                    .map(|token| token.access_token.clone()),
                state.config.installer_language.clone(),
                state.account_epoch,
                online::account_session(),
                force,
            )
        };
        let (key, game, token, language, epoch, session, force) = request;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| {
                if force && online::account_session() == session {
                    online::invalidate_section_cache(key.0, key.1)?;
                }
                online::fetch_product_section(
                    &game,
                    key.1,
                    token.as_deref(),
                    language.as_deref(),
                    session,
                )
            })();
            let _ = sender.send(result.map_err(|error| {
                if let Some(partial) = error.downcast_ref::<online::PartialSectionError>() {
                    (partial.message.clone(), Some(partial.game.clone()))
                } else {
                    (online::sync_error_message(&error), None)
                }
            }));
        });
        let model = model.clone();
        let w = w.clone_refs();
        glib::timeout_add_local(Duration::from_millis(32), move || {
            if model.borrow().account_epoch != epoch {
                return glib::ControlFlow::Break;
            }
            let result = match receiver.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(_) => Err(("Loading stopped; try again".into(), None)),
            };
            {
                let mut state = model.borrow_mut();
                state.section_active.remove(&key);
                let (game, error) = match result {
                    Ok(game) => (Some(game), None),
                    Err((message, game)) => (game, Some(message)),
                };
                if let Some(game) = game {
                    if let Some(current) =
                        state.games.iter_mut().find(|game| game.product_id == key.0)
                    {
                        online::apply_product_section(current, game, key.1);
                    } else if let Some(dlc) = state
                        .games
                        .iter_mut()
                        .flat_map(|game| &mut game.dlcs)
                        .find(|dlc| dlc.product_id == key.0)
                    {
                        let mut current = Game::from(dlc.clone());
                        online::apply_product_section(&mut current, game, key.1);
                        dlc.description = current.description;
                        dlc.changelog = current.changelog;
                        dlc.screenshots = current.screenshots;
                        dlc.metadata = current.metadata;
                        dlc.detail_artwork = current.detail_artwork;
                        dlc.hero_logo = current.hero_logo;
                        dlc.icon = current.icon;
                        dlc.links = current.links;
                    }
                    if key.1 == online::DetailSection::Product {
                        state.patch_notes.remove(&key.0);
                    }
                }
                state
                    .section_states
                    .insert(key, error.map_or(SectionState::Ready, SectionState::Failed));
            }
            if key.1 == online::DetailSection::Metadata {
                update_metadata_filter_options(&w, &model);
                refresh_filters(&w, &model.borrow());
                refresh_collection_metadata(&w, &model);
            }
            if matches!(
                key.1,
                online::DetailSection::Acquisition | online::DetailSection::Builds
            ) {
                refresh_local_action_state(&w, &model);
            }
            refresh_sync_status(&w, &model.borrow());
            pump_sections(&w, &model);
            glib::ControlFlow::Break
        });
    }
}

/// Reconcile markers and compute action state away from GTK. Completion never navigates.
pub(super) fn refresh_local_action_state(w: &Widgets, model: &Rc<RefCell<AppModel>>) {
    let (games, config, epoch) = {
        let mut state = model.borrow_mut();
        if state.local_refresh_running {
            state.local_refresh_pending = true;
            return;
        }
        state.local_refresh_running = true;
        (
            state.games.clone(),
            state.config.clone(),
            state.account_epoch,
        )
    };
    let (sender, receiver) = mpsc::channel();
    let (marker_sender, marker_receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<_> {
            let store = StateStore::open()?;
            let installed =
                crate::installation::reconcile_installed_games(&store, &config.game_libraries)?;
            let playable = installed
                .iter()
                .filter(|game| window::sidebar_game_is_playable(game, &config.game_libraries))
                .map(|game| game.product_id)
                .collect::<HashSet<_>>();
            let installed = installed
                .into_iter()
                .map(|game| (game.product_id, game))
                .collect::<HashMap<_, _>>();
            let _ = marker_sender.send((installed.clone(), playable.clone()));
            let mut actions = HashMap::new();
            let mut downloaded = downloaded_product_ids(&store.download_jobs()?);
            for game in games {
                if local_files_exist(&game.installers)
                    || local_files_exist(&game.patches)
                    || local_files_exist(&game.extras)
                    || game.dlcs.iter().any(|dlc| {
                        local_files_exist(&dlc.installers) || local_files_exist(&dlc.extras)
                    })
                {
                    downloaded.insert(game.product_id);
                }
                for detail in local_action_details(game) {
                    let local = installed.get(&detail.product_id).cloned();
                    actions.insert(
                        detail.product_id,
                        LocalActionState {
                            installed_update: local.as_ref().is_some_and(|game| {
                                store.installation_update_available(game).unwrap_or(false)
                            }),
                            backup_update: store
                                .installer_backup_update_available(detail.product_id)
                                .unwrap_or(false),
                            downloaded: default_installers_are_downloaded(&detail, &config),
                            dlc: owned_dlc_action_state(&detail, &config, local.is_some()),
                            installed: local,
                        },
                    );
                }
            }
            let mut activity = store.all_product_activity()?;
            for game in installed.values() {
                if let Some(at) = game.installed_at
                    && activity
                        .entry(game.product_id)
                        .or_default()
                        .last_activity_at
                        .is_none_or(|old| at > old)
                {
                    store.record_product_activity(game.product_id, at)?;
                    activity
                        .entry(game.product_id)
                        .or_default()
                        .last_activity_at = Some(at);
                }
            }
            Ok((installed, playable, actions, activity, downloaded))
        })();
        let _ = sender.send(result);
    });
    let model = model.clone();
    let w = w.clone_refs();
    glib::timeout_add_local(Duration::from_millis(32), move || {
        if model.borrow().account_epoch != epoch {
            return glib::ControlFlow::Break;
        }
        if let Ok((installed, playable)) = marker_receiver.try_recv() {
            let mut state = model.borrow_mut();
            state.installed_products = installed
                .values()
                .filter(|game| game.state == crate::domain::InstallationState::Installed)
                .map(|game| game.product_id)
                .collect();
            state.installed_games = installed;
            state.playable_products = playable;
            drop(state);
            refresh_filters(&w, &model.borrow());
        }
        match receiver.try_recv() {
            Ok(result) => {
                let mut state = model.borrow_mut();
                state.local_refresh_running = false;
                if let Ok((installed, playable, actions, activity, downloaded)) = result {
                    state.installed_products = installed
                        .values()
                        .filter(|game| game.state == crate::domain::InstallationState::Installed)
                        .map(|game| game.product_id)
                        .collect();
                    state.installed_games = installed;
                    state.playable_products = playable;
                    state.local_actions = actions;
                    for (id, loaded) in activity {
                        let current = state.product_activity.entry(id).or_default();
                        if current
                            .last_activity_at
                            .is_none_or(|at| loaded.last_activity_at.is_some_and(|new| new > at))
                        {
                            *current = loaded;
                        }
                    }
                    state.local_revision = state.local_revision.wrapping_add(1);
                    state.downloaded_products = downloaded;
                }
                let pending = std::mem::take(&mut state.local_refresh_pending);
                drop(state);
                refresh_filters(&w, &model.borrow());
                if pending {
                    refresh_local_action_state(&w, &model);
                }
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => {
                model.borrow_mut().local_refresh_running = false;
                glib::ControlFlow::Break
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use online::DetailSection::*;

    #[test]
    fn downloaded_owned_dlc_keeps_its_own_install_action() {
        let game = Game {
            product_id: 1,
            slug: "parent".into(),
            dlcs: vec![
                Dlc {
                    product_id: 2,
                    owned: true,
                    title: "Owned add-on".into(),
                    installers: vec![LibraryFile {
                        name: "setup".into(),
                        path: "/tmp/dlc-setup".into(),
                        size: 42,
                    }],
                    ..Default::default()
                },
                Dlc {
                    product_id: 3,
                    owned: false,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let details = local_action_details(game);
        assert_eq!(details.len(), 2);
        let dlc = details
            .iter()
            .find(|detail| detail.product_id == 2)
            .unwrap();
        assert_eq!(dlc.parent_id, Some(1));
        assert_eq!(dlc.installers.len(), 1);
        let actions = details
            .into_iter()
            .map(|detail| {
                (
                    detail.product_id,
                    LocalActionState {
                        downloaded: !detail.installers.is_empty(),
                        ..Default::default()
                    },
                )
            })
            .collect::<HashMap<_, _>>();
        assert_eq!(
            ready_local_action(&actions[&2], false),
            GamePrimaryAction::Install
        );
        assert_eq!(
            ready_local_action(&actions[&1], false),
            GamePrimaryAction::Download
        );
        assert!(!actions.contains_key(&3));
    }
    #[test]
    fn selected_metadata_jumps_background_queue_without_duplicate_work() {
        let mut states = HashMap::new();
        let mut queue = VecDeque::new();
        for id in 0..500 {
            assert!(queue_section(
                &mut states,
                &mut queue,
                (id, Metadata),
                false,
                false
            ));
        }
        assert!(!queue_section(
            &mut states,
            &mut queue,
            (499, Metadata),
            true,
            false
        ));
        assert_eq!(queue.front(), Some(&(499, Metadata)));
        assert_eq!(queue.len(), 500);
        queue.pop_front();
        assert!(!queue_section(
            &mut states,
            &mut queue,
            (499, Metadata),
            true,
            false
        ));
        assert_eq!(queue.len(), 499);
        states.insert((499, Metadata), SectionState::Failed("offline".into()));
        assert!(!queue_section(
            &mut states,
            &mut queue,
            (499, Metadata),
            true,
            false
        ));
        assert!(queue_section(
            &mut states,
            &mut queue,
            (499, Metadata),
            true,
            true
        ));
        states.insert((499, Metadata), SectionState::Ready);
        assert!(!queue_section(
            &mut states,
            &mut queue,
            (499, Metadata),
            true,
            false
        ));
    }
    #[test]
    fn dlc_descriptions_and_artwork_use_child_but_downloads_use_parent() {
        for section in [Product, Metadata, Artwork] {
            assert_eq!(section_product(20, Some(10), section), 20);
        }
        for section in [Acquisition, Builds] {
            assert_eq!(section_product(20, Some(10), section), 10);
        }
    }
    #[test]
    fn missing_remote_metadata_keeps_local_installer_action_usable() {
        let local = LocalActionState {
            downloaded: true,
            backup_update: true,
            dlc: DlcActionState {
                missing_download: true,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            ready_local_action(&local, false),
            GamePrimaryAction::Install
        );
        assert_eq!(
            ready_local_action(&local, true),
            GamePrimaryAction::DownloadUpdate
        );
    }
    #[test]
    fn remote_loading_never_blocks_installed_play() {
        let installed = crate::domain::InstalledGame {
            product_id: 1,
            library_id: "fixture".into(),
            installed_version: None,
            installation_directory: "/tmp/fixture".into(),
            installer_revision_id: None,
            installer_job_id: None,
            installer_files: Vec::new(),
            installer_complete: true,
            installer_operating_system: Some("linux".into()),
            installer_language: None,
            compatibility: None,
            primary_executable: Some("start.sh".into()),
            launch_arguments: Vec::new(),
            state: crate::domain::InstallationState::Installed,
            error: None,
            installed_at: None,
            verified_at: None,
            last_played_at: None,
            playtime_seconds: 0,
            created_at: 0,
            updated_at: 0,
        };
        let local = LocalActionState {
            installed: Some(installed),
            installed_update: true,
            ..Default::default()
        };
        assert_eq!(ready_local_action(&local, false), GamePrimaryAction::Play);
        assert_eq!(
            ready_local_action(&local, true),
            GamePrimaryAction::DownloadUpdate
        );
    }
}

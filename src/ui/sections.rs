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
    pub depot: bool,
    pub backup_update: bool,
    pub downloaded: bool,
    pub dlc: DlcActionState,
    pub coverage: InstallerCoverage,
    pub required_dlcs: HashSet<i64>,
    pub installed_dlcs: HashSet<i64>,
    pub dlc_updates: HashSet<i64>,
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
    ready_local_action(
        local,
        acquisition_ready,
        matches!(
            model
                .section_states
                .get(&(id, online::DetailSection::Builds)),
            Some(SectionState::Ready)
        ),
    )
}

fn ready_local_action(
    local: &LocalActionState,
    acquisition_ready: bool,
    builds_ready: bool,
) -> GamePrimaryAction {
    if local.depot && local.installed.is_some() {
        return if local.installed_update && builds_ready {
            GamePrimaryAction::InstallUpdate
        } else {
            GamePrimaryAction::Play
        };
    }
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
    model.detail_generation = model.detail_generation.wrapping_add(1);
    model.detail_target = None;
    invalidate_section_requests_ui(model);
    online::invalidate_library_session();
}

/// Sign-out revokes presentation immediately; its worker drains backend session locks.
pub(super) fn invalidate_section_requests_ui(model: &mut AppModel) {
    model.account_epoch = model.account_epoch.wrapping_add(1);
    model.organization_pending = false;
    model.hidden_pending.clear();
    model.policy_saving.clear();
    model.section_states.clear();
    model.section_queue.clear();
    model.section_active.clear();
    model.section_forced.clear();
    model.local_refresh_running = false;
    model.local_refresh_pending = false;
    model.local_priority_running = false;
    model.local_priority_pending.clear();
    model.local_versions.clear();
    model.sync_running = false;
    model.sync_session = None;
    model.dismissed_sync_error = None;
    model.sync_message = None;
    model.sync_failed = false;
    model.cover_states.clear();
    model.icon_states.clear();
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
                refresh_local_products(&w, &model, &HashSet::from([key.0]));
            }
            refresh_sync_status(&w, &model.borrow());
            pump_sections(&w, &model);
            glib::ControlFlow::Break
        });
    }
}

/// Full reconciliation is bounded to one worker; terminal products have a separate priority lane.
pub(super) fn refresh_local_action_state(w: &Widgets, model: &Rc<RefCell<AppModel>>) {
    {
        let mut state = model.borrow_mut();
        if state.local_refresh_running {
            state.local_refresh_pending = true;
            return;
        }
        state.local_refresh_running = true;
    }
    start_local_refresh(w, model, None);
}

pub(super) fn refresh_local_products(
    w: &Widgets,
    model: &Rc<RefCell<AppModel>>,
    ids: &HashSet<i64>,
) {
    let roots = owning_game_ids(&model.borrow(), ids);
    {
        let mut state = model.borrow_mut();
        for id in &roots {
            *state.local_versions.entry(*id).or_default() += 1;
        }
        state.local_priority_pending.extend(roots);
        if state.local_priority_running || state.local_priority_pending.is_empty() {
            return;
        }
        state.local_priority_running = true;
    }
    let ids = std::mem::take(&mut model.borrow_mut().local_priority_pending);
    start_local_refresh(w, model, Some(ids));
}

struct LocalProductSnapshot {
    root: i64,
    actions: HashMap<i64, LocalActionState>,
    downloaded: HashSet<i64>,
    installers: HashSet<i64>,
    activity: HashMap<i64, ProductActivity>,
    playable: HashSet<i64>,
    files: Vec<crate::state::ManagedFileRecord>,
    products: HashSet<i64>,
}

fn start_local_refresh(w: &Widgets, model: &Rc<RefCell<AppModel>>, ids: Option<HashSet<i64>>) {
    let state = model.borrow();
    let mut games = state
        .games
        .iter()
        .filter(|game| {
            ids.as_ref()
                .is_none_or(|ids| ids.contains(&game.product_id))
        })
        .cloned()
        .collect::<Vec<_>>();
    // The current page comes first during startup; terminal work never waits for this pass.
    games.sort_by_key(|game| Some(game.product_id) != state.selected);
    let versions = state.local_versions.clone();
    let epoch = state.account_epoch;
    let config = state.config.clone();
    let preferences = local_preferences(&config);
    let existing = state.installed_games.clone();
    drop(state);
    let targeted = ids.is_some();
    let session = online::account_session();
    let (sender, receiver) = mpsc::sync_channel(8);
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<()> {
            let store = StateStore::open()?;
            let products = games
                .iter()
                .map(|game| (game.product_id, game.slug.clone()))
                .collect::<Vec<_>>();
            let installed = if targeted {
                crate::installation::reconcile_installed_products(
                    &store,
                    &config.game_libraries,
                    &products,
                    &existing,
                )?
            } else {
                crate::installation::reconcile_installed_games(&store, &config.game_libraries)?
            }
            .into_iter()
            .map(|game| (game.product_id, game))
            .collect::<HashMap<_, _>>();
            let product_ids = games
                .iter()
                .flat_map(|game| {
                    std::iter::once(game.product_id)
                        .chain(game.dlcs.iter().map(|dlc| dlc.product_id))
                })
                .collect::<Vec<_>>();
            let mut files = if targeted {
                store.managed_files_for_products(&product_ids)?
            } else {
                store.managed_files()?
            };
            let matches = managed::pending_matches(&config.download_directory, &games, &files)?;
            if !matches.is_empty() {
                online::with_account_session(session, || {
                    managed::ensure_download_root(&config.download_directory)?;
                    store.match_managed_files(&matches)
                })?;
                files = if targeted {
                    store.managed_files_for_products(&product_ids)?
                } else {
                    store.managed_files()?
                };
            }
            let managed_paths = files
                .iter()
                .filter(|file| file.present && file.matched)
                .filter_map(|file| {
                    file.provider_file_id
                        .as_ref()
                        .or(file.artifact_path.as_ref())
                        .map(|identity| (file.product_id, identity.clone(), file.version.clone()))
                })
                .collect::<HashSet<_>>();
            let downloaded = files
                .iter()
                .filter(|file| file.present)
                .map(|file| file.product_id)
                .collect::<HashSet<_>>();
            let installers = files
                .iter()
                .filter(|file| file.present && file.kind == ArtifactKind::Installer)
                .map(|file| file.product_id)
                .collect::<HashSet<_>>();
            let mut files_by_product = HashMap::<i64, Vec<crate::state::ManagedFileRecord>>::new();
            for file in files {
                files_by_product
                    .entry(file.product_id)
                    .or_default()
                    .push(file);
            }
            let backup_updates = store.installer_backup_updates()?;
            let mut activity = store.all_product_activity()?;
            let mut activity_repairs = Vec::new();
            for game in installed.values() {
                if let Some(at) = game.installed_at
                    && activity
                        .get(&game.product_id)
                        .and_then(|activity| activity.last_activity_at)
                        .is_none_or(|old| at > old)
                {
                    activity_repairs.push((game.product_id, at));
                    activity
                        .entry(game.product_id)
                        .or_default()
                        .last_activity_at = Some(at);
                }
            }
            for game in games {
                if online::account_session() != session {
                    return Ok(());
                }
                let root = game.product_id;
                let ids = std::iter::once(root)
                    .chain(game.dlcs.iter().map(|dlc| dlc.product_id))
                    .collect::<HashSet<_>>();
                let mut product_downloaded = downloaded
                    .intersection(&ids)
                    .copied()
                    .collect::<HashSet<_>>();
                let files = ids
                    .iter()
                    .flat_map(|id| files_by_product.get(id).into_iter().flatten().cloned())
                    .collect::<Vec<_>>();
                if local_files_exist(&game.installers)
                    || local_files_exist(&game.patches)
                    || local_files_exist(&game.extras)
                    || game.dlcs.iter().any(|dlc| {
                        local_files_exist(&dlc.installers) || local_files_exist(&dlc.extras)
                    })
                {
                    product_downloaded.insert(root);
                }
                let mut actions = HashMap::new();
                for detail in local_action_details(game) {
                    let local = installed.get(&detail.product_id).cloned();
                    let marker = local
                        .as_ref()
                        .map(|game| {
                            crate::installation::load_installation_marker(
                                &game.installation_directory,
                            )
                        })
                        .transpose()?
                        .flatten();
                    let installed_dlcs = marker
                        .as_ref()
                        .map(|marker| marker.dlc.iter().map(|dlc| dlc.product_id).collect())
                        .unwrap_or_default();
                    let mut dlc_updates = HashSet::new();
                    if let Some(marker) = &marker {
                        for dlc in &marker.dlc {
                            if let Some(revision) = dlc.revision_id
                                && store.revision_has_update(revision)?
                            {
                                dlc_updates.insert(dlc.product_id);
                            }
                        }
                    }
                    let coverage = installer_backup_coverage_from(&detail, &config, &managed_paths);
                    actions.insert(
                        detail.product_id,
                        LocalActionState {
                            depot: marker.as_ref().is_some_and(|marker| {
                                marker.source == crate::domain::InstallationSource::GalaxyDepot
                            }),
                            installed_update: local
                                .as_ref()
                                .map(|game| store.installation_update_available(game))
                                .transpose()?
                                .unwrap_or(false),
                            backup_update: backup_updates.contains(&detail.product_id),
                            downloaded: coverage == InstallerCoverage::Complete,
                            dlc: owned_dlc_action_state_from(
                                &detail,
                                &config,
                                local.is_some(),
                                &managed_paths,
                                &installed_dlcs,
                            ),
                            coverage,
                            required_dlcs: required_owned_dlc_ids(&detail, &config),
                            installed_dlcs,
                            dlc_updates,
                            installed: local,
                        },
                    );
                }
                let playable = actions
                    .iter()
                    .filter(|(_, state)| {
                        state.installed.as_ref().is_some_and(|game| {
                            window::sidebar_game_is_playable(game, &config.game_libraries)
                        })
                    })
                    .map(|(&id, _)| id)
                    .collect();
                let snapshot = LocalProductSnapshot {
                    root,
                    actions,
                    playable,
                    files,
                    downloaded: product_downloaded,
                    installers: installers.intersection(&ids).copied().collect(),
                    activity: activity
                        .iter()
                        .filter(|(id, _)| ids.contains(id))
                        .map(|(&id, activity)| (id, *activity))
                        .collect(),
                    products: ids,
                };
                if sender.send(Ok(Some(snapshot))).is_err() {
                    return Ok(());
                }
            }
            for (id, at) in activity_repairs {
                if targeted {
                    break;
                }
                if online::account_session() != session {
                    break;
                }
                store.record_product_activity(id, at)?;
            }
            Ok(())
        })();
        let _ = sender.send(result.map(|_| None).map_err(|error| error.to_string()));
    });
    let w = w.clone_refs();
    let model = model.clone();
    glib::timeout_add_local(Duration::from_millis(32), move || {
        if model.borrow().account_epoch != epoch {
            return glib::ControlFlow::Break;
        }
        let stale_preferences = local_preferences(&model.borrow().config) != preferences;
        let mut complete = false;
        let mut changed = false;
        let mut error = None;
        for _ in 0..16 {
            match receiver.try_recv() {
                Ok(Ok(Some(snapshot))) => {
                    if stale_preferences {
                        continue;
                    }
                    let mut state = model.borrow_mut();
                    if state
                        .local_versions
                        .get(&snapshot.root)
                        .copied()
                        .unwrap_or(0)
                        != versions.get(&snapshot.root).copied().unwrap_or(0)
                    {
                        continue;
                    }
                    for (&id, local) in &snapshot.actions {
                        state.installed_products.remove(&id);
                        state.installed_games.remove(&id);
                        state.playable_products.remove(&id);
                        if let Some(installed) = &local.installed {
                            if installed.state == crate::domain::InstallationState::Installed {
                                state.installed_products.insert(id);
                            }
                            state.installed_games.insert(id, installed.clone());
                        }
                    }
                    state.local_actions.extend(snapshot.actions);
                    replace_product_presence(
                        &mut state.downloaded_products,
                        &snapshot.products,
                        snapshot.downloaded,
                    );
                    replace_product_presence(
                        &mut state.downloaded_installer_products,
                        &snapshot.products,
                        snapshot.installers,
                    );
                    if let Some(game) = state
                        .games
                        .iter_mut()
                        .find(|game| game.product_id == snapshot.root)
                    {
                        managed::apply_to_games(std::slice::from_mut(game), &snapshot.files);
                    }
                    state.playable_products.extend(snapshot.playable);
                    for (id, activity) in snapshot.activity {
                        let current = state.product_activity.entry(id).or_default();
                        if current.last_activity_at.is_none_or(|old| {
                            activity.last_activity_at.is_some_and(|new| new > old)
                        }) {
                            *current = activity;
                        }
                    }
                    state.local_revision = state.local_revision.wrapping_add(1);
                    changed = true;
                }
                Ok(Ok(None)) => {
                    complete = true;
                    break;
                }
                Ok(Err(message)) => {
                    error = Some(message);
                    complete = true;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    error = Some("Local state worker stopped".into());
                    complete = true;
                    break;
                }
            }
        }
        if changed {
            refresh_filters(&w, &model.borrow());
            update_sidebar_download_styles(&w, &model.borrow());
        }
        if let Some(error) = error {
            hold_status_notice(
                Some(&w.status),
                &format!(
                    "Could not refresh local files; previous state retained. Use Manage → Refresh local state to retry: {error}"
                ),
            );
        }
        if !complete {
            return glib::ControlFlow::Continue;
        }
        let mut state = model.borrow_mut();
        if targeted {
            state.local_priority_running = false;
        } else {
            state.local_refresh_running = false;
        }
        let pending = !state.local_priority_pending.is_empty();
        let full_pending =
            stale_preferences || (!targeted && std::mem::take(&mut state.local_refresh_pending));
        let depot = state
            .detail_target
            .filter(|(id, parent)| {
                parent.is_none() && state.local_actions.get(id).is_some_and(|state| state.depot)
            })
            .map(|(id, _)| id);
        drop(state);
        if let Some(id) = depot {
            request_product_section(&w, &model, id, online::DetailSection::Builds, false);
        }
        if pending {
            refresh_local_products(&w, &model, &HashSet::new());
        }
        if full_pending {
            refresh_local_action_state(&w, &model);
        }
        glib::ControlFlow::Break
    });
}

fn replace_product_presence(
    current: &mut HashSet<i64>,
    products: &HashSet<i64>,
    present: HashSet<i64>,
) {
    current.retain(|id| !products.contains(id));
    current.extend(present);
}

fn local_preferences(config: &Config) -> String {
    serde_json::to_string(&(
        &config.game_libraries,
        &config.download_directory,
        &config.installer_language,
        config.installer_windows,
        config.installer_linux,
        config.installer_macos,
    ))
    .expect("local preferences serialize")
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
            ready_local_action(&actions[&2], false, false),
            GamePrimaryAction::Install
        );
        assert_eq!(
            ready_local_action(&actions[&1], false, false),
            GamePrimaryAction::Download
        );
        assert!(!actions.contains_key(&3));
        let mut presence = HashSet::from([2, 3, 99]);
        replace_product_presence(&mut presence, &HashSet::from([1, 2, 3]), HashSet::from([2]));
        assert_eq!(presence, HashSet::from([2, 99]));
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
            ready_local_action(&local, false, false),
            GamePrimaryAction::Install
        );
        assert_eq!(
            ready_local_action(&local, true, false),
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
        let mut local = LocalActionState {
            installed: Some(installed),
            installed_update: true,
            ..Default::default()
        };
        assert_eq!(
            ready_local_action(&local, false, false),
            GamePrimaryAction::Play
        );
        assert_eq!(
            ready_local_action(&local, true, false),
            GamePrimaryAction::DownloadUpdate
        );
        local.depot = true;
        local.backup_update = true;
        assert_eq!(
            ready_local_action(&local, true, false),
            GamePrimaryAction::Play
        );
        assert_eq!(
            ready_local_action(&local, false, true),
            GamePrimaryAction::InstallUpdate
        );
        local.installed_update = false;
        assert_eq!(
            ready_local_action(&local, true, true),
            GamePrimaryAction::Play
        );
    }
}

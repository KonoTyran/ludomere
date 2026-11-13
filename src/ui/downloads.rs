use super::*;

fn coalesce_download_events(
    jobs: &mut Vec<DownloadJobRecord>,
    changed: &mut HashSet<i64>,
    failures: &mut HashMap<String, String>,
    events: impl IntoIterator<Item = download::DownloadManagerEvent>,
) -> (bool, Option<String>) {
    let mut structural = false;
    let mut error = None;
    for event in events {
        match event {
            download::DownloadManagerEvent::QueueSnapshot(snapshot) => {
                for job in &snapshot {
                    if job.state == DownloadState::Complete {
                        failures.remove(&job.job_id);
                    }
                }
                *jobs = snapshot;
                structural = true;
            }
            download::DownloadManagerEvent::Progress {
                job_id,
                downloaded,
                total,
            } => {
                failures.remove(&job_id);
                if let Some(job) = jobs.iter_mut().find(|job| job.job_id == job_id) {
                    job.bytes_downloaded = downloaded;
                    job.total_bytes = total.or(job.total_bytes);
                    job.state = DownloadState::Downloading;
                }
            }
            download::DownloadManagerEvent::ManagedFilesChanged(id) => {
                changed.insert(id);
                structural = true;
            }
            download::DownloadManagerEvent::BookkeepingFailed {
                job_id,
                product_id: _,
                message,
                session,
            } => {
                if session != online::account_session() {
                    continue;
                }
                failures.insert(job_id.clone(), message.clone());
                error = Some(message);
            }
            download::DownloadManagerEvent::AuthenticationRequired => {}
        }
    }
    (structural, error)
}

fn apply_registration_failures(jobs: &mut [DownloadJobRecord], failures: &HashMap<String, String>) {
    for job in jobs {
        if let Some(message) = failures.get(&job.job_id) {
            job.state = DownloadState::Failed;
            job.error = Some(message.clone());
            job.status_message = Some("Files downloaded; registration needs retry".into());
        }
    }
}

pub(super) fn start_download_monitor(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>) {
    struct Snapshot {
        jobs: Vec<DownloadJobRecord>,
        blocked_auto_installs: HashMap<String, i64>,
        active_job_ids: HashSet<String>,
        managed_files_changed: HashSet<i64>,
        error: Option<String>,
        session: u64,
    }

    let (sender, receiver) = mpsc::sync_channel(8);
    let (depot_sender, depot_receiver) = mpsc::channel();
    let manager_events = download::manager_events();
    std::thread::spawn(move || {
        let mut jobs = StateStore::open()
            .and_then(|store| store.download_jobs())
            .unwrap_or_default();
        let mut blocked_auto_installs = HashMap::new();
        let mut downloaded_products = HashSet::new();
        let mut downloaded_installer_products = HashSet::new();
        let mut failures = HashMap::new();
        let mut session = online::account_session();
        while let Ok(event) = manager_events.recv() {
            if session != online::account_session() {
                session = online::account_session();
                failures.clear();
                blocked_auto_installs.clear();
            }
            let mut managed_files_changed = HashSet::new();
            let (structural, mut error) = coalesce_download_events(
                &mut jobs,
                &mut managed_files_changed,
                &mut failures,
                std::iter::once(event).chain(manager_events.try_iter().take(511)),
            );
            if structural {
                let result = StateStore::open().and_then(|store| {
                    Ok((
                        store.download_install_intents()?,
                        store.managed_download_summary()?,
                    ))
                });
                match result {
                    Ok((intents, (products, installers))) => {
                        managed_files_changed
                            .extend(downloaded_products.symmetric_difference(&products).copied());
                        managed_files_changed.extend(
                            downloaded_installer_products
                                .symmetric_difference(&installers)
                                .copied(),
                        );
                        downloaded_products = products;
                        downloaded_installer_products = installers;
                        blocked_auto_installs = intents
                            .into_iter()
                            .filter(|intent| intent.state == "blocked")
                            .filter_map(|intent| {
                                intent
                                    .job_ids
                                    .first()
                                    .map(|job| (job.clone(), intent.product_id))
                            })
                            .collect();
                    }
                    Err(failure) => {
                        error = Some(failure.to_string());
                    }
                }
            }
            let mut presented_jobs = jobs.clone();
            apply_registration_failures(&mut presented_jobs, &failures);
            let snapshot = Snapshot {
                blocked_auto_installs: blocked_auto_installs.clone(),
                active_job_ids: jobs
                    .iter()
                    .filter(|job| download::is_active(&job.job_id))
                    .map(|job| job.job_id.clone())
                    .collect(),
                managed_files_changed,
                error,
                session,
                jobs: presented_jobs,
            };
            if sender.send(snapshot).is_err() {
                break;
            }
        }
    });
    let depot_events = crate::installation::subscribe_depot_events();
    std::thread::spawn(move || {
        while let Ok(crate::installation::DepotManagerEvent::Snapshot(snapshot)) =
            depot_events.recv()
        {
            if depot_sender.send(snapshot).is_err() {
                break;
            }
        }
    });

    let w = w.clone();
    let model = model.clone();
    let previously_active = Rc::new(std::cell::Cell::new(false));
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let mut latest = None;
        // Do not discard product-scoped completion notifications while collapsing
        // frequent progress snapshots. A base installer, patch, or DLC can all
        // change the owning game's action state without changing the coarse set of
        // products that have at least one downloaded installer.
        let mut managed_file_changes = HashSet::new();
        for snapshot in receiver.try_iter().take(256) {
            if snapshot.session != online::account_session() {
                continue;
            }
            managed_file_changes.extend(snapshot.managed_files_changed.iter().copied());
            if let Some(error) = &snapshot.error {
                hold_status_notice(
                    Some(&w.status),
                    &format!(
                        "Could not refresh downloaded files; previous state retained: {error}"
                    ),
                );
            }
            latest = Some(snapshot);
        }
        let mut depot_changed = false;
        {
            let mut state = model.borrow_mut();
            for snapshot in depot_receiver.try_iter().take(512) {
                if let Some(current) = state
                    .depot_operations
                    .iter_mut()
                    .find(|current| current.operation_id == snapshot.operation_id)
                {
                    depot_changed |=
                        current.state != snapshot.state || current.error != snapshot.error;
                    *current = snapshot;
                } else {
                    state.depot_operations.push(snapshot);
                    depot_changed = true;
                }
            }
            sample_transfer_history(&mut state);
        }
        let Some(snapshot) = latest else {
            if depot_changed {
                update_sidebar_download_styles(&w, &model.borrow());
            }
            if depot_changed && w.content.visible_child_name().as_deref() == Some("downloads") {
                rebuild_downloads_page(&w, &model.borrow());
            } else if w.content.visible_child_name().as_deref() == Some("downloads") {
                update_depot_page_progress(&w, &model.borrow());
            }
            return glib::ControlFlow::Continue;
        };
        if let Some(product_id) = snapshot.blocked_auto_installs.values().next() {
            w.finish_setup
                .set_action_target_value(Some(&product_id.to_variant()));
            w.finish_setup.set_visible(true);
        }
        let jobs_changed = {
            let mut state = model.borrow_mut();
            for job in &snapshot.jobs {
                if state
                    .download_jobs
                    .iter()
                    .any(|old| old.job_id == job.job_id && old.state != job.state)
                {
                    let result = match job.state {
                        DownloadState::Complete => Some("Download completed".to_owned()),
                        DownloadState::Failed => Some(notifications::failure_message(
                            "Download failed",
                            job.error
                                .as_deref()
                                .or(job.status_message.as_deref())
                                .unwrap_or("The download failed without further details."),
                        )),
                        DownloadState::Paused => Some("Download paused".to_owned()),
                        _ => None,
                    };
                    if let Some(result) = result {
                        show_status(&w, &format!("{}: {result}", job.title));
                    }
                }
            }
            let changed = download_job_structure_changed(&state.download_jobs, &snapshot.jobs)
                || state.blocked_auto_installs != snapshot.blocked_auto_installs;
            state.blocked_auto_installs = snapshot.blocked_auto_installs;
            state.download_jobs = snapshot.jobs;
            changed
        };
        if jobs_changed || depot_changed {
            update_sidebar_download_styles(&w, &model.borrow());
        }
        if !managed_file_changes.is_empty() {
            let affected_games = owning_game_ids(&model.borrow(), &managed_file_changes);
            // Installation/update availability is derived from the managed-file
            // index, so refresh even when the downloaded-installer set itself did
            // not change (for example, replacing an invalid part or downloading a
            // patch/DLC for a game that already has another installer).
            update_sidebar_download_styles(&w, &model.borrow());
            refresh_local_products(&w, &model, &affected_games);
            if model.borrow().downloaded_only {
                refresh_filters(&w, &model.borrow());
            }
        }
        let state = model.borrow();
        let active = state
            .download_jobs
            .iter()
            .filter(|job| snapshot.active_job_ids.contains(&job.job_id))
            .collect::<Vec<_>>();
        if !active.is_empty() {
            let job = active[0];
            let total = job.total_bytes;
            let finalizing = job.status_message.as_deref() == Some("Finalizing…");
            let display = state.games.iter().find_map(|game| {
                if game.product_id == job.product_id {
                    Some((game.title.as_str(), game.icon.as_ref()))
                } else {
                    game.dlcs
                        .iter()
                        .find(|dlc| dlc.product_id == job.product_id)
                        .map(|dlc| (dlc.title.as_str(), dlc.icon.as_ref()))
                }
            });
            let title = display.map_or(job.title.as_str(), |(title, _)| title);
            let percent = total
                .filter(|total| *total > 0)
                .map(|total| ((job.bytes_downloaded as f64 / total as f64) * 100.0) as u64);
            show_progress(
                &w,
                &if finalizing {
                    format!("Finalizing {title}")
                } else {
                    title.to_owned()
                },
            );
            w.download_percent
                .set_label(&percent.map(|value| format!("{value}%")).unwrap_or_default());
            w.download_percent.set_visible(percent.is_some());
            update_download_artwork(&w.download_artwork, display.and_then(|(_, icon)| icon));
            w.download_status_progress.set_visible(true);
            if let Some(total) = total.filter(|total| *total > 0) {
                w.download_status_progress
                    .set_fraction((job.bytes_downloaded as f64 / total as f64).min(1.0));
            } else {
                w.download_status_progress.pulse();
            }
            previously_active.set(true);
        } else {
            w.download_artwork.set_visible(false);
            w.download_percent.set_visible(false);
            w.download_status_progress.set_visible(false);
            let blocking = state.download_jobs.iter().find_map(|job| {
                job.status_message.as_deref().filter(|message| {
                    message.contains("Authentication required")
                        || message.contains("Waiting for network")
                        || message.contains("Download directory unavailable")
                })
            });
            if let Some(blocking) = blocking {
                show_progress(&w, &format!("Downloads waiting  ·  {blocking}"));
                previously_active.set(false);
            } else if previously_active.replace(false) {
                show_progress(&w, "");
                if state
                    .download_jobs
                    .iter()
                    .all(|job| job.state == DownloadState::Complete)
                {
                    show_status(&w, "Downloads complete");
                }
            }
        }
        drop(state);
        if (jobs_changed || depot_changed)
            && w.content.visible_child_name().as_deref() == Some("downloads")
        {
            rebuild_downloads_page(&w, &model.borrow());
        } else if w.content.visible_child_name().as_deref() == Some("downloads") {
            update_download_page_progress(&w, &model.borrow(), &snapshot.active_job_ids);
            update_depot_page_progress(&w, &model.borrow());
        }
        glib::ControlFlow::Continue
    });
}

fn update_download_artwork(image: &gtk::Image, path: Option<&std::path::PathBuf>) {
    if let Some(path) = path {
        if image.file().as_deref() != path.to_str() || image.paintable().is_none() {
            image.set_from_file(Some(path));
        }
        image.set_visible(true);
    } else {
        image.clear();
        image.set_visible(false);
    }
}

fn sample_transfer_history(model: &mut AppModel) {
    let active_id = model
        .depot_operations
        .iter()
        .find(|operation| depot_active(&operation.state))
        .map(|operation| format!("depot:{}", operation.operation_id))
        .or_else(|| {
            model
                .download_jobs
                .iter()
                .find(|job| download::is_active(&job.job_id))
                .map(|job| format!("download:{}", job.job_id))
        });
    let Some(active_id) = active_id else {
        model.transfer_totals = None;
        return;
    };
    if model.active_transfer_id.as_deref() != Some(&active_id) {
        model.transfer_history.borrow_mut().clear();
        model.transfer_totals = None;
        model.active_transfer_id = Some(active_id);
    }
    let now = std::time::Instant::now();
    let network = model
        .download_jobs
        .iter()
        .map(|job| job.bytes_downloaded)
        .sum::<u64>()
        .saturating_add(
            model
                .depot_operations
                .iter()
                .map(|operation| operation.bytes_downloaded)
                .sum::<u64>(),
        );
    let disk = model
        .download_jobs
        .iter()
        .map(|job| job.bytes_downloaded)
        .sum::<u64>()
        .saturating_add(
            model
                .depot_operations
                .iter()
                .map(|operation| operation.bytes_written)
                .sum::<u64>(),
        );
    if let Some((previous_at, previous_network, previous_disk)) = model.transfer_totals {
        let elapsed = now.duration_since(previous_at).as_secs_f64();
        if elapsed >= 0.5 {
            let mut history = model.transfer_history.borrow_mut();
            history.push_back(TransferHistorySample {
                download_bytes_per_second: network.saturating_sub(previous_network) as f64
                    / elapsed,
                disk_bytes_per_second: disk.saturating_sub(previous_disk) as f64 / elapsed,
            });
            while history.len() > 120 {
                history.pop_front();
            }
            model.transfer_totals = Some((now, network, disk));
        }
    } else {
        model.transfer_totals = Some((now, network, disk));
    }
}

pub(super) fn owning_game_ids(model: &AppModel, changed_products: &HashSet<i64>) -> HashSet<i64> {
    model
        .games
        .iter()
        .filter(|game| {
            changed_products.contains(&game.product_id)
                || game
                    .dlcs
                    .iter()
                    .any(|dlc| changed_products.contains(&dlc.product_id))
        })
        .map(|game| game.product_id)
        .collect()
}

pub(super) fn update_download_page_progress(
    w: &Widgets,
    model: &AppModel,
    active_job_ids: &HashSet<String>,
) {
    let root: gtk::Widget = w.downloads.clone().upcast();
    for job in model
        .download_jobs
        .iter()
        .filter(|job| active_job_ids.contains(&job.job_id))
    {
        if let Some(detail) =
            find_named_descendant(&root, &format!("download-detail-{}", job.job_id))
                .and_downcast::<gtk::Label>()
        {
            let message = job
                .status_message
                .clone()
                .unwrap_or_else(|| format!("Downloading {}", human_size(job.bytes_downloaded)));
            detail.set_label(&message);
        }
        if let Some(progress) =
            find_named_descendant(&root, &format!("download-progress-{}", job.job_id))
                .and_downcast::<gtk::ProgressBar>()
        {
            if let Some(total) = job.total_bytes.filter(|total| *total > 0) {
                progress.set_fraction((job.bytes_downloaded as f64 / total as f64).min(1.0));
                progress.set_text(Some(&format!(
                    "{} / {}",
                    human_size(job.bytes_downloaded),
                    human_size(total)
                )));
                progress.set_show_text(true);
            } else {
                progress.pulse();
            }
        }
    }
    if let Some(job) = model.download_jobs.iter().find(|job| {
        active_job_ids.contains(&job.job_id)
            && find_named_descendant(&root, &format!("active-download-{}", job.job_id)).is_some()
    }) {
        let total = job.total_bytes.unwrap_or_default();
        if let Some(progress) = find_named_descendant(&root, "active-download-progress")
            .and_downcast::<gtk::ProgressBar>()
            && total > 0
        {
            progress.set_fraction((job.bytes_downloaded as f64 / total as f64).min(1.0));
        }
        if let Some(detail) =
            find_named_descendant(&root, "active-download-detail").and_downcast::<gtk::Label>()
        {
            detail.set_label(&format!(
                "{} / {}",
                human_size(job.bytes_downloaded),
                human_size(total)
            ));
        }
    }
}

pub(super) fn download_job_structure_changed(
    old: &[DownloadJobRecord],
    new: &[DownloadJobRecord],
) -> bool {
    old.len() != new.len()
        || old.iter().zip(new).any(|(old, new)| {
            old.job_id != new.job_id
                || old.state != new.state
                || old.error != new.error
                || old.status_message != new.status_message
        })
}

fn featured_downloads<'a>(
    jobs: &'a [DownloadJobRecord],
    operations: &'a [crate::installation::DepotOperationSnapshot],
    active_job_ids: &HashSet<String>,
) -> (
    Option<&'a crate::installation::DepotOperationSnapshot>,
    Option<&'a DownloadJobRecord>,
) {
    if let Some(operation) = operations
        .iter()
        .rev()
        .find(|operation| depot_active(&operation.state))
    {
        return (Some(operation), None);
    }
    if let Some(job) = jobs
        .iter()
        .rev()
        .find(|job| active_job_ids.contains(&job.job_id))
    {
        return (None, Some(job));
    }
    if let Some(operation) = operations
        .iter()
        .rev()
        .find(|operation| matches!(operation.state.as_str(), "interrupted" | "failed"))
    {
        return (Some(operation), None);
    }
    (
        None,
        jobs.iter()
            .rev()
            .find(|job| matches!(job.state.as_str(), "paused" | "failed")),
    )
}

pub(super) fn rebuild_downloads_page(w: &Widgets, model: &AppModel) {
    while let Some(child) = w.downloads.first_child() {
        w.downloads.remove(&child);
    }
    let jobs = &model.download_jobs;
    let page = gtk::Box::new(gtk::Orientation::Vertical, 24);
    page.set_margin_bottom(36);

    let active_job_ids = jobs
        .iter()
        .filter(|job| download::is_active(&job.job_id))
        .map(|job| job.job_id.clone())
        .collect::<HashSet<_>>();
    let (featured_depot, featured_job) =
        featured_downloads(jobs, &model.depot_operations, &active_job_ids);
    if let Some(operation) = featured_depot {
        page.append(&active_depot_header(operation, model, &w.window));
    } else if let Some(job) = featured_job {
        page.append(&active_download_header(job, model, w));
    } else if model
        .transfer_history
        .borrow()
        .iter()
        .any(|sample| sample.download_bytes_per_second > 0.0 || sample.disk_bytes_per_second > 0.0)
    {
        page.append(&completed_transfer_history_header(model));
    }

    let depot_is_active = featured_depot.is_some_and(|operation| depot_active(&operation.state));

    let queued = jobs
        .iter()
        .filter(|job| {
            job.state == "queued" || (depot_is_active && active_job_ids.contains(&job.job_id))
        })
        .filter(|job| depot_is_active || !active_job_ids.contains(&job.job_id))
        .collect::<Vec<_>>();
    let queued_depots = model
        .depot_operations
        .iter()
        .filter(|operation| operation.state == "queued")
        .collect::<Vec<_>>();
    let queue_heading = download_section_heading("Up Next", queued.len() + queued_depots.len());
    if queued.is_empty() && queued_depots.is_empty() {
        let empty_queue = gtk::Box::new(gtk::Orientation::Vertical, 6);
        empty_queue.add_css_class("downloads-empty-queue");
        empty_queue.append(&queue_heading);
        let empty = gtk::Label::new(Some("There are no downloads waiting in the queue"));
        empty.set_xalign(0.0);
        empty.add_css_class("downloads-empty");
        empty_queue.append(&empty);
        page.append(&empty_queue);
    } else {
        page.append(&queue_heading);
        for job in queued {
            page.append(&download_job_card(job, model, w, false));
        }
        for operation in queued_depots {
            page.append(&depot_operation_card(operation, model, w, false));
        }
    }

    let paused_jobs = jobs
        .iter()
        .filter(|job| matches!(job.state.as_str(), "paused" | "failed"))
        .filter(|job| featured_job.is_none_or(|featured| featured.job_id != job.job_id))
        .collect::<Vec<_>>();
    let paused_depots = model
        .depot_operations
        .iter()
        .filter(|operation| matches!(operation.state.as_str(), "interrupted" | "failed"))
        .filter(|operation| {
            featured_depot.is_none_or(|featured| featured.operation_id != operation.operation_id)
        })
        .collect::<Vec<_>>();
    if !paused_jobs.is_empty() || !paused_depots.is_empty() {
        page.append(&download_section_heading(
            "Paused and Failed",
            paused_jobs.len() + paused_depots.len(),
        ));
        for job in paused_jobs {
            page.append(&download_job_card(job, model, w, false));
        }
        for operation in paused_depots {
            page.append(&depot_operation_card(operation, model, w, false));
        }
    }

    const COMPLETED_HISTORY_DAYS: i64 = 7;
    const MAX_COMPLETED_DOWNLOADS: usize = 50;
    let cutoff = chrono::Utc::now().timestamp() - COMPLETED_HISTORY_DAYS * 24 * 60 * 60;
    let mut completed = jobs
        .iter()
        .filter(|job| job.state == "complete" && job.updated_at >= cutoff)
        .collect::<Vec<_>>();
    completed.sort_by_key(|job| std::cmp::Reverse(job.updated_at));
    completed.truncate(MAX_COMPLETED_DOWNLOADS);
    let completed_depots = model
        .depot_operations
        .iter()
        .filter(|operation| operation.state == "complete")
        .collect::<Vec<_>>();
    page.append(&download_section_heading(
        "Completed",
        completed.len() + completed_depots.len(),
    ));
    if completed.is_empty() && completed_depots.is_empty() {
        let empty = gtk::Label::new(Some("Completed downloads will appear here"));
        empty.set_xalign(0.0);
        empty.add_css_class("downloads-empty");
        page.append(&empty);
    } else {
        for job in completed {
            page.append(&download_job_card(job, model, w, false));
        }
        for operation in completed_depots {
            page.append(&depot_operation_card(operation, model, w, false));
        }
    }
    w.downloads.append(&page);
}

fn depot_active(state: &str) -> bool {
    matches!(
        state,
        "preparing"
            | "dependencies"
            | "setup"
            | "verifying"
            | "verifying_existing"
            | "calculating"
            | "downloading"
            | "materializing"
            | "extracting"
            | "committing"
            | "finalizing"
            | "cancelling"
    )
}

fn transfer_history_graph(model: &AppModel) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_widget_name("transfer-history-graph");
    area.set_content_width(250);
    area.set_content_height(140);
    area.set_hexpand(true);
    area.set_vexpand(true);
    let history = model.transfer_history.clone();
    area.set_draw_func(move |_, context, width, height| {
        let history = history.borrow();
        let peak = history
            .iter()
            .flat_map(|sample| {
                [
                    sample.download_bytes_per_second,
                    sample.disk_bytes_per_second,
                ]
            })
            .fold(1.0_f64, f64::max);
        let step = width as f64 / 120.0;
        let offset = (width as f64 - history.len() as f64 * step).max(0.0);
        let plot_height = height as f64 * 0.58;
        for (index, sample) in history.iter().enumerate() {
            let bar_height = sample.download_bytes_per_second / peak * plot_height;
            context.rectangle(
                offset + index as f64 * step,
                height as f64 - bar_height,
                (step - 1.0).max(1.0),
                bar_height,
            );
        }
        let network = gtk::cairo::LinearGradient::new(0.0, 0.0, width as f64, 0.0);
        network.add_color_stop_rgba(0.0, 0.08, 0.48, 0.82, 0.0);
        network.add_color_stop_rgba(0.28, 0.08, 0.48, 0.82, 0.0);
        network.add_color_stop_rgba(0.5, 0.08, 0.48, 0.82, 0.32);
        network.add_color_stop_rgba(0.75, 0.08, 0.48, 0.82, 0.78);
        network.add_color_stop_rgba(1.0, 0.08, 0.48, 0.82, 0.78);
        let _ = context.set_source(&network);
        let _ = context.fill();
        context.set_line_width(2.0);
        for (index, sample) in history.iter().enumerate() {
            let x = offset + index as f64 * step;
            let y = height as f64 - (sample.disk_bytes_per_second / peak * plot_height) - 2.0;
            if index == 0 {
                context.move_to(x, y);
            } else {
                context.line_to(x, y);
            }
        }
        let disk = gtk::cairo::LinearGradient::new(0.0, 0.0, width as f64, 0.0);
        disk.add_color_stop_rgba(0.0, 0.45, 0.78, 0.42, 0.0);
        disk.add_color_stop_rgba(0.28, 0.45, 0.78, 0.42, 0.0);
        disk.add_color_stop_rgba(0.5, 0.45, 0.78, 0.42, 0.4);
        disk.add_color_stop_rgba(0.75, 0.45, 0.78, 0.42, 1.0);
        disk.add_color_stop_rgba(1.0, 0.45, 0.78, 0.42, 1.0);
        let _ = context.set_source(&disk);
        let _ = context.stroke();
    });
    area
}

fn current_rates(model: &AppModel) -> (f64, f64) {
    model
        .transfer_history
        .borrow()
        .back()
        .map_or((0.0, 0.0), |sample| {
            (
                sample.download_bytes_per_second,
                sample.disk_bytes_per_second,
            )
        })
}

fn estimated_remaining(model: &AppModel, remaining: u64) -> Option<String> {
    let history = model.transfer_history.borrow();
    let samples = history.iter().rev().take(16).collect::<Vec<_>>();
    if samples.len() < 8 {
        return None;
    }
    let rate = samples
        .iter()
        .map(|sample| sample.download_bytes_per_second)
        .sum::<f64>()
        / samples.len() as f64;
    (rate > 1.0).then(|| format_remaining((remaining as f64 / rate).ceil() as u64))
}

fn format_remaining(seconds: u64) -> String {
    if seconds >= 3600 {
        format!(
            "About {} hr {} min remaining",
            seconds / 3600,
            seconds % 3600 / 60
        )
    } else if seconds >= 60 {
        format!("About {} min remaining", seconds / 60)
    } else {
        format!("About {seconds} sec remaining")
    }
}

fn depot_phase_progress(
    operation: &crate::installation::DepotOperationSnapshot,
) -> (&'static str, u64, Option<u64>) {
    let label = match operation.state.as_str() {
        "queued" => return ("Waiting to start", 0, None),
        "preparing" => return ("Preparing download", 0, None),
        "calculating" => return ("Calculating download size", 0, None),
        "extracting" => "Extracting game files",
        "verifying" | "verifying_existing" => "Checking game files",
        "dependencies" => "Component data processed",
        _ => {
            return (
                if operation.download_total_bytes == Some(0) && operation.bytes_downloaded == 0 {
                    "No Depot file download required"
                } else if operation.download_total_bytes.is_none() {
                    "Data processed"
                } else if operation.download_total_bytes == Some(0)
                    || !depot_active(&operation.state)
                {
                    "Depot files downloaded"
                } else if operation
                    .download_total_bytes
                    .is_some_and(|total| operation.bytes_downloaded >= total)
                {
                    "Download complete"
                } else {
                    "Downloading data"
                },
                operation.bytes_downloaded,
                operation.download_total_bytes,
            );
        }
    };
    (
        label,
        operation.bytes_completed,
        (operation.total_bytes > 0).then_some(operation.total_bytes),
    )
}

#[derive(Debug, PartialEq)]
enum DepotMeter {
    Hidden,
    Pulse,
    Fraction(f64),
}

#[derive(Debug, PartialEq)]
struct DepotCounter {
    label: &'static str,
    detail: String,
    meter: DepotMeter,
}

fn depot_counter_presentation(
    operation: &crate::installation::DepotOperationSnapshot,
) -> (DepotCounter, DepotCounter) {
    let (label, completed, total) = depot_phase_progress(operation);
    let finishing = matches!(
        operation.state.as_str(),
        "setup" | "committing" | "finalizing"
    );
    let processing = matches!(
        operation.state.as_str(),
        "extracting" | "verifying" | "verifying_existing" | "dependencies"
    );
    let download = DepotCounter {
        label,
        detail: if total == Some(0) && completed == 0 {
            String::new()
        } else if let Some(total) = total.filter(|total| *total > 0) {
            format!("{} / {}", human_size(completed), human_size(total))
        } else if completed > 0 {
            human_size(completed)
        } else {
            "Not reported yet".into()
        },
        meter: if !depot_active(&operation.state) || finishing || operation.state == "cancelling" {
            DepotMeter::Hidden
        } else if let Some(total) = total.filter(|total| *total > 0) {
            DepotMeter::Fraction(depot_download_fraction(completed, total))
        } else if processing || total.is_none() {
            DepotMeter::Pulse
        } else {
            DepotMeter::Hidden
        },
    };
    let writing = matches!(operation.state.as_str(), "materializing" | "extracting");
    let disk = DepotCounter {
        label: if finishing {
            "Finishing installation"
        } else if writing {
            "Writing game files"
        } else {
            "Payload writes"
        },
        detail: if finishing {
            operation.setup.as_ref().map_or_else(
                || "Working…".into(),
                |setup| {
                    if setup.total > 0 {
                        format!(
                            "{} · {} / {}",
                            setup.component, setup.completed, setup.total
                        )
                    } else {
                        setup.component.clone()
                    }
                },
            )
        } else {
            let mut text = format!(
                "Payload data written this run: {}",
                human_size(operation.bytes_written)
            );
            if operation.total_write_bytes > 0 {
                text.push_str(&format!(
                    " · Full write estimate: {}",
                    human_size(operation.total_write_bytes)
                ));
            }
            text
        },
        meter: if writing || finishing {
            DepotMeter::Pulse
        } else {
            DepotMeter::Hidden
        },
    };
    (download, disk)
}

fn update_depot_header_counters(
    named: &HashMap<glib::GString, gtk::Widget>,
    operation: &crate::installation::DepotOperationSnapshot,
) {
    let (download, disk) = depot_counter_presentation(operation);
    for (prefix, counter) in [("active-download", download), ("active-disk", disk)] {
        if let Some(label) = named
            .get(format!("{prefix}-label").as_str())
            .cloned()
            .and_downcast::<gtk::Label>()
        {
            label.set_label(counter.label);
        }
        if let Some(detail) = named
            .get(format!("{prefix}-detail").as_str())
            .cloned()
            .and_downcast::<gtk::Label>()
        {
            detail.set_label(&counter.detail);
            detail.set_tooltip_text(Some(&counter.detail));
        }
        if let Some(progress) = named
            .get(format!("{prefix}-progress").as_str())
            .cloned()
            .and_downcast::<gtk::ProgressBar>()
        {
            progress.set_visible(counter.meter != DepotMeter::Hidden);
            match counter.meter {
                DepotMeter::Hidden => progress.set_fraction(0.0),
                DepotMeter::Pulse => progress.pulse(),
                DepotMeter::Fraction(fraction) => progress.set_fraction(fraction),
            }
        }
    }
}

fn depot_download_fraction(completed: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        (completed as f64 / total as f64).clamp(0.0, 1.0)
    }
}

fn active_header_shell(
    artwork: Option<&std::path::PathBuf>,
    logo: Option<&std::path::PathBuf>,
    title: &str,
    stage: &str,
    model: &AppModel,
) -> (gtk::Box, gtk::Box) {
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    header.add_css_class("active-transfer-header");
    let visual = gtk::Overlay::new();
    visual.set_hexpand(true);
    visual.set_size_request(560, 174);
    let backdrop = picture(artwork, -1, 174, "active-transfer-background");
    backdrop.set_hexpand(true);
    visual.set_child(Some(&backdrop));
    let fade = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    fade.set_can_target(false);
    fade.add_css_class("active-transfer-fade");
    visual.add_overlay(&fade);
    let graph = transfer_history_graph(model);
    graph.set_can_target(false);
    visual.add_overlay(&graph);
    let stage = gtk::Label::new(Some(stage));
    stage.set_halign(gtk::Align::End);
    stage.set_valign(gtk::Align::Start);
    stage.set_margin_top(14);
    stage.set_margin_end(18);
    stage.add_css_class("active-transfer-stage");
    visual.add_overlay(&stage);
    if let Some(path) = logo {
        let logo = active_transfer_logo(path);
        logo.set_halign(gtk::Align::Start);
        logo.set_valign(gtk::Align::Start);
        logo.set_margin_start(18);
        logo.set_margin_top(14);
        visual.add_overlay(&logo);
    } else {
        let title = gtk::Label::new(Some(title));
        title.set_xalign(0.0);
        title.set_halign(gtk::Align::Start);
        title.set_valign(gtk::Align::Start);
        title.set_margin_start(18);
        title.set_margin_top(14);
        title.add_css_class("game-title");
        title.add_css_class("active-transfer-title");
        visual.add_overlay(&title);
    }
    header.append(&visual);
    let details = gtk::Box::new(gtk::Orientation::Vertical, 9);
    details.add_css_class("active-transfer-details");
    details.set_size_request(350, -1);
    details.set_hexpand(false);
    header.append(&details);
    (header, details)
}

fn active_transfer_logo(path: &std::path::PathBuf) -> gtk::Picture {
    const MAX_WIDTH: i32 = 115;
    const MAX_HEIGHT: i32 = 36;
    let logo = gtk::Picture::new();
    logo.set_size_request(MAX_WIDTH, MAX_HEIGHT);
    logo.set_content_fit(gtk::ContentFit::Contain);
    logo.set_can_shrink(true);
    logo.add_css_class("active-transfer-logo");
    if let Ok(source) = gdk_pixbuf::Pixbuf::from_file(path) {
        let scale = (MAX_WIDTH as f64 / source.width() as f64)
            .min(MAX_HEIGHT as f64 / source.height() as f64)
            .min(1.0);
        let width = (source.width() as f64 * scale).round().max(1.0) as i32;
        let height = (source.height() as f64 * scale).round().max(1.0) as i32;
        if let Some(scaled) = source.scale_simple(width, height, InterpType::Bilinear) {
            logo.set_paintable(Some(&gtk::gdk::Texture::for_pixbuf(&scaled)));
        }
    }
    logo
}

fn transfer_stats(model: &AppModel, live: bool) -> gtk::Box {
    let (network, disk) = if live {
        current_rates(model)
    } else {
        (0.0, 0.0)
    };
    let peak = model
        .transfer_history
        .borrow()
        .iter()
        .map(|sample| sample.download_bytes_per_second)
        .fold(0.0_f64, f64::max);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.set_homogeneous(true);
    row.append(&transfer_metric(
        "<span foreground='#3494f2'>▥</span> NETWORK",
        "active-network-rate",
        network,
    ));
    row.append(&transfer_metric(
        "<span foreground='#3494f2'>▥</span> PEAK",
        "active-peak-rate",
        peak,
    ));
    row.append(&transfer_metric(
        "<span foreground='#73c76b'>━</span> DISK WRITE RATE",
        "active-disk-rate",
        disk,
    ));
    row
}

fn transfer_metric(title_markup: &str, value_name: &str, bytes_per_second: f64) -> gtk::Box {
    let metric = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let title = gtk::Label::new(None);
    title.set_use_markup(true);
    title.set_markup(title_markup);
    title.set_xalign(0.0);
    title.add_css_class("transfer-metric-title");
    metric.append(&title);
    let value = gtk::Label::new(Some(&format!("{}/s", human_size(bytes_per_second as u64))));
    value.set_widget_name(value_name);
    value.set_xalign(0.0);
    value.set_width_chars(1);
    value.set_ellipsize(gtk::pango::EllipsizeMode::End);
    value.add_css_class("transfer-metric-value");
    metric.append(&value);
    metric
}

fn labeled_progress(label: &str, fraction: f64, detail: &str, disk: bool) -> gtk::Box {
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 3);
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let label = gtk::Label::new(Some(label));
    label.set_widget_name(if disk {
        "active-disk-label"
    } else {
        "active-download-label"
    });
    label.set_xalign(0.0);
    label.set_hexpand(true);
    heading.append(&label);
    let detail = gtk::Label::new(Some(detail));
    detail.set_width_chars(1);
    detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
    detail.set_widget_name(if disk {
        "active-disk-detail"
    } else {
        "active-download-detail"
    });
    detail.add_css_class("dim-label");
    heading.append(&detail);
    box_.append(&heading);
    let progress = gtk::ProgressBar::new();
    progress.set_widget_name(if disk {
        "active-disk-progress"
    } else {
        "active-download-progress"
    });
    progress.set_fraction(fraction.clamp(0.0, 1.0));
    progress.add_css_class(if disk {
        "download-disk-progress"
    } else {
        "download-network-progress"
    });
    box_.append(&progress);
    box_
}

fn active_download_header(job: &DownloadJobRecord, model: &AppModel, w: &Widgets) -> gtk::Box {
    let game = model
        .games
        .iter()
        .find(|game| game.product_id == job.product_id);
    let title = game.map_or(job.title.as_str(), |game| game.title.as_str());
    let (header, details) = active_header_shell(
        game.and_then(|game| game.detail_artwork.as_ref().or(game.artwork.as_ref())),
        game.and_then(|game| game.hero_logo.as_ref()),
        title,
        match job.state.as_str() {
            "queued" => "QUEUED",
            "paused" => "PAUSED",
            "failed" => "FAILED",
            _ => "DOWNLOADING",
        },
        model,
    );
    header.set_widget_name(&format!("active-download-{}", job.job_id));
    details.append(&transfer_stats(model, download::is_active(&job.job_id)));
    let total = job.total_bytes.unwrap_or_default();
    let fraction = if total > 0 {
        job.bytes_downloaded as f64 / total as f64
    } else {
        0.0
    };
    details.append(&labeled_progress(
        "Downloading data",
        fraction,
        &format!(
            "{} / {}",
            human_size(job.bytes_downloaded),
            human_size(total)
        ),
        false,
    ));
    if let Some(message) = job.error.as_deref().or(job.status_message.as_deref()) {
        let message = gtk::Label::new(Some(message));
        message.set_widget_name("active-download-message");
        message.set_xalign(0.0);
        message.set_wrap(true);
        message.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        message.set_width_chars(1);
        message.set_selectable(true);
        details.append(
            &gtk::ScrolledWindow::builder()
                .child(&message)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vscrollbar_policy(gtk::PolicyType::Automatic)
                .max_content_height(120)
                .propagate_natural_height(true)
                .vexpand(false)
                .build(),
        );
    }
    let eta = estimated_remaining(model, total.saturating_sub(job.bytes_downloaded));
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let eta = gtk::Label::new(eta.as_deref());
    eta.set_widget_name("active-transfer-eta");
    eta.set_xalign(0.0);
    eta.set_hexpand(true);
    eta.add_css_class("dim-label");
    footer.append(&eta);
    let job_id = job.job_id.clone();
    if download::is_active(&job.job_id) {
        let pause = gtk::Button::from_icon_name("media-playback-pause-symbolic");
        pause.set_tooltip_text(Some("Pause"));
        pause.connect_clicked(move |button| {
            if download::cancel(&job_id) {
                button.set_sensitive(false);
            }
        });
        footer.append(&pause);
    } else {
        let resume = gtk::Button::from_icon_name("media-playback-start-symbolic");
        resume.set_tooltip_text(Some(if job.state == "failed" {
            "Retry download"
        } else {
            "Resume"
        }));
        resume.set_sensitive(model.account_token.is_some());
        let token = model
            .account_token
            .as_ref()
            .map(|token| token.access_token.clone());
        let resume_id = job_id.clone();
        let retry = job.state == "failed";
        resume.connect_clicked(move |button| {
            let accepted = token.as_ref().is_some_and(|token| {
                if retry {
                    download::retry(&resume_id, token.clone())
                } else {
                    download::resume(&resume_id, token.clone())
                }
            });
            if accepted {
                button.set_sensitive(false);
            }
        });
        footer.append(&resume);
        let cancel = gtk::Button::from_icon_name("user-trash-symbolic");
        cancel.set_tooltip_text(Some("Cancel permanently"));
        let window = w.window.clone();
        cancel.connect_clicked(move |_| {
            let confirmation = adw::AlertDialog::builder()
                .heading("Cancel download?")
                .body("This removes the download and its partial files.")
                .build();
            confirmation.add_responses(&[("keep", "Keep"), ("cancel", "Cancel Download")]);
            confirmation.set_response_appearance("cancel", adw::ResponseAppearance::Destructive);
            let job_id = job_id.clone();
            confirmation.choose(Some(&window), gio::Cancellable::NONE, move |response| {
                if response == "cancel" {
                    download::remove(&job_id);
                }
            });
        });
        footer.append(&cancel);
    }
    details.append(&footer);
    header
}

fn active_depot_header(
    operation: &crate::installation::DepotOperationSnapshot,
    model: &AppModel,
    window: &adw::ApplicationWindow,
) -> gtk::Box {
    let game = model
        .games
        .iter()
        .find(|game| game.product_id == operation.product_id);
    let (header, details) = active_header_shell(
        game.and_then(|game| game.detail_artwork.as_ref().or(game.artwork.as_ref())),
        game.and_then(|game| game.hero_logo.as_ref()),
        game.map_or("Galaxy installation", |game| game.title.as_str()),
        depot_stage_label(&operation.state),
        model,
    );
    header.set_widget_name(&format!("active-depot-{}", operation.operation_id));
    if operation.state == "cancelling" {
        let progress = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        progress.append(&gtk::Spinner::builder().spinning(true).build());
        progress.append(
            &gtk::Label::builder()
                .label("Cancelling… Waiting for installation to stop and removing temporary files.")
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .max_width_chars(70)
                .xalign(0.0)
                .build(),
        );
        details.append(&progress);
        return header;
    }
    details.append(&transfer_stats(model, depot_active(&operation.state)));
    details.append(&labeled_progress("", 0.0, "", false));
    details.append(&labeled_progress("", 0.0, "", true));
    update_depot_header_counters(&download_progress_widgets(header.upcast_ref()), operation);
    if let Some(message) = operation.error.as_deref() {
        let message = gtk::Label::new(Some(message));
        message.set_widget_name("active-depot-message");
        message.set_xalign(0.0);
        message.set_wrap(true);
        message.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        message.set_width_chars(1);
        message.set_selectable(true);
        details.append(
            &gtk::ScrolledWindow::builder()
                .child(&message)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vscrollbar_policy(gtk::PolicyType::Automatic)
                .max_content_height(120)
                .propagate_natural_height(true)
                .vexpand(false)
                .build(),
        );
    }
    let footer_text = operation.download_total_bytes.and_then(|total| {
        estimated_remaining(model, total.saturating_sub(operation.bytes_downloaded))
    });
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let eta = gtk::Label::new(footer_text.as_deref());
    eta.set_widget_name("active-transfer-eta");
    eta.set_xalign(0.0);
    eta.set_hexpand(true);
    eta.add_css_class("dim-label");
    footer.append(&eta);
    let operation_id = operation.operation_id.clone();
    if depot_active(&operation.state) {
        let pause = gtk::Button::from_icon_name("media-playback-pause-symbolic");
        pause.set_tooltip_text(Some("Pause"));
        pause.connect_clicked(move |button| {
            if crate::installation::cancel_depot_operation(&operation_id) {
                button.set_sensitive(false);
            }
        });
        footer.append(&pause);
    } else {
        let resume = gtk::Button::from_icon_name("media-playback-start-symbolic");
        resume.set_tooltip_text(Some("Resume"));
        resume.set_sensitive(model.account_token.is_some());
        let resume_window = window.clone();
        let resume_id = operation_id.clone();
        let product_id = operation.product_id;
        connect_windows_action(
            &resume,
            window,
            true,
            move || Some(product_id),
            move |_| {
                let _ = gtk::prelude::WidgetExt::activate_action(
                    &resume_window,
                    "win.resume-depot",
                    Some(&resume_id.to_variant()),
                );
            },
        );
        footer.append(&resume);
        let cancel = gtk::Button::from_icon_name("user-trash-symbolic");
        cancel.set_tooltip_text(Some("Cancel permanently"));
        connect_depot_cancellation(
            &cancel,
            window,
            operation_id,
            crate::installation::abandon_depot_operation,
        );
        footer.append(&cancel);
    }
    details.append(&footer);
    header
}

fn connect_depot_cancellation(
    button: &gtk::Button,
    window: &adw::ApplicationWindow,
    operation_id: String,
    abandon: impl Fn(&str) -> bool + 'static,
) {
    let window = window.downgrade();
    let abandon = Rc::new(abandon);
    button.connect_clicked(move |button| {
        let Some(window) = window.upgrade() else { return; };
        let confirmation = adw::AlertDialog::builder()
            .heading("Cancel this installation?")
            .body("This abandons the current attempt and deletes its resumable temporary files. Files already published into the game directory are not rolled back.")
            .build();
        confirmation.add_responses(&[("keep", "Keep"), ("cancel", "Cancel Installation")]);
        confirmation.set_default_response(Some("keep"));
        confirmation.set_close_response("keep");
        confirmation.set_response_appearance("cancel", adw::ResponseAppearance::Destructive);
        let button = button.downgrade();
        let parent = window.downgrade();
        let operation_id = operation_id.clone();
        let abandon = abandon.clone();
        confirmation.choose(Some(&window), gio::Cancellable::NONE, move |response| {
            if response != "cancel" { return; }
            if abandon(&operation_id) {
                if let Some(button) = button.upgrade() { button.set_sensitive(false); }
            } else if let Some(window) = parent.upgrade() {
                let error = adw::AlertDialog::builder()
                    .heading("Cancellation unavailable")
                    .body("Game operations may be busy. The installation may also have finished, changed, or already be cancelling. Refresh its status and retry shortly.")
                    .build();
                error.add_response("close", "Close");
                error.present(Some(&window));
            }
        });
    });
}

fn depot_stage_label(state: &str) -> &'static str {
    match state {
        "queued" => "QUEUED",
        "preparing" => "PREPARING DOWNLOAD",
        "verifying" => "CHECKING FILES",
        "verifying_existing" => "CHECKING EXISTING FILES",
        "calculating" => "CALCULATING DOWNLOAD SIZE",
        "downloading" => "STARTING DOWNLOAD",
        "materializing" => "DOWNLOADING",
        "extracting" => "EXTRACTING GAME FILES",
        "committing" => "INSTALLING FILES",
        "finalizing" => "FINALIZING",
        "cancelling" => "CANCELLING",
        "dependencies" => "PROCESSING REQUIRED COMPONENTS",
        "setup" => "SETTING UP REQUIRED COMPONENTS",
        "interrupted" => "PAUSED",
        "failed" => "FAILED",
        "complete" => "COMPLETE",
        "cancelled" | "abandoned" => "CANCELLED",
        _ => "WORKING",
    }
}

fn completed_transfer_history_header(model: &AppModel) -> gtk::Box {
    let header = gtk::Box::new(gtk::Orientation::Vertical, 0);
    header.add_css_class("completed-transfer-history");
    let status = gtk::Label::new(Some(
        "No transfers are running. Recent transfer history is shown below.",
    ));
    status.set_widget_name("idle-transfer-history-status");
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    status.set_margin_start(18);
    status.set_margin_end(18);
    status.set_margin_top(12);
    header.append(&status);
    let overlay = gtk::Overlay::new();
    overlay.set_size_request(-1, 174);
    overlay.set_child(Some(&transfer_history_graph(model)));
    let stats = transfer_stats(model, false);
    stats.set_halign(gtk::Align::End);
    stats.set_valign(gtk::Align::Start);
    stats.set_size_request(350, -1);
    stats.set_margin_top(16);
    stats.set_margin_end(18);
    overlay.add_overlay(&stats);
    header.append(&overlay);
    header
}

fn depot_operation_card(
    operation: &crate::installation::DepotOperationSnapshot,
    model: &AppModel,
    w: &Widgets,
    featured: bool,
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class(if featured {
        "download-active-card"
    } else {
        "download-queue-row"
    });
    let game = model
        .games
        .iter()
        .find(|game| game.product_id == operation.product_id);
    let width = if featured { 250 } else { 150 };
    row.append(&card_picture(
        game.and_then(|game| game.artwork.as_ref()),
        width,
        width * 9 / 16,
    ));
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 6);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some(
        game.map_or("Galaxy installation", |game| game.title.as_str()),
    ));
    title.set_xalign(0.0);
    title.add_css_class(if featured {
        "game-title"
    } else {
        "section-title"
    });
    copy.append(&title);
    let detail = gtk::Label::new(Some(operation.error.as_deref().unwrap_or(&operation.state)));
    detail.set_widget_name(&format!("depot-detail-{}", operation.operation_id));
    detail.set_xalign(0.0);
    detail.set_wrap(true);
    detail.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    detail.set_width_chars(1);
    detail.set_selectable(true);
    detail.add_css_class("dim-label");
    copy.append(&detail);
    if operation.download_total_bytes.is_some() && operation.state != "complete" {
        let progress = gtk::ProgressBar::new();
        progress.set_widget_name(&format!("depot-progress-{}", operation.operation_id));
        progress.set_fraction(depot_download_fraction(
            operation.bytes_downloaded,
            operation.download_total_bytes.unwrap_or_default(),
        ));
        copy.append(&progress);
    }
    row.append(&copy);
    if operation.state == "cancelling" {
        detail.set_label("Cancelling…");
        row.append(&gtk::Spinner::builder().spinning(true).build());
        return row;
    }
    let operation_id = operation.operation_id.clone();
    if depot_active(&operation.state) {
        let pause = gtk::Button::from_icon_name("media-playback-pause-symbolic");
        pause.set_tooltip_text(Some("Pause"));
        pause.connect_clicked(move |button| {
            if crate::installation::cancel_depot_operation(&operation_id) {
                button.set_sensitive(false);
            }
        });
        row.append(&pause);
    } else if matches!(operation.state.as_str(), "interrupted" | "failed") {
        let resume = gtk::Button::from_icon_name("media-playback-start-symbolic");
        resume.set_tooltip_text(Some("Resume"));
        resume.set_sensitive(model.account_token.is_some());
        let resume_window = w.window.clone();
        let product_id = operation.product_id;
        connect_windows_action(
            &resume,
            &w.window,
            true,
            move || Some(product_id),
            move |_| {
                let _ = gtk::prelude::WidgetExt::activate_action(
                    &resume_window,
                    "win.resume-depot",
                    Some(&operation_id.to_variant()),
                );
            },
        );
        row.append(&resume);
    }
    if operation.state != "complete" {
        let cancel = gtk::Button::from_icon_name("user-trash-symbolic");
        cancel.set_tooltip_text(Some("Cancel and remove partial files"));
        let operation_id = operation.operation_id.clone();
        connect_depot_cancellation(
            &cancel,
            &w.window,
            operation_id,
            crate::installation::abandon_depot_operation,
        );
        row.append(&cancel);
    }
    let _ = w;
    row
}

fn download_progress_widgets(root: &gtk::Widget) -> HashMap<glib::GString, gtk::Widget> {
    let mut named = HashMap::new();
    let mut pending = vec![root.clone()];
    while let Some(widget) = pending.pop() {
        // Match find_named_descendant: omit the root and keep the first
        // depth-first occurrence when multiple widgets share a name.
        if widget != *root {
            named
                .entry(widget.widget_name())
                .or_insert_with(|| widget.clone());
        }
        let mut child = widget.last_child();
        while let Some(widget) = child {
            child = widget.prev_sibling();
            pending.push(widget);
        }
    }
    named
}

fn update_depot_page_progress(w: &Widgets, model: &AppModel) {
    let named = download_progress_widgets(w.downloads.upcast_ref());
    if let Some(graph) = named.get("transfer-history-graph") {
        graph.queue_draw();
    }
    if let Some(sample) = model.transfer_history.borrow().back() {
        let live = model
            .depot_operations
            .iter()
            .any(|operation| depot_active(&operation.state))
            || model
                .download_jobs
                .iter()
                .any(|job| download::is_active(&job.job_id));
        update_transfer_metric(
            &named,
            "active-network-rate",
            if live {
                sample.download_bytes_per_second
            } else {
                0.0
            },
        );
        update_transfer_metric(
            &named,
            "active-disk-rate",
            if live {
                sample.disk_bytes_per_second
            } else {
                0.0
            },
        );
        let peak = model
            .transfer_history
            .borrow()
            .iter()
            .map(|sample| sample.download_bytes_per_second)
            .fold(0.0_f64, f64::max);
        update_transfer_metric(&named, "active-peak-rate", peak);
    }
    let remaining = model
        .depot_operations
        .iter()
        .find(|operation| {
            depot_active(&operation.state)
                && named.contains_key(format!("active-depot-{}", operation.operation_id).as_str())
        })
        .and_then(|operation| {
            operation
                .download_total_bytes
                .map(|total| total.saturating_sub(operation.bytes_downloaded))
        })
        .or_else(|| {
            model
                .download_jobs
                .iter()
                .find(|job| {
                    download::is_active(&job.job_id)
                        && named.contains_key(format!("active-download-{}", job.job_id).as_str())
                })
                .map(|job| {
                    job.total_bytes
                        .unwrap_or_default()
                        .saturating_sub(job.bytes_downloaded)
                })
        });
    if let Some(eta) = named
        .get("active-transfer-eta")
        .cloned()
        .and_downcast::<gtk::Label>()
    {
        eta.set_label(
            &remaining
                .and_then(|remaining| estimated_remaining(model, remaining))
                .unwrap_or_default(),
        );
    }
    for operation in &model.depot_operations {
        let (_, completed, total) = depot_phase_progress(operation);
        if let Some(detail) = named
            .get(format!("depot-detail-{}", operation.operation_id).as_str())
            .cloned()
            .and_downcast::<gtk::Label>()
        {
            let percent = total
                .filter(|total| *total > 0)
                .map(|total| completed.saturating_mul(100) / total);
            if let Some(error) = operation.error.as_deref() {
                detail.set_label(error);
            } else {
                detail.set_label(&percent.map_or_else(
                    || depot_stage_label(&operation.state).to_owned(),
                    |percent| {
                        format!(
                            "{} · {}%",
                            depot_stage_label(&operation.state),
                            percent.min(100)
                        )
                    },
                ));
            }
        }
        if let Some(progress) = named
            .get(format!("depot-progress-{}", operation.operation_id).as_str())
            .cloned()
            .and_downcast::<gtk::ProgressBar>()
        {
            if let Some(total) = total.filter(|total| *total > 0) {
                progress.set_fraction(depot_download_fraction(completed, total));
            } else if depot_active(&operation.state) {
                progress.pulse();
            }
        }
        if named.contains_key(format!("active-depot-{}", operation.operation_id).as_str()) {
            update_depot_header_counters(&named, operation);
        }
    }
}

fn update_transfer_metric(
    named: &HashMap<glib::GString, gtk::Widget>,
    name: &str,
    bytes_per_second: f64,
) {
    if let Some(label) = named.get(name).cloned().and_downcast::<gtk::Label>() {
        label.set_label(&format!("{}/s", human_size(bytes_per_second as u64)));
    }
}

pub(super) fn installer_language_options(model: &AppModel) -> Vec<String> {
    let mut languages = model
        .games
        .iter()
        .flat_map(|game| {
            game.remote_artifacts.iter().chain(
                game.dlcs
                    .iter()
                    .filter(|dlc| dlc.owned)
                    .flat_map(|dlc| dlc.remote_artifacts.iter()),
            )
        })
        .filter(|artifact| artifact.kind == ArtifactKind::Installer)
        .filter_map(|artifact| artifact.language.clone())
        .filter(|language| !language.is_empty())
        .collect::<Vec<_>>();
    languages.sort_by_key(|language| language.to_lowercase());
    languages.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    languages.insert(0, "Any language".into());
    languages
}

pub(super) fn download_section_heading(title: &str, count: usize) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("download-section-heading");
    let title = gtk::Label::new(Some(&format!("{title} ({count})")));
    title.add_css_class("section-title");
    row.append(&title);
    let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    separator.set_hexpand(true);
    separator.set_valign(gtk::Align::Center);
    row.append(&separator);
    row
}

pub(super) fn download_job_card(
    job: &crate::state::DownloadJobRecord,
    model: &AppModel,
    w: &Widgets,
    featured: bool,
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.set_widget_name(&job.job_id);
    row.add_css_class(if featured {
        "download-active-card"
    } else {
        "download-queue-row"
    });
    let product_display = model.games.iter().find_map(|game| {
        if game.product_id == job.product_id {
            Some((game.title.as_str(), game.artwork.as_ref()))
        } else {
            game.dlcs
                .iter()
                .find(|dlc| dlc.product_id == job.product_id)
                .map(|dlc| (dlc.title.as_str(), dlc.artwork.as_ref()))
        }
    });
    let artwork = product_display.and_then(|(_, artwork)| artwork);
    let width = if featured { 250 } else { 150 };
    row.append(&card_picture(artwork, width, width * 9 / 16));
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 6);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    let display_title = product_display
        .map(|(title, _)| title)
        .filter(|_| generic_artifact_title(&job.title))
        .unwrap_or(&job.title);
    let title = gtk::Label::new(Some(display_title));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.set_width_chars(1);
    title.set_wrap(true);
    title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title.add_css_class(if featured {
        "game-title"
    } else {
        "section-title"
    });
    copy.append(&title);
    let state_detail = match job.state.as_str() {
        "complete" if job.status_message.is_some() => {
            job.status_message.clone().unwrap_or_default()
        }
        "complete" => {
            let completed = chrono::DateTime::from_timestamp(job.updated_at, 0)
                .map(|date| {
                    date.with_timezone(&chrono::Local)
                        .format("%b %-d, %-I:%M %p")
                        .to_string()
                })
                .unwrap_or_default();
            format!(
                "{} downloaded{}",
                human_size(job.bytes_downloaded),
                if completed.is_empty() {
                    String::new()
                } else {
                    format!("  ·  Completed {completed}")
                }
            )
        }
        "failed" => job
            .error
            .clone()
            .unwrap_or_else(|| "Download failed".into()),
        "paused" => "Paused".into(),
        "downloading" => job
            .status_message
            .clone()
            .unwrap_or_else(|| format!("Downloading {}", human_size(job.bytes_downloaded))),
        "queued" => job
            .status_message
            .clone()
            .unwrap_or_else(|| "Waiting in download queue".into()),
        _ => job.state.as_str().to_string(),
    };
    let artifact_detail = job
        .artifacts
        .first()
        .map(|artifact| {
            [
                artifact.operating_system.as_deref(),
                artifact.language.as_deref(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ")
        })
        .filter(|detail| !detail.is_empty());
    let detail = gtk::Label::new(Some(
        &artifact_detail.map_or(state_detail.clone(), |artifact| {
            format!("{state_detail}  ·  {artifact}")
        }),
    ));
    detail.set_xalign(0.0);
    detail.set_width_chars(1);
    detail.set_widget_name(&format!("download-detail-{}", job.job_id));
    detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
    detail.add_css_class("dim-label");
    copy.append(&detail);
    if job.state == "downloading" {
        let progress = gtk::ProgressBar::new();
        progress.set_widget_name(&format!("download-progress-{}", job.job_id));
        progress.set_hexpand(true);
        if let Some(total) = job.total_bytes.filter(|total| *total > 0) {
            progress.set_fraction((job.bytes_downloaded as f64 / total as f64).min(1.0));
            progress.set_text(Some(&format!(
                "{} / {}",
                human_size(job.bytes_downloaded),
                human_size(total)
            )));
            progress.set_show_text(true);
        } else {
            progress.pulse();
        }
        copy.append(&progress);
    }
    row.append(&copy);
    if let Some(product_id) = model.blocked_auto_installs.get(&job.job_id).copied() {
        let retry = gtk::Button::with_label("Retry installation");
        retry.set_tooltip_text(Some("After resolving the reported prerequisites, retry using saved defaults. No components are downloaded automatically."));
        let status = w.status.clone();
        retry.connect_clicked(move |button| {
            let session = online::account_session();
            button.set_sensitive(false);
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = sender.send(download::retry_install_after_download(product_id));
            });
            let button = button.clone();
            let status = status.clone();
            glib::timeout_add_local(Duration::from_millis(50), move || {
                if online::account_session() != session {
                    return glib::ControlFlow::Break;
                }
                match receiver.try_recv() {
                    Ok(Ok(())) => {
                        status.set_label("Installation retry requested");
                        glib::ControlFlow::Break
                    }
                    Ok(Err(error)) => {
                        status.set_label(&format!("Installation could not start: {error}"));
                        button.set_sensitive(true);
                        glib::ControlFlow::Break
                    }
                    Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(_) => {
                        status.set_label("Installation retry stopped. Try again.");
                        button.set_sensitive(true);
                        glib::ControlFlow::Break
                    }
                }
            });
        });
        row.append(&retry);
    }
    if featured {
        let pause = gtk::Button::from_icon_name("media-playback-pause-symbolic");
        pause.set_tooltip_text(Some("Pause download"));
        pause.add_css_class("suggested-action");
        let job_id = job.job_id.clone();
        pause.connect_clicked(move |button| {
            if download::cancel(&job_id) {
                button.set_sensitive(false);
            }
        });
        row.append(&pause);
    } else if job.state != "complete" {
        let resume = gtk::Button::from_icon_name("media-playback-start-symbolic");
        resume.set_tooltip_text(Some(if job.state == "failed" {
            "Retry download"
        } else {
            "Resume download"
        }));
        resume.set_sensitive(model.account_token.is_some() && !job.artifacts.is_empty());
        resume.add_css_class("suggested-action");
        let job_id = job.job_id.clone();
        let retry = job.state == "failed";
        let token = model
            .account_token
            .as_ref()
            .map(|token| token.access_token.clone());
        resume.connect_clicked(move |button| {
            let Some(token) = token.clone() else {
                return;
            };
            let accepted = if retry {
                download::retry(&job_id, token)
            } else {
                download::resume(&job_id, token)
            };
            if accepted {
                button.set_sensitive(false);
            }
        });
        row.append(&resume);
    }
    let remove = gtk::Button::from_icon_name("user-trash-symbolic");
    remove.set_tooltip_text(Some(if job.state == "complete" {
        "Remove from download history"
    } else {
        "Remove download and partial files"
    }));
    let job_id = job.job_id.clone();
    let completed = job.state == "complete";
    let window = w.window.clone();
    remove.connect_clicked(move |_| {
        let confirmation = adw::AlertDialog::builder()
            .heading(if completed {
                "Remove download history?"
            } else {
                "Remove download?"
            })
            .body(if completed {
                "This removes only the history entry. Downloaded game files will remain on disk."
            } else {
                "This removes the queue entry and deletes its partial download data."
            })
            .build();
        confirmation.add_responses(&[("cancel", "Cancel"), ("remove", "Remove")]);
        confirmation.set_default_response(Some("cancel"));
        confirmation.set_close_response("cancel");
        confirmation.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
        let job_id = job_id.clone();
        confirmation.choose(Some(&window), gio::Cancellable::NONE, move |response| {
            if response == "remove" {
                download::remove(&job_id);
            }
        });
    });
    row.append(&remove);
    row.append(&folder_button(
        "Open download directory",
        &job.destination,
        &w.window,
    ));
    row
}

#[cfg(test)]
mod active_transfer_tests {
    use super::*;

    #[test]
    #[ignore = "requires isolated HOME/XDG, private GTK display and D-Bus"]
    fn footer_artwork_reuses_loaded_texture_and_retries_missing_image() {
        gtk::init().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("icon.png");
        let image = gtk::Image::new();
        update_download_artwork(&image, Some(&path));
        assert!(image.paintable().is_none());
        image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]))
            .save(&path)
            .unwrap();
        update_download_artwork(&image, Some(&path));
        let first = image.paintable().unwrap();
        for _ in 0..1000 {
            update_download_artwork(&image, Some(&path));
            assert_eq!(image.paintable().as_ref(), Some(&first));
        }
        let next = directory.path().join("second.png");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 255, 255]))
            .save(&next)
            .unwrap();
        update_download_artwork(&image, Some(&next));
        assert_ne!(image.paintable().as_ref(), Some(&first));
        assert_eq!(image.file().as_deref(), next.to_str());
        update_download_artwork(&image, None);
        assert!(!image.is_visible());
        assert!(image.paintable().is_none());
    }

    #[test]
    #[ignore = "requires isolated HOME/XDG, private GTK display and D-Bus"]
    fn progress_widget_index_preserves_depth_first_lookup_and_rebuilds() {
        gtk::init().unwrap();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_widget_name("root-only");
        let nested = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let first = gtk::Label::new(Some("first"));
        first.set_widget_name("duplicate");
        nested.append(&first);
        root.append(&nested);
        let second = gtk::Label::new(Some("second"));
        second.set_widget_name("duplicate");
        root.append(&second);
        for id in 0..1000 {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            let detail = gtk::Label::new(Some("state"));
            detail.set_widget_name(&format!("depot-detail-{id}"));
            let progress = gtk::ProgressBar::new();
            progress.set_widget_name(&format!("depot-progress-{id}"));
            row.append(&detail);
            row.append(&progress);
            root.append(&row);
        }
        let names = (0..1000)
            .flat_map(|id| [format!("depot-detail-{id}"), format!("depot-progress-{id}")])
            .collect::<Vec<_>>();
        let started = std::time::Instant::now();
        let expected = names
            .iter()
            .map(|name| find_named_descendant(root.upcast_ref(), name).unwrap())
            .collect::<Vec<_>>();
        let repeated = started.elapsed();
        let started = std::time::Instant::now();
        let named = download_progress_widgets(root.upcast_ref());
        for (name, expected) in names.iter().zip(&expected) {
            assert_eq!(named.get(name.as_str()), Some(expected));
        }
        eprintln!(
            "1000 synthetic Depot rows, 2000 lookups: repeated traversal {repeated:?}; one index plus lookups {:?}",
            started.elapsed()
        );
        assert_eq!(named.get("duplicate"), Some(first.upcast_ref()));
        assert!(!named.contains_key("root-only"));
        assert!(!named.contains_key("absent"));
        nested.remove(&first);
        let rebuilt = download_progress_widgets(root.upcast_ref());
        assert_eq!(rebuilt.get("duplicate"), Some(second.upcast_ref()));
    }

    fn job() -> DownloadJobRecord {
        DownloadJobRecord {
            job_id: "fixture".into(),
            product_id: 42,
            title: "Fixture".into(),
            artifacts: Vec::new(),
            state: DownloadState::Downloading,
            destination: "/unused".into(),
            bytes_downloaded: 0,
            total_bytes: Some(2000),
            completed_files: Vec::new(),
            error: None,
            status_message: None,
            queue_position: None,
            retry_started_at: None,
            next_retry_at: None,
            created_at: 0,
            updated_at: 0,
            completed_at: None,
        }
    }

    #[test]
    #[ignore = "private p378 HOME/all XDG/TMP, GTK and D-Bus; inert download records only"]
    fn idle_downloads_keep_compact_headings_and_readable_metrics() {
        for key in [
            "HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_RUNTIME_DIR",
            "TMPDIR",
        ] {
            assert!(
                std::env::var(key)
                    .unwrap()
                    .starts_with("/tmp/ludomere-p378-"),
                "{key}"
            );
        }
        adw::init().unwrap();
        let settings = gtk::Settings::default().unwrap();
        settings.set_gtk_enable_animations(false);
        install_css();
        #[track_caller]
        fn wait(check: impl Fn() -> bool) {
            let until = std::time::Instant::now() + Duration::from_secs(5);
            while !check() && std::time::Instant::now() < until {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        fn nodes(root: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
            let mut result = vec![root.as_ref().clone()];
            for child in
                std::iter::successors(root.as_ref().first_child(), |child| child.next_sibling())
            {
                result.extend(nodes(&child));
            }
            result
        }
        fn label(root: &impl IsA<gtk::Widget>, text: &str) -> gtk::Label {
            nodes(root)
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
                .find(|label| label.text() == text)
                .unwrap()
        }
        #[track_caller]
        fn readable(label: &gtk::Label) {
            assert!(label.is_mapped() && !label.layout().is_ellipsized());
            let (ink, logical) = label.layout().pixel_extents();
            let (x, y) = label.layout_offsets();
            assert!(
                x + logical.x() >= 0 && y + logical.y() >= 0,
                "{} text starts outside allocation",
                label.text()
            );
            assert!(
                x + logical.x() + logical.width() <= label.width(),
                "{} text exceeds width",
                label.text()
            );
            assert!(
                y + logical.y() + logical.height() <= label.height(),
                "{} text exceeds height",
                label.text()
            );
            // A visible-overflow label may draw glyph ink beyond its logical box.
            // Require that ink to remain inside every actual clipping boundary.
            for clip in
                std::iter::successors(Some(label.clone().upcast::<gtk::Widget>()), |widget| {
                    widget.parent()
                })
            {
                if clip.overflow() != gtk::Overflow::Hidden
                    && !clip.is::<gtk::Viewport>()
                    && !clip.is::<gtk::ScrolledWindow>()
                    && !clip.is::<gtk::Window>()
                {
                    continue;
                }
                let start = label
                    .compute_point(
                        &clip,
                        &gtk::graphene::Point::new((x + ink.x()) as f32, (y + ink.y()) as f32),
                    )
                    .unwrap();
                let end = label
                    .compute_point(
                        &clip,
                        &gtk::graphene::Point::new(
                            (x + ink.x() + ink.width()) as f32,
                            (y + ink.y() + ink.height()) as f32,
                        ),
                    )
                    .unwrap();
                assert!(
                    start.x() >= 0.0 && start.y() >= 0.0,
                    "{} ink starts outside clipping boundary",
                    label.text()
                );
                assert!(
                    end.x() <= clip.width() as f32 && end.y() <= clip.height() as f32,
                    "{} ink exceeds clipping boundary",
                    label.text()
                );
            }
        }
        fn heading(root: &impl IsA<gtk::Widget>, text: &str) -> gtk::Widget {
            let title = label(root, text);
            readable(&title);
            let row = title.parent().unwrap();
            let separator = title
                .next_sibling()
                .unwrap()
                .downcast::<gtk::Separator>()
                .unwrap();
            let natural = separator.measure(gtk::Orientation::Vertical, -1).1;
            assert_eq!(separator.height(), natural);
            assert!(separator.height() < title.height());
            let line = separator.compute_bounds(&row).unwrap();
            let text = title.compute_bounds(&row).unwrap();
            assert!(
                ((line.y() + line.height() / 2.0) - (text.y() + text.height() / 2.0)).abs() <= 1.0
            );
            row
        }
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.IdleDownloadsTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let widgets = window::create_widgets(
            &app,
            &Config {
                window_width: 1100,
                window_height: 700,
                ..Config::default()
            },
        );
        widgets.content.set_visible_child_name("downloads");
        widgets.window.present();
        let old_font = settings.gtk_font_name();
        let ordinary = old_font.clone().unwrap_or_else(|| "Sans 11".into());
        let mut enlarged = gtk::pango::FontDescription::from_string(&ordinary);
        enlarged.set_size((enlarged.size().max(11 * gtk::pango::SCALE) as f64 * 1.3) as i32);
        let enlarged = enlarged.to_string();
        let mut model = AppModel::default();
        let destination =
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap()).join("absent-downloads");
        let now = chrono::Utc::now().timestamp();
        for font in [ordinary.as_str(), enlarged.as_str()] {
            settings.set_gtk_font_name(Some(font));
            *model.transfer_history.borrow_mut() = [TransferHistorySample {
                download_bytes_per_second: 1_000_000.0,
                disk_bytes_per_second: 2_000_000.0,
            }]
            .into();
            model.download_jobs = [1, 3, 2]
                .into_iter()
                .map(|order| {
                    let mut record = job();
                    record.job_id = format!("history-{order}");
                    record.title = format!("Completed fixture {order}");
                    record.state = DownloadState::Complete;
                    record.updated_at = now - order;
                    record.destination = destination.clone();
                    record
                })
                .collect();
            model.account_token = None;
            rebuild_downloads_page(&widgets, &model);
            let queue_title = label(&widgets.downloads, "Up Next (0)");
            let rate = find_named_descendant(widgets.downloads.upcast_ref(), "active-disk-rate")
                .and_downcast::<gtk::Label>()
                .unwrap();
            wait(|| queue_title.is_mapped() && queue_title.height() > 0 && rate.width() > 0);
            assert_eq!(widgets.window.width(), 1100);
            let queue_heading = heading(&widgets.downloads, "Up Next (0)");
            let completed_heading = heading(&widgets.downloads, "Completed (3)");
            let compact = queue_heading
                .parent()
                .unwrap()
                .downcast::<gtk::Box>()
                .unwrap();
            assert!(compact.has_css_class("downloads-empty-queue"));
            assert_eq!(compact.spacing(), 6);
            let empty = label(&compact, "There are no downloads waiting in the queue");
            assert_eq!(empty.parent().as_ref(), Some(compact.upcast_ref()));
            assert_eq!(
                empty.measure(gtk::Orientation::Vertical, -1).1 - empty.layout().pixel_size().1,
                6
            );
            let heading_bounds = queue_heading.compute_bounds(&compact).unwrap();
            let empty_bounds = empty.compute_bounds(&compact).unwrap();
            assert!(
                (empty_bounds.y() - heading_bounds.y() - heading_bounds.height() - 6.0).abs()
                    <= 1.0
            );
            let page = widgets.downloads.first_child().unwrap();
            let ids = std::iter::successors(page.first_child(), |child| child.next_sibling())
                .map(|child| child.widget_name().to_string())
                .filter(|name| name.starts_with("history-"))
                .collect::<Vec<_>>();
            assert_eq!(ids, ["history-1", "history-2", "history-3"]);
            assert!(
                completed_heading.compute_bounds(&page).unwrap().y()
                    > compact.compute_bounds(&page).unwrap().y()
            );
            let metric = rate.parent().unwrap();
            let metric_title = metric
                .first_child()
                .unwrap()
                .downcast::<gtk::Label>()
                .unwrap();
            assert_eq!(metric_title.text(), "━ DISK WRITE RATE");
            assert_eq!(rate.text(), "0 B/s");
            readable(&metric_title);
            readable(&rate);
            let metric_bounds = metric.compute_bounds(&widgets.downloads).unwrap();
            assert!(
                metric_bounds.x() >= 0.0
                    && metric_bounds.x() + metric_bounds.width()
                        <= widgets.downloads.width() as f32
            );

            for (samples, visible) in [
                (vec![], false),
                (vec![(0.0, 0.0), (0.0, 0.0)], false),
                (vec![(1_000.0, 0.0)], true),
                (vec![(0.0, 2_000.0)], true),
                (vec![(1_000.0, 2_000.0), (0.0, 0.0)], true),
            ] {
                *model.transfer_history.borrow_mut() = samples
                    .iter()
                    .map(|&(network, disk)| TransferHistorySample {
                        download_bytes_per_second: network,
                        disk_bytes_per_second: disk,
                    })
                    .collect();
                rebuild_downloads_page(&widgets, &model);
                let queue = label(&widgets.downloads, "Up Next (0)");
                wait(|| queue.is_mapped() && queue.height() > 0);
                let headers = nodes(&widgets.downloads)
                    .into_iter()
                    .filter(|widget| widget.has_css_class("completed-transfer-history"))
                    .collect::<Vec<_>>();
                assert_eq!(headers.len(), usize::from(visible));
                let status = find_named_descendant(
                    widgets.downloads.upcast_ref(),
                    "idle-transfer-history-status",
                )
                .and_downcast::<gtk::Label>();
                assert_eq!(status.is_some(), visible);
                if let Some(header) = headers.first() {
                    assert!(header.is_mapped());
                    let status = status.unwrap();
                    assert_eq!(status.parent().as_ref(), Some(header));
                    assert_eq!(
                        status.text(),
                        "No transfers are running. Recent transfer history is shown below."
                    );
                    assert!(status.wraps());
                    readable(&status);
                    assert_eq!(status.next_sibling().unwrap().height_request(), 174);
                }
                let page = widgets.downloads.first_child().unwrap();
                assert_eq!(
                    std::iter::successors(page.first_child(), |child| child.next_sibling())
                        .map(|child| child.widget_name().to_string())
                        .filter(|name| name.starts_with("history-"))
                        .collect::<Vec<_>>(),
                    ["history-1", "history-2", "history-3"]
                );
                assert_eq!(
                    model
                        .transfer_history
                        .borrow()
                        .iter()
                        .map(|sample| (
                            sample.download_bytes_per_second,
                            sample.disk_bytes_per_second
                        ))
                        .collect::<Vec<_>>(),
                    samples
                );
            }

            model.download_jobs.clear();
            let mut queued = job();
            queued.job_id = "queued-idle-fixture".into();
            queued.state = DownloadState::Queued;
            queued.destination = destination.clone();
            model.download_jobs.push(queued);
            rebuild_downloads_page(&widgets, &model);
            let status = find_named_descendant(
                widgets.downloads.upcast_ref(),
                "idle-transfer-history-status",
            )
            .and_downcast::<gtk::Label>()
            .unwrap();
            wait(|| status.is_mapped() && status.width() > 0);
            readable(&status);
            let queue_heading = heading(&widgets.downloads, "Up Next (1)");
            assert_eq!(
                queue_heading.next_sibling().unwrap().widget_name(),
                "queued-idle-fixture"
            );
            assert!(
                !nodes(&widgets.downloads)
                    .iter()
                    .any(|widget| widget.has_css_class("downloads-empty-queue"))
            );
            model.download_jobs.clear();
            rebuild_downloads_page(&widgets, &model);
            let completed_empty = label(&widgets.downloads, "Completed downloads will appear here");
            wait(|| completed_empty.is_mapped() && completed_empty.height() > 0);
            heading(&widgets.downloads, "Up Next (0)");
            heading(&widgets.downloads, "Completed (0)");
            assert!(
                label(
                    &widgets.downloads,
                    "There are no downloads waiting in the queue"
                )
                .is_mapped()
            );
            assert_eq!(
                completed_empty.measure(gtk::Orientation::Vertical, -1).1
                    - completed_empty.layout().pixel_size().1,
                30,
                "Completed placeholder keeps its original padding"
            );

            // A second paused record is featured, leaving the first in the actual paused-card list.
            *model.transfer_history.borrow_mut() = [TransferHistorySample {
                download_bytes_per_second: 1_000.0,
                disk_bytes_per_second: 2_000.0,
            }]
            .into();
            for resumable in [false, true] {
                let mut queued = job();
                queued.job_id = "queued-fixture".into();
                queued.state = DownloadState::Queued;
                queued.destination = destination.clone();
                let mut paused = queued.clone();
                paused.job_id = "paused-fixture".into();
                paused.state = DownloadState::Paused;
                if resumable {
                    paused.artifacts.push(serde_json::from_value(serde_json::json!({"product_id": 42, "kind": "installer", "name": "inert", "download_path": "inert"})).unwrap());
                }
                let mut featured = paused.clone();
                featured.job_id = "featured-paused-fixture".into();
                model.account_token = resumable.then(|| auth::Token {
                    access_token: "inert".into(),
                    refresh_token: "inert".into(),
                    user_id: "fixture".into(),
                    expires_at: i64::MAX,
                });
                model.download_jobs = vec![queued, paused, featured];
                rebuild_downloads_page(&widgets, &model);
                let queued_title = label(&widgets.downloads, "Up Next (1)");
                wait(|| queued_title.is_mapped() && queued_title.height() > 0);
                assert!(
                    find_named_descendant(
                        widgets.downloads.upcast_ref(),
                        "idle-transfer-history-status"
                    )
                    .is_none()
                );
                let queued_heading = heading(&widgets.downloads, "Up Next (1)");
                assert_eq!(
                    queued_heading.next_sibling().unwrap().widget_name(),
                    "queued-fixture"
                );
                assert!(
                    !nodes(&widgets.downloads)
                        .iter()
                        .any(|widget| widget.has_css_class("downloads-empty-queue"))
                );
                let paused_row =
                    find_named_descendant(widgets.downloads.upcast_ref(), "paused-fixture")
                        .unwrap();
                assert_eq!(
                    find_named_descendant(&paused_row, "download-detail-paused-fixture")
                        .and_downcast::<gtk::Label>()
                        .unwrap()
                        .text(),
                    "Paused"
                );
                let resume = nodes(&paused_row)
                    .into_iter()
                    .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
                    .find(|button| button.tooltip_text().as_deref() == Some("Resume download"))
                    .unwrap();
                assert_eq!(resume.is_sensitive(), resumable);
                let featured = find_named_descendant(
                    widgets.downloads.upcast_ref(),
                    "active-download-featured-paused-fixture",
                )
                .unwrap();
                assert!(featured.is_mapped());
                assert!(
                    nodes(&featured)
                        .iter()
                        .any(|widget| widget.tooltip_text().as_deref() == Some("Resume"))
                );
                model.download_jobs.last_mut().unwrap().state = DownloadState::Failed;
                model.download_jobs.last_mut().unwrap().error =
                    Some("Synthetic retained failure".into());
                rebuild_downloads_page(&widgets, &model);
                let message = label(&widgets.downloads, "Synthetic retained failure");
                wait(|| message.is_mapped());
                assert!(
                    find_named_descendant(
                        widgets.downloads.upcast_ref(),
                        "idle-transfer-history-status"
                    )
                    .is_none()
                );
                assert!(message.is_selectable());
                let featured = find_named_descendant(
                    widgets.downloads.upcast_ref(),
                    "active-download-featured-paused-fixture",
                )
                .unwrap();
                for tooltip in ["Retry download", "Cancel permanently"] {
                    let button = nodes(&featured)
                        .into_iter()
                        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
                        .find(|button| button.tooltip_text().as_deref() == Some(tooltip))
                        .unwrap();
                    assert!(button.is_mapped());
                    assert_eq!(
                        button.is_sensitive(),
                        tooltip == "Cancel permanently" || resumable
                    );
                }
                assert!(widgets.window.visible_dialog().is_none());
            }
        }
        settings.set_gtk_font_name(old_font.as_deref());
        assert_eq!(
            widgets.content.visible_child_name().as_deref(),
            Some("downloads")
        );
        widgets.window.destroy();
    }

    #[test]
    fn active_transfer_precedes_paused_and_failed_history() {
        let active = job();
        let mut inactive = job();
        inactive.job_id = "later".into();
        let active_ids = HashSet::from([active.job_id.clone()]);
        let mut operation = crate::installation::DepotOperationSnapshot {
            setup: None,
            operation_id: "depot".into(),
            product_id: 43,
            state: "failed".into(),
            bytes_completed: 0,
            bytes_downloaded: 0,
            bytes_written: 0,
            total_write_bytes: 0,
            total_bytes: 0,
            download_total_bytes: None,
            error: None,
        };
        for state in [DownloadState::Paused, DownloadState::Failed] {
            inactive.state = state;
            let jobs = [active.clone(), inactive.clone()];
            for depot_state in ["failed", "interrupted"] {
                operation.state = depot_state.into();
                for operations in [vec![], vec![operation.clone()]] {
                    let (depot, job) = featured_downloads(&jobs, &operations, &active_ids);
                    assert!(depot.is_none());
                    assert_eq!(job.unwrap().job_id, active.job_id);
                }
            }
        }
        operation.state = "failed".into();
        let failed_depot = operation.clone();
        operation.state = "materializing".into();
        let jobs = [active, inactive.clone()];
        let operations = [operation, failed_depot.clone()];
        let (depot, job) = featured_downloads(&jobs, &operations, &active_ids);
        assert_eq!(depot.unwrap().state, "materializing");
        assert!(job.is_none());

        let operations = [failed_depot];
        let (depot, job) = featured_downloads(&jobs, &operations, &HashSet::new());
        assert_eq!(depot.unwrap().state, "failed");
        assert!(job.is_none());
        let (depot, job) = featured_downloads(&jobs, &[], &HashSet::new());
        assert!(depot.is_none());
        assert_eq!(job.unwrap().job_id, inactive.job_id);
        inactive.state = DownloadState::Complete;
        let jobs = [inactive];
        let (depot, job) = featured_downloads(&jobs, &[], &HashSet::new());
        assert!(depot.is_none() && job.is_none());
    }

    #[test]
    fn changed_failure_reason_refreshes_download_presentation() {
        let mut old = job();
        old.state = DownloadState::Failed;
        old.error = Some("First failure".into());
        let mut new = old.clone();
        new.error = Some("Updated failure".into());
        assert!(download_job_structure_changed(&[old], &[new]));
    }

    #[test]
    #[ignore = "requires isolated HOME/all XDG, private GTK display and D-Bus"]
    fn long_featured_errors_scroll_without_hiding_actions() {
        for key in [
            "HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_RUNTIME_DIR",
            "TMPDIR",
        ] {
            assert!(
                std::env::var(key)
                    .unwrap()
                    .starts_with("/tmp/ludomere-p333-")
            );
        }
        adw::init().unwrap();
        gtk::Settings::default()
            .unwrap()
            .set_gtk_enable_animations(false);
        install_css();
        fn wait(check: impl Fn() -> bool) {
            let until = std::time::Instant::now() + Duration::from_secs(5);
            while !check() && std::time::Instant::now() < until {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.DownloadErrorLayoutTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let widgets = window::create_widgets(
            &app,
            &Config {
                window_width: 1280,
                window_height: 600,
                ..Config::default()
            },
        );
        widgets.content.set_visible_child_name("downloads");
        widgets.window.present();
        let outer = widgets
            .downloads
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast::<gtk::ScrolledWindow>()
            .unwrap();
        let mut model = AppModel {
            account_token: Some(auth::Token {
                access_token: "inert-fixture-token".into(),
                refresh_token: "inert-fixture-refresh".into(),
                user_id: "fixture".into(),
                expires_at: i64::MAX,
            }),
            ..AppModel::default()
        };
        let long_error = format!(
            "Setup could not finish.\n{}\nFinal diagnostic line: restore access and retry.",
            "Synthetic error with a long unbroken path-like-token/".repeat(300)
        );
        for depot in [false, true] {
            for error in [long_error.as_str(), "Restore access and retry."] {
                model.download_jobs.clear();
                model.depot_operations.clear();
                if depot {
                    model
                        .depot_operations
                        .push(crate::installation::DepotOperationSnapshot {
                            setup: None,
                            operation_id: "layout-fixture".into(),
                            product_id: 43,
                            state: "failed".into(),
                            bytes_completed: 10,
                            bytes_downloaded: 10,
                            bytes_written: 10,
                            total_write_bytes: 100,
                            total_bytes: 100,
                            download_total_bytes: Some(100),
                            error: Some(error.into()),
                        });
                } else {
                    let mut failed = job();
                    failed.state = DownloadState::Failed;
                    failed.error = Some(error.into());
                    model.download_jobs.push(failed);
                }
                rebuild_downloads_page(&widgets, &model);
                let header = find_named_descendant(
                    widgets.downloads.upcast_ref(),
                    if depot {
                        "active-depot-layout-fixture"
                    } else {
                        "active-download-fixture"
                    },
                )
                .unwrap();
                let message = find_named_descendant(
                    &header,
                    if depot {
                        "active-depot-message"
                    } else {
                        "active-download-message"
                    },
                )
                .and_downcast::<gtk::Label>()
                .unwrap();
                let scroll = message
                    .ancestor(gtk::ScrolledWindow::static_type())
                    .and_downcast::<gtk::ScrolledWindow>()
                    .unwrap();
                let footer = header.last_child().unwrap().last_child().unwrap();
                let cancel = footer
                    .last_child()
                    .unwrap()
                    .downcast::<gtk::Button>()
                    .unwrap();
                let retry = cancel
                    .prev_sibling()
                    .unwrap()
                    .downcast::<gtk::Button>()
                    .unwrap();
                wait(|| {
                    cancel.is_mapped()
                        && header.height() > 0
                        && scroll.vadjustment().page_size() > 0.0
                });
                outer.vadjustment().set_value(0.0);
                update_depot_page_progress(&widgets, &model);
                eprintln!(
                    "depot={depot}, error bytes={}, header={}px, error viewport={}px, window={}x{}",
                    error.len(),
                    header.height(),
                    scroll.height(),
                    widgets.window.width(),
                    widgets.window.height()
                );
                assert_eq!(message.text(), error);
                assert!(message.is_selectable());
                assert_eq!(message.ellipsize(), gtk::pango::EllipsizeMode::None);
                assert_ne!(scroll, outer);
                assert!(scroll.height() <= 120);
                assert!(
                    header.height() <= 400,
                    "Header expanded to {}px",
                    header.height()
                );
                assert!(widgets.window.height() <= 600);
                assert_eq!(
                    retry.tooltip_text().as_deref(),
                    Some(if depot { "Resume" } else { "Retry download" })
                );
                assert_eq!(cancel.tooltip_text().as_deref(), Some("Cancel permanently"));
                let bounds = [retry.clone(), cancel.clone()].map(|button| {
                    assert!(button.is_sensitive() && button.is_mapped());
                    let bounds = button.compute_bounds(&outer).unwrap();
                    assert!(bounds.x() >= 0.0 && bounds.y() >= 0.0);
                    assert!(bounds.x() + bounds.width() <= outer.width() as f32);
                    assert!(bounds.y() + bounds.height() <= outer.height() as f32);
                    bounds
                });
                if error == long_error.as_str() {
                    assert!(scroll.vadjustment().upper() > scroll.vadjustment().page_size());
                    scroll.vadjustment().set_value(scroll.vadjustment().upper());
                    wait(|| {
                        message
                            .compute_bounds(&scroll)
                            .is_some_and(|bounds| bounds.y() < 0.0)
                    });
                    assert!(scroll.vadjustment().value() > 0.0);
                    assert_eq!(retry.compute_bounds(&outer).unwrap(), bounds[0]);
                    assert_eq!(cancel.compute_bounds(&outer).unwrap(), bounds[1]);
                    assert_eq!(outer.vadjustment().value(), 0.0);
                } else {
                    assert!(scroll.height() < 120);
                }
                assert_eq!(
                    widgets.content.visible_child_name().as_deref(),
                    Some("downloads")
                );
                assert!(widgets.window.visible_dialog().is_none());
            }
        }
        widgets.window.destroy();
    }

    #[test]
    #[ignore = "requires isolated HOME/all XDG, private GTK display and D-Bus"]
    fn featured_archive_keeps_full_error_and_retry_without_navigation() {
        assert!(
            std::env::var("HOME")
                .unwrap()
                .starts_with("/tmp/ludomere-p326-")
        );
        adw::init().unwrap();
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.DownloadFailureTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let widgets = window::create_widgets(&app, &Config::default());
        let visible_page = widgets.content.visible_child_name();
        let mut failed = job();
        failed.state = DownloadState::Failed;
        failed.error = Some(format!(
            "The downloaded archive could not be registered.\n{}\nRetry after restoring access.",
            "Synthetic diagnostic detail ".repeat(24)
        ));
        let mut older = failed.clone();
        older.job_id = "older-failure".into();
        let mut model = AppModel {
            download_jobs: vec![older, failed.clone()],
            ..AppModel::default()
        };
        rebuild_downloads_page(&widgets, &model);
        let header = find_named_descendant(
            widgets.downloads.upcast_ref(),
            &format!("active-download-{}", failed.job_id),
        )
        .unwrap();
        let message = find_named_descendant(&header, "active-download-message")
            .and_downcast::<gtk::Label>()
            .unwrap();
        update_depot_page_progress(&widgets, &model);
        assert_eq!(message.text().as_str(), failed.error.as_deref().unwrap());
        assert!(message.wraps() && message.is_selectable());
        assert_eq!(message.ellipsize(), gtk::pango::EllipsizeMode::None);
        assert!(
            download_progress_widgets(&header)
                .values()
                .any(|widget| { widget.tooltip_text().as_deref() == Some("Retry download") })
        );
        assert!(
            find_named_descendant(
                widgets.downloads.upcast_ref(),
                "download-detail-older-failure"
            )
            .is_some()
        );
        assert!(
            find_named_descendant(widgets.downloads.upcast_ref(), "download-detail-fixture")
                .is_none()
        );

        model.download_jobs = vec![failed];
        model.download_jobs[0].error = None;
        model.download_jobs[0].status_message = Some("Registration needs retry".into());
        rebuild_downloads_page(&widgets, &model);
        let message =
            find_named_descendant(widgets.downloads.upcast_ref(), "active-download-message")
                .and_downcast::<gtk::Label>()
                .unwrap();
        assert_eq!(message.text(), "Registration needs retry");
        model.download_jobs[0].state = DownloadState::Paused;
        model.download_jobs[0].status_message = None;
        rebuild_downloads_page(&widgets, &model);
        assert!(
            find_named_descendant(widgets.downloads.upcast_ref(), "active-download-message")
                .is_none()
        );
        assert!(
            download_progress_widgets(widgets.downloads.upcast_ref())
                .values()
                .any(|widget| widget.tooltip_text().as_deref() == Some("Resume"))
        );
        assert_eq!(widgets.content.visible_child_name(), visible_page);
        assert!(widgets.window.visible_dialog().is_none());
        widgets.window.destroy();
    }

    #[test]
    #[ignore = "requires isolated HOME/all XDG, private GTK display and D-Bus"]
    fn depot_failure_details_survive_progress_updates() {
        assert!(
            std::env::var("HOME")
                .unwrap()
                .starts_with("/tmp/ludomere-p326-")
        );
        adw::init().unwrap();
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.DepotFailureTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let widgets = window::create_widgets(&app, &Config::default());
        let visible_page = widgets.content.visible_child_name();
        let failed = crate::installation::DepotOperationSnapshot {
            setup: None,
            operation_id: "featured-failure".into(),
            product_id: 43,
            state: "failed".into(),
            bytes_completed: 10,
            bytes_downloaded: 10,
            bytes_written: 10,
            total_write_bytes: 100,
            total_bytes: 100,
            download_total_bytes: Some(100),
            error: Some("Synthetic Depot error\nRestore access and retry.".into()),
        };
        let mut older = failed.clone();
        older.operation_id = "older-failure".into();
        let model = AppModel {
            depot_operations: vec![older, failed.clone()],
            ..AppModel::default()
        };
        rebuild_downloads_page(&widgets, &model);
        for name in ["active-depot-message", "depot-detail-older-failure"] {
            let message = find_named_descendant(widgets.downloads.upcast_ref(), name)
                .and_downcast::<gtk::Label>()
                .unwrap();
            for _ in 0..3 {
                update_depot_page_progress(&widgets, &model);
                assert_eq!(message.text().as_str(), failed.error.as_deref().unwrap());
            }
            assert!(message.wraps() && message.is_selectable());
            assert_eq!(message.ellipsize(), gtk::pango::EllipsizeMode::None);
        }
        assert_eq!(widgets.content.visible_child_name(), visible_page);
        assert!(widgets.window.visible_dialog().is_none());
        widgets.window.destroy();
    }

    #[test]
    #[ignore = "requires isolated HOME/all XDG, private GTK display and D-Bus"]
    fn depot_cancel_requires_consent_and_keeps_progress_and_errors_visible() {
        for key in [
            "HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_RUNTIME_DIR",
            "TMPDIR",
        ] {
            assert!(
                std::env::var(key)
                    .unwrap()
                    .starts_with("/tmp/ludomere-p327-")
            );
        }
        adw::init().unwrap();
        gtk::Settings::default()
            .unwrap()
            .set_gtk_enable_animations(false);
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.DepotCancellationTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let widgets = window::create_widgets(&app, &Config::default());
        widgets.window.present();
        let visible_page = widgets.content.visible_child_name();
        let count = Rc::new(std::cell::Cell::new(0));
        let accepted = Rc::new(std::cell::Cell::new(true));
        let button = gtk::Button::with_label("Cancel permanently");
        widgets.downloads.append(&button);
        let requests = count.clone();
        let allowed = accepted.clone();
        connect_depot_cancellation(&button, &widgets.window, "fixture".into(), move |id| {
            assert_eq!(id, "fixture");
            requests.set(requests.get() + 1);
            allowed.get()
        });
        let drain = || {
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
        };
        fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
            let mut result = vec![widget.clone()];
            let mut child = widget.first_child();
            while let Some(widget) = child {
                result.extend(descendants(&widget));
                child = widget.next_sibling();
            }
            result
        }
        fn respond(dialog: &adw::AlertDialog, label: &str) {
            let button = descendants(dialog.upcast_ref())
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
                .find(|button| button.label().as_deref() == Some(label))
                .unwrap_or_else(|| panic!("Missing dialog response button: {label}"));
            assert!(button.is_sensitive());
            button.emit_clicked();
        }
        for close in [false, true] {
            button.emit_clicked();
            drain();
            let dialog = widgets
                .window
                .visible_dialog()
                .unwrap()
                .downcast::<adw::AlertDialog>()
                .unwrap();
            assert_eq!(dialog.default_response().as_deref(), Some("keep"));
            assert_eq!(dialog.close_response(), "keep");
            assert!(dialog.body().contains("not rolled back"));
            assert_eq!(count.get(), 0);
            if close {
                dialog.close();
            } else {
                respond(&dialog, "Keep");
            }
            drain();
            assert_eq!(count.get(), 0);
            assert!(button.is_sensitive());
        }
        button.emit_clicked();
        drain();
        respond(
            &widgets
                .window
                .visible_dialog()
                .unwrap()
                .downcast::<adw::AlertDialog>()
                .unwrap(),
            "Cancel Installation",
        );
        drain();
        assert_eq!(count.get(), 1);
        assert!(!button.is_sensitive());
        button.set_sensitive(true);
        accepted.set(false);
        button.emit_clicked();
        drain();
        respond(
            &widgets
                .window
                .visible_dialog()
                .unwrap()
                .downcast::<adw::AlertDialog>()
                .unwrap(),
            "Cancel Installation",
        );
        drain();
        assert_eq!(count.get(), 2);
        let error = widgets
            .window
            .visible_dialog()
            .unwrap()
            .downcast::<adw::AlertDialog>()
            .unwrap();
        assert_eq!(error.heading().as_deref(), Some("Cancellation unavailable"));
        assert!(error.body().contains("busy") && error.body().contains("retry"));
        assert!(button.is_sensitive());
        respond(&error, "Close");
        drain();

        let mut model = AppModel {
            depot_operations: vec![crate::installation::DepotOperationSnapshot {
                setup: None,
                operation_id: "fixture".into(),
                product_id: 43,
                state: "cancelling".into(),
                bytes_completed: 0,
                bytes_downloaded: 0,
                bytes_written: 0,
                total_write_bytes: 0,
                total_bytes: 0,
                download_total_bytes: None,
                error: None,
            }],
            ..AppModel::default()
        };
        rebuild_downloads_page(&widgets, &model);
        update_depot_page_progress(&widgets, &model);
        let header =
            find_named_descendant(widgets.downloads.upcast_ref(), "active-depot-fixture").unwrap();
        let busy = descendants(&header);
        assert!(busy.iter().any(|widget| {
            widget
                .downcast_ref::<gtk::Label>()
                .is_some_and(|label| label.text().contains("Cancelling"))
        }));
        assert!(!busy.iter().any(|widget| matches!(
            widget.tooltip_text().as_deref(),
            Some("Resume" | "Pause" | "Cancel permanently")
        )));
        assert!(widgets.window.visible_dialog().is_none());
        model.depot_operations[0].state = "failed".into();
        model.depot_operations[0].error =
            Some("Cancellation cleanup failed: protected temporary path is a directory".into());
        rebuild_downloads_page(&widgets, &model);
        update_depot_page_progress(&widgets, &model);
        let message = find_named_descendant(widgets.downloads.upcast_ref(), "active-depot-message")
            .and_downcast::<gtk::Label>()
            .unwrap();
        assert_eq!(
            message.text().as_str(),
            model.depot_operations[0].error.as_deref().unwrap()
        );
        assert!(
            descendants(widgets.downloads.upcast_ref())
                .iter()
                .any(
                    |widget| widget.tooltip_text().as_deref() == Some("Cancel permanently")
                        && widget.is_sensitive()
                )
        );
        assert_eq!(widgets.content.visible_child_name(), visible_page);
        assert!(widgets.window.visible_dialog().is_none());
        widgets.window.destroy();
    }

    #[test]
    fn progress_burst_needs_no_inventory_read_and_retains_terminal_products() {
        let mut jobs = vec![job()];
        let mut changed = HashSet::new();
        let mut failures = HashMap::new();
        let events = (1..=1000).map(|downloaded| download::DownloadManagerEvent::Progress {
            job_id: "fixture".into(),
            downloaded,
            total: Some(2000),
        });
        let (read_inventory, error) =
            coalesce_download_events(&mut jobs, &mut changed, &mut failures, events);
        assert!(!read_inventory);
        assert!(error.is_none());
        assert_eq!(jobs[0].bytes_downloaded, 1000);
        let events = [
            download::DownloadManagerEvent::ManagedFilesChanged(42),
            download::DownloadManagerEvent::ManagedFilesChanged(43),
            download::DownloadManagerEvent::Progress {
                job_id: "fixture".into(),
                downloaded: 1500,
                total: Some(2000),
            },
        ];
        assert!(coalesce_download_events(&mut jobs, &mut changed, &mut failures, events).0);
        assert_eq!(changed, HashSet::from([42, 43]));
        assert_eq!(jobs[0].bytes_downloaded, 1500);
    }

    #[test]
    fn registration_failure_survives_stale_database_snapshot_until_retry() {
        let mut jobs = vec![job()];
        let mut changed = HashSet::new();
        let mut failures = HashMap::new();
        coalesce_download_events(
            &mut jobs,
            &mut changed,
            &mut failures,
            [download::DownloadManagerEvent::BookkeepingFailed {
                session: online::account_session(),
                job_id: "fixture".into(),
                product_id: 42,
                message: "Registration failed; retry".into(),
            }],
        );
        coalesce_download_events(
            &mut jobs,
            &mut changed,
            &mut failures,
            [download::DownloadManagerEvent::QueueSnapshot(vec![job()])],
        );
        let mut presented = jobs.clone();
        apply_registration_failures(&mut presented, &failures);
        assert_eq!(presented[0].state, DownloadState::Failed);
        assert_eq!(
            presented[0].error.as_deref(),
            Some("Registration failed; retry")
        );
        assert!(jobs[0].error.is_none());
        assert!(changed.is_empty());
        let mut retry = job();
        retry.state = DownloadState::Queued;
        coalesce_download_events(
            &mut jobs,
            &mut changed,
            &mut failures,
            [download::DownloadManagerEvent::QueueSnapshot(vec![retry])],
        );
        let mut presented = jobs.clone();
        apply_registration_failures(&mut presented, &failures);
        assert_eq!(presented[0].state, DownloadState::Failed);
        coalesce_download_events(
            &mut jobs,
            &mut changed,
            &mut failures,
            [download::DownloadManagerEvent::Progress {
                job_id: "fixture".into(),
                downloaded: 1500,
                total: Some(2000),
            }],
        );
        assert!(failures.is_empty());
        assert_eq!(jobs[0].state, DownloadState::Downloading);
    }

    fn counter_operation() -> crate::installation::DepotOperationSnapshot {
        crate::installation::DepotOperationSnapshot {
            operation_id: "counter-fixture".into(),
            product_id: 42,
            state: "materializing".into(),
            bytes_completed: 40,
            total_bytes: 100,
            bytes_downloaded: 10,
            download_total_bytes: Some(100),
            bytes_written: 0,
            total_write_bytes: 200,
            error: Some("Synthetic diagnostic remains available.".into()),
            setup: None,
        }
    }

    #[test]
    fn depot_counter_presentation_preserves_actuals_and_phase_meaning() {
        let mut operation = counter_operation();
        for (written, estimate) in [(0, 200), (50, 200), (300, 200), (50, 0)] {
            operation.bytes_written = written;
            operation.total_write_bytes = estimate;
            let (download, disk) = depot_counter_presentation(&operation);
            assert_eq!(download.detail, "10 B / 100 B");
            assert_eq!(download.meter, DepotMeter::Fraction(0.1));
            assert_eq!(disk.label, "Writing game files");
            assert_eq!(disk.meter, DepotMeter::Pulse);
            assert_eq!(
                disk.detail,
                format!(
                    "Payload data written this run: {written} B{}",
                    if estimate == 0 {
                        String::new()
                    } else {
                        format!(" · Full write estimate: {estimate} B")
                    }
                )
            );
        }
        operation.download_total_bytes = Some(0);
        operation.bytes_downloaded = 0;
        let (download, _) = depot_counter_presentation(&operation);
        assert_eq!(download.label, "No Depot file download required");
        assert_eq!(download.detail, "");
        assert_eq!(download.meter, DepotMeter::Hidden);
        operation.bytes_downloaded = 10;
        let (download, _) = depot_counter_presentation(&operation);
        assert_eq!(download.label, "Depot files downloaded");
        assert_eq!(download.detail, "10 B");
        operation.download_total_bytes = None;
        assert_eq!(
            depot_counter_presentation(&operation).0.label,
            "Data processed"
        );
        operation.bytes_downloaded = 0;
        assert_eq!(
            depot_counter_presentation(&operation).0.detail,
            "Not reported yet"
        );
        for phase in [
            "dependencies",
            "verifying",
            "verifying_existing",
            "extracting",
        ] {
            operation.state = phase.into();
            operation.bytes_downloaded = 40; // Also includes cached component acquisition.
            let (download, disk) = depot_counter_presentation(&operation);
            assert_eq!(download.detail, "40 B / 100 B");
            assert_eq!(download.meter, DepotMeter::Fraction(0.4));
            assert_eq!(
                disk.meter,
                if phase == "extracting" {
                    DepotMeter::Pulse
                } else {
                    DepotMeter::Hidden
                }
            );
            if phase == "dependencies" {
                assert_eq!(download.label, "Component data processed");
                assert_eq!(depot_stage_label(phase), "PROCESSING REQUIRED COMPONENTS");
            }
        }
        operation.setup = Some(crate::installation::DepotSetupProgress {
            component: "Fixture component".into(),
            completed: 1,
            total: 3,
        });
        for phase in ["setup", "committing", "finalizing"] {
            operation.state = phase.into();
            let (download, disk) = depot_counter_presentation(&operation);
            assert_eq!(download.meter, DepotMeter::Hidden);
            assert_eq!(disk.label, "Finishing installation");
            assert_eq!(disk.detail, "Fixture component · 1 / 3");
            assert_eq!(disk.meter, DepotMeter::Pulse);
        }
        operation.setup = None;
        for phase in [
            "queued",
            "preparing",
            "calculating",
            "downloading",
            "interrupted",
            "failed",
            "paused",
            "cancelled",
            "abandoned",
            "complete",
            "cancelling",
        ] {
            operation.state = phase.into();
            let (download, disk) = depot_counter_presentation(&operation);
            assert_eq!(disk.label, "Payload writes");
            assert_eq!(disk.meter, DepotMeter::Hidden);
            if !depot_active(phase) || phase == "cancelling" {
                assert_eq!(download.meter, DepotMeter::Hidden);
            }
            if matches!(phase, "failed" | "interrupted") {
                assert_eq!(download.label, "Data processed");
                assert_eq!(download.detail, "40 B");
            }
        }
    }

    #[test]
    #[ignore = "private HOME/all XDG/TMP, GTK and D-Bus; inert snapshots only"]
    fn depot_featured_counter_presentation_matches_in_place_updates() {
        for key in [
            "HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_RUNTIME_DIR",
            "TMPDIR",
        ] {
            assert!(
                std::env::var(key)
                    .unwrap()
                    .starts_with("/tmp/ludomere-p374-"),
                "{key}"
            );
        }
        adw::init().unwrap();
        gtk::Settings::default()
            .unwrap()
            .set_gtk_enable_animations(false);
        install_css();
        #[track_caller]
        fn wait(check: impl Fn() -> bool) {
            let until = std::time::Instant::now() + Duration::from_secs(5);
            while !check() && std::time::Instant::now() < until {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        fn assert_counters(
            header: &gtk::Widget,
            operation: &crate::installation::DepotOperationSnapshot,
        ) {
            let (download, disk) = depot_counter_presentation(operation);
            for (prefix, expected) in [("active-download", download), ("active-disk", disk)] {
                let label = find_named_descendant(header, &format!("{prefix}-label"))
                    .and_downcast::<gtk::Label>()
                    .unwrap();
                let detail = find_named_descendant(header, &format!("{prefix}-detail"))
                    .and_downcast::<gtk::Label>()
                    .unwrap();
                let progress = find_named_descendant(header, &format!("{prefix}-progress"))
                    .and_downcast::<gtk::ProgressBar>()
                    .unwrap();
                assert_eq!(label.text(), expected.label);
                assert_eq!(detail.text(), expected.detail);
                assert_eq!(
                    detail.tooltip_text().as_deref(),
                    Some(expected.detail.as_str())
                );
                assert_eq!(progress.get_visible(), expected.meter != DepotMeter::Hidden);
                match expected.meter {
                    DepotMeter::Fraction(fraction) => assert_eq!(progress.fraction(), fraction),
                    DepotMeter::Hidden => assert_eq!(progress.fraction(), 0.0),
                    DepotMeter::Pulse => {}
                }
            }
        }
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.DepotCountersTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let widgets = window::create_widgets(
            &app,
            &Config {
                window_width: 1100,
                window_height: 600,
                ..Config::default()
            },
        );
        widgets.content.set_visible_child_name("downloads");
        widgets.window.present();
        let outer = widgets
            .downloads
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast::<gtk::ScrolledWindow>()
            .unwrap();
        let mut model = AppModel {
            depot_operations: vec![counter_operation()],
            ..AppModel::default()
        };
        for phase in [
            "materializing",
            "dependencies",
            "verifying_existing",
            "extracting",
            "setup",
            "interrupted",
            "failed",
        ] {
            model.depot_operations[0].state = phase.into();
            model.depot_operations[0].total_write_bytes = u64::MAX;
            model.depot_operations[0].setup =
                (phase == "setup").then(|| crate::installation::DepotSetupProgress {
                    component: "Fixture component".into(),
                    completed: 1,
                    total: 3,
                });
            rebuild_downloads_page(&widgets, &model);
            let header = find_named_descendant(
                widgets.downloads.upcast_ref(),
                "active-depot-counter-fixture",
            )
            .unwrap();
            let footer = header.last_child().unwrap().last_child().unwrap();
            let buttons = std::iter::successors(footer.first_child(), |child| child.next_sibling())
                .filter_map(|child| child.downcast::<gtk::Button>().ok())
                .collect::<Vec<_>>();
            assert_eq!(
                buttons
                    .iter()
                    .map(|button| button.tooltip_text().unwrap().to_string())
                    .collect::<Vec<_>>(),
                if depot_active(phase) {
                    vec!["Pause"]
                } else {
                    vec!["Resume", "Cancel permanently"]
                }
            );
            wait(|| {
                buttons
                    .iter()
                    .all(|button| button.is_mapped() && button.width() > 0)
                    && outer.height() > 0
            });
            assert_counters(&header, &model.depot_operations[0]);
            update_depot_page_progress(&widgets, &model);
            assert_counters(&header, &model.depot_operations[0]);
            for button in &buttons {
                let bounds = button.compute_bounds(&outer).unwrap();
                assert!(bounds.x() >= 0.0 && bounds.y() >= 0.0);
                assert!(bounds.x() + bounds.width() <= outer.width() as f32);
                assert!(bounds.y() + bounds.height() <= outer.height() as f32);
            }
            // Actual updater must retire activity on this same header, before any rebuild.
            for next in ["materializing", "setup", "interrupted", "failed"] {
                model.depot_operations[0].state = next.into();
                update_depot_page_progress(&widgets, &model);
                assert_counters(&header, &model.depot_operations[0]);
                assert_eq!(
                    find_named_descendant(
                        widgets.downloads.upcast_ref(),
                        "active-depot-counter-fixture"
                    )
                    .as_ref(),
                    Some(&header)
                );
            }
            let diagnostic = find_named_descendant(&header, "active-depot-message")
                .and_downcast::<gtk::Label>()
                .unwrap();
            assert_eq!(diagnostic.text(), "Synthetic diagnostic remains available.");
            assert!(diagnostic.is_selectable());
            assert!(buttons.iter().all(|button| button.is_mapped()));
        }
        model.depot_operations[0].state = "complete".into();
        rebuild_downloads_page(&widgets, &model);
        assert!(
            find_named_descendant(
                widgets.downloads.upcast_ref(),
                "active-depot-counter-fixture"
            )
            .is_none()
        );
        assert!(
            find_named_descendant(
                widgets.downloads.upcast_ref(),
                "depot-detail-counter-fixture"
            )
            .is_some()
        );
        assert_eq!(
            widgets.content.visible_child_name().as_deref(),
            Some("downloads")
        );
        assert!(widgets.window.visible_dialog().is_none());
        widgets.window.destroy();
    }

    #[test]
    fn depot_local_phases_use_processing_counts_instead_of_stalled_network_bytes() {
        let mut operation = crate::installation::DepotOperationSnapshot {
            operation_id: "progress-test".into(),
            product_id: 42,
            state: "extracting".into(),
            bytes_completed: 400,
            total_bytes: 1000,
            bytes_downloaded: 10,
            download_total_bytes: Some(100),
            bytes_written: 400,
            total_write_bytes: 2000,
            error: None,
            setup: None,
        };
        assert_eq!(
            depot_phase_progress(&operation),
            ("Extracting game files", 400, Some(1000))
        );
        assert!(depot_active("extracting"));
        for phase in ["verifying", "verifying_existing", "dependencies"] {
            operation.state = phase.into();
            let (_, completed, total) = depot_phase_progress(&operation);
            assert_eq!((completed, total), (400, Some(1000)));
        }
        operation.total_bytes = 0;
        assert_eq!(depot_phase_progress(&operation).2, None);
        operation.state = "materializing".into();
        assert_eq!(
            depot_phase_progress(&operation),
            ("Downloading data", 10, Some(100))
        );
    }

    #[test]
    fn depot_download_progress_uses_persisted_completed_bytes() {
        assert_eq!(depot_download_fraction(52, 100), 0.52);
        assert_eq!(depot_download_fraction(0, 0), 0.0);
        assert_eq!(depot_download_fraction(120, 100), 1.0);
    }

    #[test]
    fn remaining_time_uses_readable_units() {
        assert_eq!(format_remaining(45), "About 45 sec remaining");
        assert_eq!(format_remaining(125), "About 2 min remaining");
        assert_eq!(format_remaining(3_720), "About 1 hr 2 min remaining");
    }
}

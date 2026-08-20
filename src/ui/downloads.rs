use super::*;

pub(super) fn start_download_monitor(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>) {
    struct Snapshot {
        jobs: Vec<DownloadJobRecord>,
        downloaded_products: HashSet<i64>,
        downloaded_installer_products: HashSet<i64>,
        active_job_ids: HashSet<String>,
        managed_files_changed: Option<i64>,
    }

    let (sender, receiver) = mpsc::channel();
    let (depot_sender, depot_receiver) = mpsc::channel();
    let manager_events = download::manager_events();
    std::thread::spawn(move || {
        let mut jobs = StateStore::open()
            .and_then(|store| store.download_jobs())
            .unwrap_or_default();
        while let Ok(event) = manager_events.recv() {
            let mut managed_files_changed = None;
            match event {
                download::DownloadManagerEvent::QueueSnapshot(snapshot) => jobs = snapshot,
                download::DownloadManagerEvent::Progress {
                    job_id,
                    downloaded,
                    total,
                } => {
                    if let Some(job) = jobs.iter_mut().find(|job| job.job_id == job_id) {
                        job.bytes_downloaded = downloaded;
                        job.total_bytes = total.or(job.total_bytes);
                        job.state = DownloadState::Downloading;
                    }
                }
                download::DownloadManagerEvent::ManagedFilesChanged(product_id) => {
                    tracing::debug!(product_id, "managed download files changed");
                    managed_files_changed = Some(product_id);
                }
                download::DownloadManagerEvent::AuthenticationRequired => {}
            }
            let snapshot = Snapshot {
                downloaded_products: downloaded_product_ids(&jobs),
                downloaded_installer_products: downloaded_installer_product_ids(&jobs),
                active_job_ids: jobs
                    .iter()
                    .filter(|job| download::is_active(&job.job_id))
                    .map(|job| job.job_id.clone())
                    .collect(),
                managed_files_changed,
                jobs: jobs.clone(),
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
        while let Ok(snapshot) = receiver.try_recv() {
            if let Some(product_id) = snapshot.managed_files_changed {
                managed_file_changes.insert(product_id);
            }
            latest = Some(snapshot);
        }
        let mut depot_changed = false;
        {
            let mut state = model.borrow_mut();
            while let Ok(snapshot) = depot_receiver.try_recv() {
                if let Some(current) = state
                    .depot_operations
                    .iter_mut()
                    .find(|current| current.operation_id == snapshot.operation_id)
                {
                    depot_changed |= current.state != snapshot.state;
                    *current = snapshot;
                } else {
                    state.depot_operations.push(snapshot);
                    depot_changed = true;
                }
            }
            sample_transfer_history(&mut state);
        }
        let Some(snapshot) = latest else {
            if depot_changed && w.content.visible_child_name().as_deref() == Some("downloads") {
                rebuild_downloads_page(&w, &model.borrow());
            } else if w.content.visible_child_name().as_deref() == Some("downloads") {
                update_depot_page_progress(&w, &model.borrow());
            }
            return glib::ControlFlow::Continue;
        };
        let (should_refresh_filters, should_refresh_sidebar, jobs_changed) = {
            let mut state = model.borrow_mut();
            let products_changed = state.downloaded_products != snapshot.downloaded_products;
            let installers_changed =
                state.downloaded_installer_products != snapshot.downloaded_installer_products;
            let jobs_changed = download_job_structure_changed(&state.download_jobs, &snapshot.jobs);
            if products_changed {
                state.downloaded_products = snapshot.downloaded_products;
            }
            if installers_changed {
                state.downloaded_installer_products = snapshot.downloaded_installer_products;
            }
            state.download_jobs = snapshot.jobs;
            (
                products_changed && state.downloaded_only,
                installers_changed,
                jobs_changed,
            )
        };
        if should_refresh_filters {
            refresh_filters(&w, &model.borrow());
        }
        if should_refresh_sidebar {
            update_sidebar_download_styles(&w, &model.borrow());
        }
        if !managed_file_changes.is_empty() {
            let affected_games = owning_game_ids(&model.borrow(), &managed_file_changes);
            // Installation/update availability is derived from the managed-file
            // index, so refresh even when the downloaded-installer set itself did
            // not change (for example, replacing an invalid part or downloading a
            // patch/DLC for a game that already has another installer).
            update_sidebar_download_styles(&w, &model.borrow());
            refresh_selected_game_after_managed_change(&w, &model, &affected_games);
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
            w.status.set_label(&if finalizing {
                format!("Finalizing {title}")
            } else {
                title.to_owned()
            });
            w.download_percent
                .set_label(&percent.map(|value| format!("{value}%")).unwrap_or_default());
            w.download_percent.set_visible(percent.is_some());
            if let Some(path) = display.and_then(|(_, icon)| icon) {
                w.download_artwork.set_from_file(Some(path));
                w.download_artwork.set_visible(true);
            } else {
                w.download_artwork.clear();
                w.download_artwork.set_visible(false);
            }
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
                w.status
                    .set_label(&format!("Downloads waiting  ·  {blocking}"));
                previously_active.set(false);
            } else if previously_active.replace(false) {
                w.status.set_label("Downloads complete");
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

fn owning_game_ids(model: &AppModel, changed_products: &HashSet<i64>) -> HashSet<i64> {
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

fn refresh_selected_game_after_managed_change(
    w: &Widgets,
    model: &Rc<RefCell<AppModel>>,
    affected_games: &HashSet<i64>,
) {
    let Some(selected) = model.borrow().selected else {
        return;
    };
    if !affected_games.contains(&selected)
        || w.content.visible_child_name().as_deref() != Some("details")
    {
        return;
    }

    let root: gtk::Widget = w.details.clone().upcast();
    let visible_tab = find_named_descendant(&root, "game-tabs")
        .and_downcast::<gtk::Stack>()
        .and_then(|stack| stack.visible_child_name())
        .map(|name| name.to_string());
    let scroll_position = w.details_scroll.vadjustment().value();

    show_game(w, model, selected, Some(false));

    if let Some(visible_tab) = visible_tab {
        let root: gtk::Widget = w.details.clone().upcast();
        if let Some(stack) = find_named_descendant(&root, "game-tabs").and_downcast::<gtk::Stack>()
        {
            stack.set_visible_child_name(&visible_tab);
        }
    }
    w.details_scroll.vadjustment().set_value(scroll_position);
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
    if let Some(job) = model
        .download_jobs
        .iter()
        .find(|job| active_job_ids.contains(&job.job_id))
    {
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
                || old.status_message != new.status_message
        })
}

pub(super) fn rebuild_downloads_page(w: &Widgets, model: &AppModel) {
    while let Some(child) = w.downloads.first_child() {
        w.downloads.remove(&child);
    }
    let jobs = &model.download_jobs;
    let page = gtk::Box::new(gtk::Orientation::Vertical, 24);
    page.set_margin_bottom(36);

    let featured_depot = model.depot_operations.iter().rev().find(|operation| {
        depot_active(&operation.state)
            || matches!(operation.state.as_str(), "interrupted" | "failed")
    });
    let featured_job = featured_depot
        .is_none()
        .then(|| {
            jobs.iter().rev().find(|job| {
                download::is_active(&job.job_id)
                    || matches!(job.state.as_str(), "paused" | "failed")
            })
        })
        .flatten();
    if let Some(operation) = featured_depot {
        page.append(&active_depot_header(operation, model));
    } else if let Some(job) = featured_job {
        page.append(&active_download_header(job, model, w));
    } else if !model.transfer_history.borrow().is_empty() {
        page.append(&completed_transfer_history_header(model));
    }

    let depot_is_active = featured_depot.is_some_and(|operation| depot_active(&operation.state));

    let queued = jobs
        .iter()
        .filter(|job| {
            job.state == "queued" || (depot_is_active && download::is_active(&job.job_id))
        })
        .filter(|job| depot_is_active || !download::is_active(&job.job_id))
        .collect::<Vec<_>>();
    let queued_depots = model
        .depot_operations
        .iter()
        .filter(|operation| operation.state == "queued")
        .collect::<Vec<_>>();
    page.append(&download_section_heading(
        "Up Next",
        queued.len() + queued_depots.len(),
    ));
    if queued.is_empty() && queued_depots.is_empty() {
        let empty = gtk::Label::new(Some("There are no downloads waiting in the queue"));
        empty.set_xalign(0.0);
        empty.add_css_class("downloads-empty");
        page.append(&empty);
    } else {
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
            | "verifying"
            | "verifying_existing"
            | "calculating"
            | "downloading"
            | "materializing"
            | "committing"
            | "finalizing"
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

fn depot_install_fraction(state: &str, written: u64, total: u64) -> f64 {
    match state {
        "committing" => 0.94,
        "finalizing" => 0.97,
        "complete" => 1.0,
        _ if total > 0 => (written as f64 / total as f64 * 0.9).clamp(0.0, 0.9),
        _ => 0.0,
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
        "<span foreground='#73c76b'>━</span> DISK USAGE",
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
        resume.set_tooltip_text(Some("Resume"));
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
    details.append(&transfer_stats(model, depot_active(&operation.state)));
    let download_fraction = operation.download_total_bytes.map_or(0.0, |total| {
        depot_download_fraction(operation.bytes_downloaded, total)
    });
    let download_label = if operation.state == "preparing" {
        "Preparing download"
    } else if operation.state == "verifying_existing" {
        "Checking existing files"
    } else if operation.state == "verifying" {
        "Checking downloaded files"
    } else if operation.state == "calculating" {
        "Calculating download size"
    } else if operation
        .download_total_bytes
        .is_some_and(|total| operation.bytes_downloaded >= total)
    {
        "Download complete"
    } else {
        "Downloading data"
    };
    details.append(&labeled_progress(
        download_label,
        download_fraction,
        &operation.download_total_bytes.map_or_else(
            || "Calculating…".into(),
            |total| {
                format!(
                    "{} / {}",
                    human_size(operation.bytes_downloaded),
                    human_size(total)
                )
            },
        ),
        false,
    ));
    let install_fraction = depot_install_fraction(
        &operation.state,
        operation.bytes_written,
        operation.total_write_bytes,
    );
    details.append(&labeled_progress(
        "Installing files",
        install_fraction,
        &format!("{:.0}%", install_fraction * 100.0),
        true,
    ));
    let footer_text = operation.error.clone().or_else(|| {
        operation.download_total_bytes.and_then(|total| {
            estimated_remaining(model, total.saturating_sub(operation.bytes_downloaded))
        })
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
        let token = model
            .account_token
            .as_ref()
            .map(|token| token.access_token.clone());
        let resume_id = operation_id.clone();
        resume.connect_clicked(move |button| {
            if let Some(token) = token.clone()
                && crate::installation::resume_depot_operation(resume_id.clone(), token)
            {
                button.set_sensitive(false);
            }
        });
        footer.append(&resume);
        let cancel = gtk::Button::from_icon_name("user-trash-symbolic");
        cancel.set_tooltip_text(Some("Cancel permanently"));
        cancel.connect_clicked(move |button| {
            if crate::installation::abandon_depot_operation(&operation_id) {
                button.set_sensitive(false);
            }
        });
        footer.append(&cancel);
    }
    details.append(&footer);
    header
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
        "committing" => "INSTALLING FILES",
        "finalizing" => "FINALIZING",
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
        let token = model
            .account_token
            .as_ref()
            .map(|token| token.access_token.clone());
        resume.connect_clicked(move |button| {
            if let Some(token) = token.clone()
                && crate::installation::resume_depot_operation(operation_id.clone(), token)
            {
                button.set_sensitive(false);
            }
        });
        row.append(&resume);
    }
    if operation.state != "complete" {
        let cancel = gtk::Button::from_icon_name("user-trash-symbolic");
        cancel.set_tooltip_text(Some("Cancel and remove partial files"));
        let operation_id = operation.operation_id.clone();
        cancel.connect_clicked(move |button| {
            if crate::installation::abandon_depot_operation(&operation_id) {
                button.set_sensitive(false);
            }
        });
        row.append(&cancel);
    }
    let _ = w;
    row
}

fn update_depot_page_progress(w: &Widgets, model: &AppModel) {
    let root: gtk::Widget = w.downloads.clone().upcast();
    if let Some(graph) = find_named_descendant(&root, "transfer-history-graph") {
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
            &root,
            "active-network-rate",
            if live {
                sample.download_bytes_per_second
            } else {
                0.0
            },
        );
        update_transfer_metric(
            &root,
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
        update_transfer_metric(&root, "active-peak-rate", peak);
    }
    let remaining = model
        .depot_operations
        .iter()
        .find(|operation| depot_active(&operation.state))
        .and_then(|operation| {
            operation
                .download_total_bytes
                .map(|total| total.saturating_sub(operation.bytes_downloaded))
        })
        .or_else(|| {
            model
                .download_jobs
                .iter()
                .find(|job| download::is_active(&job.job_id))
                .map(|job| {
                    job.total_bytes
                        .unwrap_or_default()
                        .saturating_sub(job.bytes_downloaded)
                })
        });
    if let Some(eta) =
        find_named_descendant(&root, "active-transfer-eta").and_downcast::<gtk::Label>()
    {
        eta.set_label(
            &remaining
                .and_then(|remaining| estimated_remaining(model, remaining))
                .unwrap_or_default(),
        );
    }
    for operation in &model.depot_operations {
        if let Some(detail) =
            find_named_descendant(&root, &format!("depot-detail-{}", operation.operation_id))
                .and_downcast::<gtk::Label>()
        {
            let percent = operation
                .download_total_bytes
                .filter(|total| *total > 0)
                .map(|total| operation.bytes_downloaded.saturating_mul(100) / total);
            detail.set_label(&percent.map_or_else(
                || operation.state.clone(),
                |percent| format!("{} · {percent}%", operation.state),
            ));
        }
        if let Some(progress) =
            find_named_descendant(&root, &format!("depot-progress-{}", operation.operation_id))
                .and_downcast::<gtk::ProgressBar>()
            && operation.download_total_bytes.is_some()
        {
            progress.set_fraction(depot_download_fraction(
                operation.bytes_downloaded,
                operation.download_total_bytes.unwrap_or_default(),
            ));
        }
        if depot_active(&operation.state) {
            if let Some(progress) = find_named_descendant(&root, "active-download-progress")
                .and_downcast::<gtk::ProgressBar>()
                && operation.download_total_bytes.is_some()
            {
                progress.set_fraction(depot_download_fraction(
                    operation.bytes_downloaded,
                    operation.download_total_bytes.unwrap_or_default(),
                ));
            }
            if let Some(detail) =
                find_named_descendant(&root, "active-download-detail").and_downcast::<gtk::Label>()
            {
                detail.set_label(&operation.download_total_bytes.map_or_else(
                    || "Calculating…".into(),
                    |total| {
                        format!(
                            "{} / {}",
                            human_size(operation.bytes_downloaded),
                            human_size(total)
                        )
                    },
                ));
            }
            let install_fraction = depot_install_fraction(
                &operation.state,
                operation.bytes_written,
                operation.total_write_bytes,
            );
            if let Some(progress) = find_named_descendant(&root, "active-disk-progress")
                .and_downcast::<gtk::ProgressBar>()
            {
                progress.set_fraction(install_fraction.clamp(0.0, 1.0));
            }
            if let Some(detail) =
                find_named_descendant(&root, "active-disk-detail").and_downcast::<gtk::Label>()
            {
                detail.set_label(&format!("{:.0}%", install_fraction * 100.0));
            }
        }
    }
}

fn update_transfer_metric(root: &gtk::Widget, name: &str, bytes_per_second: f64) {
    if let Some(label) = find_named_descendant(root, name).and_downcast::<gtk::Label>() {
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
        "paused" => "Paused — return to the game’s Offline Installers tab to resume".into(),
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
    fn depot_disk_progress_reserves_final_ten_percent() {
        assert_eq!(depot_install_fraction("materializing", 50, 100), 0.45);
        assert_eq!(depot_install_fraction("materializing", 200, 100), 0.9);
        assert_eq!(depot_install_fraction("committing", 100, 100), 0.94);
        assert_eq!(depot_install_fraction("finalizing", 100, 100), 0.97);
        assert_eq!(depot_install_fraction("complete", 100, 100), 1.0);
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

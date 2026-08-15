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
        let Some(snapshot) = latest else {
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
        if jobs_changed && w.content.visible_child_name().as_deref() == Some("downloads") {
            rebuild_downloads_page(&w, &model.borrow());
        } else if w.content.visible_child_name().as_deref() == Some("downloads") {
            update_download_page_progress(&w, &model.borrow(), &snapshot.active_job_ids);
        }
        glib::ControlFlow::Continue
    });
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
    page.set_margin_start(28);
    page.set_margin_end(28);
    page.set_margin_top(24);
    page.set_margin_bottom(36);
    let title = gtk::Label::new(Some("Downloads"));
    title.set_xalign(0.0);
    title.add_css_class("downloads-title");
    page.append(&title);

    let active = jobs
        .iter()
        .filter(|job| download::is_active(&job.job_id))
        .collect::<Vec<_>>();
    for job in active {
        page.append(&download_job_card(job, model, w, true));
    }

    let queued = jobs
        .iter()
        .filter(|job| job.state != "complete")
        .filter(|job| !download::is_active(&job.job_id))
        .collect::<Vec<_>>();
    page.append(&download_section_heading("Up Next", queued.len()));
    if queued.is_empty() {
        let empty = gtk::Label::new(Some("There are no downloads waiting in the queue"));
        empty.set_xalign(0.0);
        empty.add_css_class("downloads-empty");
        page.append(&empty);
    } else {
        for job in queued {
            page.append(&download_job_card(job, model, w, false));
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
    page.append(&download_section_heading("Completed", completed.len()));
    if completed.is_empty() {
        let empty = gtk::Label::new(Some("Completed downloads will appear here"));
        empty.set_xalign(0.0);
        empty.add_css_class("downloads-empty");
        page.append(&empty);
    } else {
        for job in completed {
            page.append(&download_job_card(job, model, w, false));
        }
    }
    w.downloads.append(&page);
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

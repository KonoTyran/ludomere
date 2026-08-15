use super::*;

type ManagedArtifactIdentity = (i64, String, Option<String>);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum InstallerCoverage {
    #[default]
    None,
    Partial,
    Complete,
}

pub(super) fn installer_backup_coverage(
    game: &DetailPageModel,
    config: &Config,
) -> InstallerCoverage {
    let managed_paths = managed_artifact_paths();
    let mut required = 0_usize;
    let mut downloaded = 0_usize;
    for artifacts in std::iter::once(game.remote_artifacts.as_slice()).chain(
        game.dlcs
            .iter()
            .filter(|dlc| dlc.owned)
            .map(|dlc| dlc.remote_artifacts.as_slice()),
    ) {
        let groups = preferred_installer_groups(artifacts, config);
        required += groups.len();
        downloaded += groups
            .iter()
            .filter(|group| {
                dialog_artifact_state(group, &managed_paths) == DialogArtifactState::Downloaded
            })
            .count();
    }
    if required > 0 && downloaded == required {
        InstallerCoverage::Complete
    } else if downloaded > 0 {
        InstallerCoverage::Partial
    } else {
        InstallerCoverage::None
    }
}

fn preferred_installer_groups(artifacts: &[RemoteArtifact], config: &Config) -> Vec<ArtifactGroup> {
    let groups = download_selection::group_artifacts(artifacts)
        .into_iter()
        .filter(|group| group.kind == ArtifactKind::Installer)
        .collect::<Vec<_>>();
    let languages = download_selection::available_languages(groups.iter());
    let selected_languages =
        download_selection::default_languages(&languages, config.installer_language.as_deref());
    let mut selected_os = BTreeSet::new();
    if config.installer_windows {
        selected_os.insert("windows".into());
    }
    if config.installer_linux {
        selected_os.insert("linux".into());
    }
    if config.installer_macos {
        selected_os.insert("macos".into());
    }
    groups
        .iter()
        .filter(|group| {
            download_selection::matches_preferences(group, &selected_os, &selected_languages)
        })
        .filter(|group| preferred_artifact_group(group, groups.iter()))
        .cloned()
        .collect()
}

pub(super) fn required_owned_dlc_ids(game: &DetailPageModel, config: &Config) -> HashSet<i64> {
    game.dlcs
        .iter()
        .filter(|dlc| {
            dlc.owned && !preferred_installer_groups(&dlc.remote_artifacts, config).is_empty()
        })
        .map(|dlc| dlc.product_id)
        .collect()
}

pub(super) fn default_installers_are_downloaded(game: &DetailPageModel, config: &Config) -> bool {
    installer_backup_coverage(game, config) == InstallerCoverage::Complete
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct DlcActionState {
    pub(super) missing_download: bool,
    pub(super) missing_install: bool,
}

pub(super) fn owned_dlc_action_state(
    game: &DetailPageModel,
    config: &Config,
    base_installed: bool,
) -> DlcActionState {
    if game.parent_id.is_some() {
        return DlcActionState::default();
    }
    let managed_paths = managed_artifact_paths();
    let installed_ids = if base_installed {
        StateStore::open()
            .and_then(|store| crate::installation::installed_dlc_ids(&store, game.product_id))
            .unwrap_or_default()
    } else {
        Default::default()
    };
    let mut state = DlcActionState::default();
    for dlc in game.dlcs.iter().filter(|dlc| {
        dlc.owned
            && dlc
                .remote_artifacts
                .iter()
                .any(|artifact| artifact.kind == ArtifactKind::Installer)
    }) {
        let downloaded = product_default_installers_are_downloaded(
            &dlc.remote_artifacts,
            config,
            &managed_paths,
            false,
        );
        state.missing_download |= !downloaded;
        state.missing_install |= base_installed && !installed_ids.contains(&dlc.product_id);
    }
    state
}

pub(super) fn has_additional_download_options(game: &DetailPageModel) -> bool {
    let managed_paths = managed_artifact_paths();
    std::iter::once(game.remote_artifacts.as_slice())
        .chain(
            (game.parent_id.is_none())
                .then_some(
                    game.dlcs
                        .iter()
                        .filter(|dlc| dlc.owned)
                        .map(|dlc| dlc.remote_artifacts.as_slice()),
                )
                .into_iter()
                .flatten(),
        )
        .flat_map(download_selection::group_artifacts)
        .any(|group| {
            matches!(
                dialog_artifact_state(&group, &managed_paths),
                DialogArtifactState::Available | DialogArtifactState::Resumable
            )
        })
}

pub(super) fn product_default_installers_are_downloaded(
    artifacts: &[RemoteArtifact],
    config: &Config,
    managed_paths: &HashSet<ManagedArtifactIdentity>,
    allow_no_installers: bool,
) -> bool {
    let groups = download_selection::group_artifacts(artifacts)
        .into_iter()
        .filter(|group| group.kind == ArtifactKind::Installer)
        .collect::<Vec<_>>();
    if groups.is_empty() {
        return allow_no_installers;
    }
    let languages = download_selection::available_languages(groups.iter());
    let selected_languages =
        download_selection::default_languages(&languages, config.installer_language.as_deref());
    let mut selected_os = BTreeSet::new();
    if config.installer_windows {
        selected_os.insert("windows".into());
    }
    if config.installer_linux {
        selected_os.insert("linux".into());
    }
    if config.installer_macos {
        selected_os.insert("macos".into());
    }
    let preferred = groups
        .iter()
        .filter(|group| {
            download_selection::matches_preferences(group, &selected_os, &selected_languages)
        })
        .filter(|group| preferred_artifact_group(group, groups.iter()))
        .collect::<Vec<_>>();
    if preferred.is_empty() {
        return false;
    }
    preferred
        .into_iter()
        .all(|group| dialog_artifact_state(group, managed_paths) == DialogArtifactState::Downloaded)
}

pub(super) fn managed_artifact_paths() -> HashSet<ManagedArtifactIdentity> {
    StateStore::open()
        .and_then(|store| store.managed_files())
        .unwrap_or_default()
        .into_iter()
        .filter(|file| file.present && file.matched)
        .filter_map(|file| {
            file.provider_file_id
                .or(file.artifact_path)
                .map(|identity| (file.product_id, identity, file.version))
        })
        .collect()
}

pub(super) fn show_install_dialog(window: &adw::ApplicationWindow, detail: &DetailPageModel) {
    show_install_dialog_with_mode(window, detail, false);
}

pub(super) fn show_repair_dialog(window: &adw::ApplicationWindow, detail: &DetailPageModel) {
    show_install_dialog_with_mode(window, detail, true);
}

fn show_install_dialog_with_mode(
    window: &adw::ApplicationWindow,
    detail: &DetailPageModel,
    repair: bool,
) {
    let config = Config::load_or_create().unwrap_or_default();
    let store = StateStore::open().ok();
    let managed_files = store
        .as_ref()
        .and_then(|store| store.managed_files().ok())
        .unwrap_or_default();
    let installation_product_id = detail.parent_id.unwrap_or(detail.product_id);
    let existing_installation = store.as_ref().and_then(|store| {
        crate::installation::reconcile_installed_games(store, &config.game_libraries)
            .ok()?
            .into_iter()
            .find(|game| game.product_id == installation_product_id)
            .filter(|game| game.state == crate::domain::InstallationState::Installed)
    });
    let installed_dlc_ids = store
        .as_ref()
        .and_then(|store| {
            crate::installation::installed_dlc_ids(store, installation_product_id).ok()
        })
        .unwrap_or_default();
    let base_product_id = installation_product_id;
    let mut candidates = StateStore::open()
        .and_then(|store| {
            Ok(crate::installation::detect_installer_candidates(
                base_product_id,
                &store.load_all_download_revisions(base_product_id)?,
                &store.managed_files()?,
                &config,
            ))
        })
        .unwrap_or_default();
    candidates.usable.retain(|candidate| {
        candidate.method != crate::installation::InstallationMethod::Unsupported
    });
    // Do not default back to the installed release: the detector places the
    // newest currently offered downloaded installer first.
    let dialog = adw::Dialog::builder()
        .content_width(680)
        .content_height(620)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(
        &format!(
            "{} {}",
            if repair { "Repair" } else { "Install" },
            detail.title
        ),
        if repair {
            "Reinstall the game and its DLC"
        } else {
            "Create an installation plan"
        },
    )));
    root.append(&header);
    let body = adw::PreferencesPage::new();
    let installer_group = adw::PreferencesGroup::new();
    installer_group.set_title("Installer");
    let candidate_labels = candidates
        .usable
        .iter()
        .map(|candidate| {
            format!(
                "{} · {} · {}",
                candidate.operating_system.as_deref().unwrap_or("Any OS"),
                candidate.language.as_deref().unwrap_or("Any language"),
                candidate.version.as_deref().unwrap_or("Unknown version"),
            )
        })
        .collect::<Vec<_>>();
    let candidate_list = gtk::StringList::new(
        &candidate_labels
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    let candidate = gtk::DropDown::new(Some(candidate_list), gtk::Expression::NONE);
    candidate.set_selected(candidates.preferred.unwrap_or(0) as u32);
    let candidate_menu = gtk::MenuButton::new();
    candidate_menu.set_hexpand(true);
    candidate_menu.set_sensitive(!candidates.usable.is_empty());
    candidate_menu.add_css_class("install-choice-menu");
    let candidate_popover = gtk::Popover::new();
    let candidate_choices = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let mut candidate_choice_buttons = Vec::new();
    for (index, installer) in candidates.usable.iter().enumerate() {
        let choice = gtk::Button::new();
        choice.add_css_class("flat");
        choice.add_css_class("install-choice-row");
        choice.set_child(Some(&install_choice_content(
            detail.icon.as_deref(),
            &detail.title,
            &candidate_labels[index],
            installer.total_size,
            index == candidate.selected() as usize,
            false,
        )));
        choice.connect_clicked({
            let candidate = candidate.clone();
            let popover = candidate_popover.clone();
            move |_| {
                candidate.set_selected(index as u32);
                popover.popdown();
            }
        });
        candidate_choices.append(&choice);
        candidate_choice_buttons.push(choice);
    }
    candidate_popover.set_child(Some(&candidate_choices));
    candidate_menu.set_popover(Some(&candidate_popover));
    if let Some(installer) = candidates.usable.get(candidate.selected() as usize) {
        candidate_menu.set_child(Some(&install_choice_content(
            detail.icon.as_deref(),
            &detail.title,
            &candidate_labels[candidate.selected() as usize],
            installer.total_size,
            false,
            true,
        )));
    }
    installer_group.add(&candidate_menu);
    if let Some(installed_game) = &existing_installation
        && !repair
    {
        candidate_menu.set_sensitive(false);
        let installed_label = format!(
            "{} · {} · {}",
            installed_game
                .installer_operating_system
                .as_deref()
                .unwrap_or("Unknown OS"),
            installed_game
                .installer_language
                .as_deref()
                .unwrap_or("Unknown language"),
            installed_game
                .installed_version
                .as_deref()
                .unwrap_or("Unknown version")
        );
        candidate_menu.set_child(Some(&install_choice_content(
            detail.icon.as_deref(),
            &detail.title,
            &installed_label,
            0,
            false,
            false,
        )));
        candidate_menu.set_tooltip_text(Some(
            "Installed base game; no base installer file is required to add DLC",
        ));
    }
    let dlc_products = if detail.parent_id.is_some() {
        vec![(detail.product_id, detail.title.clone())]
    } else {
        detail
            .dlcs
            .iter()
            .filter(|dlc| dlc.owned)
            .map(|dlc| (dlc.product_id, dlc.title.clone()))
            .collect()
    };
    let selected_base_version = candidates
        .usable
        .get(candidate.selected() as usize)
        .and_then(|installer| installer.version.as_deref())
        .or_else(|| {
            existing_installation
                .as_ref()
                .and_then(|installed| installed.installed_version.as_deref())
        });
    let mut dlc_choices = Vec::new();
    for (dlc_product_id, dlc_title) in dlc_products {
        let revisions = store
            .as_ref()
            .and_then(|store| store.load_all_download_revisions(dlc_product_id).ok())
            .unwrap_or_default();
        let choices = crate::installation::detect_installer_candidates(
            dlc_product_id,
            &revisions,
            &managed_files,
            &config,
        );
        let installers = choices
            .usable
            .into_iter()
            .filter(|installer| {
                installer.complete
                    && installer.method != crate::installation::InstallationMethod::Unsupported
                    && existing_installation.as_ref().is_none_or(|installed| {
                        installer_matches_installed_game(installer, installed)
                    })
            })
            .collect::<Vec<_>>();
        let selected = installers
            .iter()
            .find(|installer| versions_match(installer.version.as_deref(), selected_base_version))
            .cloned();
        let check = gtk::CheckButton::with_label(&dlc_title);
        let already_installed = installed_dlc_ids.contains(&dlc_product_id);
        check.set_active(selected.is_some());
        check.set_sensitive(selected.is_some() && (repair || !already_installed));
        if already_installed && !repair {
            check.set_tooltip_text(Some("Already installed"));
        }
        dlc_choices.push(DlcInstallerChoice {
            product_id: dlc_product_id,
            title: dlc_title,
            installers,
            selected: Rc::new(RefCell::new(selected)),
            check,
            already_installed,
        });
    }
    let dlc_summary = gtk::Label::new(None);
    if !dlc_choices.is_empty() {
        let dlc_menu = gtk::MenuButton::new();
        dlc_menu.set_hexpand(true);
        dlc_menu.add_css_class("install-dlc-menu");
        dlc_summary.set_xalign(0.0);
        dlc_summary.set_hexpand(true);
        let dlc_menu_content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        dlc_menu_content.append(&gtk::Image::from_icon_name("package-x-generic-symbolic"));
        dlc_menu_content.append(&dlc_summary);
        dlc_menu_content.append(&gtk::Image::from_icon_name("pan-down-symbolic"));
        dlc_menu.set_child(Some(&dlc_menu_content));
        let dlc_popover = gtk::Popover::new();
        let dlc_list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        dlc_list.set_margin_start(12);
        dlc_list.set_margin_end(12);
        dlc_list.set_margin_top(10);
        dlc_list.set_margin_bottom(10);
        let update_summary: Rc<dyn Fn()> = {
            let choices = dlc_choices.clone();
            let summary = dlc_summary.clone();
            Rc::new(move || update_dlc_summary(&summary, &choices, repair))
        };
        for choice in &dlc_choices {
            let check = &choice.check;
            dlc_list.append(check);
            let update_summary = update_summary.clone();
            check.connect_toggled(move |_| update_summary());
        }
        update_summary();
        dlc_popover.set_child(Some(&dlc_list));
        dlc_menu.set_popover(Some(&dlc_popover));
        installer_group.add(&dlc_menu);
    }
    let interactive_prompts = adw::SwitchRow::new();
    interactive_prompts.set_title("Interactive install");
    interactive_prompts.set_subtitle(
        "Show the installer and let you choose optional settings; Ludomere still supplies the install directory",
    );
    interactive_prompts.set_tooltip_text(Some(
        "When disabled, Windows installers run fully unattended. Enable this to choose options such as the installer language yourself.",
    ));
    interactive_prompts.set_active(config.interactive_installer_prompts);
    {
        let window = window.clone();
        let reverting = Rc::new(std::cell::Cell::new(false));
        interactive_prompts.connect_active_notify(move |row| {
            if reverting.replace(false) || !row.is_active() {
                return;
            }
            let config = Config::load_or_create().unwrap_or_default();
            if config.interactive_installer_explanation_dismissed {
                return;
            }
            let never_show = gtk::CheckButton::with_label("Don't show this explanation again");
            let explanation = adw::AlertDialog::builder()
                .heading("Enable interactive install?")
                .body("Windows installers will open normally so you can choose optional settings such as language. Ludomere will still provide the required game directory. Linux installers will ask you for responses they cannot handle automatically.")
                .extra_child(&never_show)
                .build();
            explanation.add_responses(&[("cancel", "Cancel"), ("enable", "Enable")]);
            explanation.set_default_response(Some("enable"));
            explanation.set_close_response("cancel");
            explanation.set_response_appearance("enable", adw::ResponseAppearance::Suggested);
            let row = row.clone();
            let reverting = reverting.clone();
            explanation.choose(Some(&window), gio::Cancellable::NONE, move |response| {
                if never_show.is_active()
                    && let Ok(mut config) = Config::load_or_create()
                {
                    config.interactive_installer_explanation_dismissed = true;
                    let _ = config.save();
                }
                if response != "enable" {
                    reverting.set(true);
                    row.set_active(false);
                }
            });
        });
    }
    let installer_detail = adw::ActionRow::new();
    body.add(&installer_group);

    let destination_group = adw::PreferencesGroup::new();
    destination_group.set_title("INSTALL TO:");
    let storage_settings = gtk::Button::from_icon_name("emblem-system-symbolic");
    storage_settings.set_tooltip_text(Some("Open Storage settings"));
    destination_group.set_header_suffix(Some(&storage_settings));
    let library_labels = config
        .game_libraries
        .iter()
        .map(|library| library.name.as_str())
        .collect::<Vec<_>>();
    let library_list = gtk::StringList::new(&library_labels);
    let library = gtk::DropDown::new(Some(library_list), gtk::Expression::NONE);
    let default_library = existing_installation
        .as_ref()
        .and_then(|installed| {
            config.game_libraries.iter().position(|library| {
                library.id == installed.library_id
                    || installed.installation_directory.parent() == Some(library.path.as_path())
            })
        })
        .or_else(|| {
            config
                .game_libraries
                .iter()
                .position(|library| library.default)
        })
        .unwrap_or(0);
    library.set_selected(default_library as u32);
    let library_choices = gtk::ListBox::new();
    library_choices.set_selection_mode(gtk::SelectionMode::Single);
    library_choices.add_css_class("install-library-list");
    let storage_locked = existing_installation.is_some();
    for (index, game_library) in config.game_libraries.iter().enumerate() {
        let row = gtk::ListBoxRow::new();
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        content.append(&gtk::Image::from_icon_name("drive-harddisk-symbolic"));
        let mount = gtk::Label::new(Some(&settings::storage::filesystem_mount_point(
            &game_library.path,
        )));
        mount.set_xalign(0.0);
        mount.set_hexpand(true);
        mount.add_css_class("install-library-path");
        content.append(&mount);
        if game_library.default {
            let star = gtk::Image::from_icon_name("starred-symbolic");
            star.add_css_class("storage-default-library");
            star.set_tooltip_text(Some("Default game library"));
            content.append(&star);
        }
        let free = gtk::Label::new(Some("Calculating…"));
        free.add_css_class("install-library-free");
        content.append(&free);
        update_install_library_free_space(&game_library.path, &free);
        row.set_child(Some(&content));
        if storage_locked && index != default_library {
            row.add_css_class("dim-label");
            row.set_tooltip_text(Some(
                "Use Storage settings to move an installed game to another library.",
            ));
        } else if storage_locked {
            row.set_tooltip_text(Some("This game is installed in this library."));
        }
        library_choices.append(&row);
        if index == default_library {
            library_choices.select_row(Some(&row));
        }
    }
    library_choices.connect_row_selected({
        let library = library.clone();
        let locked_index = storage_locked.then_some(default_library as i32);
        move |list, row| {
            if let Some(row) = row {
                if let Some(locked) = locked_index {
                    if row.index() != locked
                        && let Some(installed_row) = list.row_at_index(locked)
                    {
                        list.select_row(Some(&installed_row));
                    }
                    library.set_selected(locked as u32);
                } else {
                    library.set_selected(row.index() as u32);
                }
            }
        }
    });
    destination_group.add(&library_choices);
    let path_row = adw::ActionRow::new();
    body.add(&destination_group);

    update_install_plan_preview(
        &installer_detail,
        &path_row,
        &candidates.usable,
        candidate.selected() as usize,
        &config.game_libraries,
        library.selected() as usize,
        &detail.slug,
    );
    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .child(&body)
        .build();
    root.append(&scroll);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    footer.add_css_class("download-selector-footer");
    interactive_prompts.set_title("Interactive install");
    interactive_prompts.set_subtitle("");
    interactive_prompts.set_hexpand(false);
    interactive_prompts.add_css_class("install-footer-switch");
    interactive_prompts.set_activatable(true);
    footer.append(&interactive_prompts);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    footer.append(&spacer);
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_hexpand(true);
    status.set_wrap(true);
    footer.append(&status);
    let close = gtk::Button::with_label("Cancel");
    footer.append(&close);
    let install = gtk::Button::new();
    install.add_css_class("suggested-action");
    install.set_sensitive(
        !config.game_libraries.is_empty()
            && ((!candidates.usable.is_empty() && (repair || existing_installation.is_none()))
                || (existing_installation.is_some()
                    && dlc_choices.iter().any(|choice| choice.check.is_sensitive()))),
    );
    update_install_action(
        &install,
        &candidates.usable,
        candidate.selected() as usize,
        existing_installation.is_some() && !repair,
        &dlc_choices,
    );
    footer.append(&install);
    for choice in &dlc_choices {
        let check = &choice.check;
        let install = install.clone();
        let candidates = candidates.usable.clone();
        let candidate = candidate.clone();
        let dlc_choices = dlc_choices.clone();
        let base_installed = existing_installation.is_some() && !repair;
        check.connect_toggled(move |_| {
            update_install_action(
                &install,
                &candidates,
                candidate.selected() as usize,
                base_installed,
                &dlc_choices,
            );
        });
    }
    root.append(&footer);
    {
        let dialog = dialog.clone();
        close.connect_clicked(move |_| {
            dialog.close();
        });
    }
    {
        let dialog = dialog.clone();
        let window = window.clone();
        storage_settings.connect_clicked(move |_| {
            dialog.close();
            if let Err(error) = gtk::prelude::WidgetExt::activate_action(
                &window,
                "win.settings-page",
                Some(&"storage".to_variant()),
            ) {
                tracing::warn!(%error, "could not open Storage settings");
            }
        });
    }
    {
        let detail_row = installer_detail.clone();
        let path_row = path_row.clone();
        let candidates = candidates.usable.clone();
        let libraries = config.game_libraries.clone();
        let library = library.clone();
        let slug = detail.slug.clone();
        let install = install.clone();
        let candidate_menu = candidate_menu.clone();
        let candidate_labels = candidate_labels.clone();
        let candidate_choice_buttons = candidate_choice_buttons.clone();
        let icon = detail.icon.clone();
        let title = detail.title.clone();
        let existing_installation = existing_installation.is_some() && !repair;
        let dlc_choices = dlc_choices.clone();
        let dlc_summary = dlc_summary.clone();
        candidate.connect_selected_notify(move |candidate| {
            let selected = candidate.selected() as usize;
            let base_version = candidates
                .get(selected)
                .and_then(|installer| installer.version.as_deref());
            for choice in &dlc_choices {
                let matching = choice
                    .installers
                    .iter()
                    .find(|installer| versions_match(installer.version.as_deref(), base_version))
                    .cloned();
                *choice.selected.borrow_mut() = matching.clone();
                choice.check.set_active(matching.is_some());
                choice
                    .check
                    .set_sensitive(matching.is_some() && (repair || !choice.already_installed));
                choice.check.set_tooltip_text(if matching.is_none() {
                    Some("A complete DLC installer matching the selected game version is not downloaded")
                } else {
                    None
                });
            }
            update_dlc_summary(&dlc_summary, &dlc_choices, repair);
            update_install_plan_preview(
                &detail_row,
                &path_row,
                &candidates,
                selected,
                &libraries,
                library.selected() as usize,
                &slug,
            );
            update_install_action(
                &install,
                &candidates,
                selected,
                existing_installation,
                &dlc_choices,
            );
            if let Some(installer) = candidates.get(selected) {
                candidate_menu.set_child(Some(&install_choice_content(
                    icon.as_deref(),
                    &title,
                    &candidate_labels[selected],
                    installer.total_size,
                    false,
                    true,
                )));
            }
            for (index, button) in candidate_choice_buttons.iter().enumerate() {
                if let Some(installer) = candidates.get(index) {
                    button.set_child(Some(&install_choice_content(
                        icon.as_deref(),
                        &title,
                        &candidate_labels[index],
                        installer.total_size,
                        index == selected,
                        false,
                    )));
                }
            }
        });
    }
    {
        let detail_row = installer_detail.clone();
        let path_row = path_row.clone();
        let candidates = candidates.usable.clone();
        let libraries = config.game_libraries.clone();
        let candidate = candidate.clone();
        let slug = detail.slug.clone();
        library.connect_selected_notify(move |library| {
            update_install_plan_preview(
                &detail_row,
                &path_row,
                &candidates,
                candidate.selected() as usize,
                &libraries,
                library.selected() as usize,
                &slug,
            );
        });
    }
    {
        let candidates = candidates.usable;
        let dlc_choices = dlc_choices.clone();
        let existing_installation = existing_installation.clone();
        let libraries = config.game_libraries;
        let product_id = detail.product_id;
        let slug = detail.slug.clone();
        let candidate = candidate.clone();
        let library = library.clone();
        let status = status.clone();
        let dialog = dialog.clone();
        let interactive_prompts = interactive_prompts.clone();
        install.connect_clicked(move |_| {
            let candidate = candidates.get(candidate.selected() as usize);
            if existing_installation.is_none() && candidate.is_none() {
                return;
            }
            let Some(library) = libraries.get(library.selected() as usize) else {
                return;
            };
            if let Err(error) = std::fs::create_dir_all(&library.path) {
                status.set_label(&format!("Could not create game library: {error}"));
                status.add_css_class("error");
                return;
            }
            let now = chrono::Utc::now().timestamp();
            let plan = if let Some(mut installed) = existing_installation.clone() {
                if repair {
                    let Some(candidate) = candidate else { return };
                    installed.installed_version = candidate.version.clone();
                    installed.installer_revision_id = candidate.revision_id;
                    installed.installer_files = candidate.paths.clone();
                    installed.installer_complete = candidate.complete;
                    installed.installer_operating_system = candidate.operating_system.clone();
                    installed.installer_language = candidate.language.clone();
                }
                installed
            } else {
                let candidate = candidate.expect("a new installation requires an installer");
                crate::domain::InstalledGame {
                    product_id,
                    library_id: library.id.clone(),
                    installed_version: candidate.version.clone(),
                    installation_directory: library.path.join(&slug),
                    installer_revision_id: candidate.revision_id,
                    installer_job_id: None,
                    installer_files: candidate.paths.clone(),
                    installer_complete: candidate.complete,
                    installer_operating_system: candidate.operating_system.clone(),
                    installer_language: candidate.language.clone(),
                    compatibility: None,
                    primary_executable: None,
                    launch_arguments: Vec::new(),
                    state: crate::domain::InstallationState::Pending,
                    error: None,
                    installed_at: None,
                    verified_at: None,
                    last_played_at: None,
                    playtime_seconds: 0,
                    created_at: now,
                    updated_at: now,
                }
            };
            match StateStore::open()
                .and_then(|store| crate::installation::save_game_preferences(&store, &plan))
            {
                Ok(()) => {
                    status.remove_css_class("error");
                    let windows = candidate.is_some_and(|candidate| {
                        candidate.method
                            == crate::installation::InstallationMethod::WindowsCompatibility
                    });
                    status.set_label(if windows {
                        "Preparing UMU compatibility environment…"
                    } else {
                        "Starting Linux installer…"
                    });
                    status.remove_css_class("success");
                    let additional_installers = dlc_choices
                        .iter()
                        .filter(|choice| {
                            choice.check.is_active() && (repair || choice.check.is_sensitive())
                        })
                        .filter_map(|choice| {
                            let installer = choice.selected.borrow().clone()?;
                            versions_match(
                                installer.version.as_deref(),
                                candidate
                                    .and_then(|base| base.version.as_deref())
                                    .or(plan.installed_version.as_deref()),
                            )
                            .then_some((choice, installer))
                        })
                        .map(
                            |(choice, installer)| crate::installation::AdditionalInstaller {
                                product_id: choice.product_id,
                                revision_id: installer.revision_id,
                                version: installer.version.clone(),
                                title: choice.title.clone(),
                                files: installer.paths.clone(),
                            },
                        )
                        .collect::<Vec<_>>();
                    let install_base = repair || existing_installation.is_none();
                    let started = crate::installation::enqueue_installation(
                        plan,
                        additional_installers,
                        install_base,
                        interactive_prompts.is_active(),
                    );
                    if !started {
                        status
                            .set_label("An installation operation is already active for this game");
                        status.add_css_class("error");
                        return;
                    }
                    dialog.close();
                }
                Err(error) => {
                    status.set_label(&format!("Could not save installation plan: {error}"));
                    status.add_css_class("error");
                }
            }
        });
    }
    dialog.set_child(Some(&root));
    dialog.present(Some(window));
}

pub(super) fn present_installer_prompt(
    window: &adw::ApplicationWindow,
    product_id: i64,
    prompt: &str,
    choices: &[String],
    context: &str,
) {
    let dialog = adw::Dialog::builder()
        .content_width(760)
        .content_height(480)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(
        "Installer needs your response",
        prompt,
    )));
    root.append(&header);
    let prompt_details = gtk::Box::new(gtk::Orientation::Vertical, 8);
    prompt_details.set_margin_start(20);
    prompt_details.set_margin_end(20);
    prompt_details.set_vexpand(true);
    let context_heading = gtk::Label::new(Some("Recent installer output"));
    context_heading.set_xalign(0.0);
    context_heading.add_css_class("heading");
    prompt_details.append(&context_heading);
    let context_label = gtk::Label::new(Some(context));
    context_label.set_xalign(0.0);
    context_label.set_wrap(true);
    context_label.set_selectable(true);
    context_label.add_css_class("monospace");
    context_label.add_css_class("installer-prompt-context");
    let context_scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .min_content_height(220)
        .child(&context_label)
        .build();
    prompt_details.append(&context_scroll);
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("Type the response to send to the installer"));
    let actions = gtk::Grid::builder()
        .column_spacing(8)
        .row_spacing(8)
        .margin_start(20)
        .margin_end(20)
        .margin_bottom(16)
        .build();
    let cancel = gtk::Button::with_label("Cancel Installation");
    cancel.add_css_class("destructive-action");
    let dialog_for_cancel = dialog.clone();
    cancel.connect_clicked(move |_| {
        crate::installation::cancel_operation(product_id);
        dialog_for_cancel.close();
    });
    if choices.is_empty() {
        prompt_details.append(&entry);
        let send = gtk::Button::with_label("Send");
        send.add_css_class("suggested-action");
        let dialog_for_send = dialog.clone();
        let entry_for_send = entry.clone();
        send.connect_clicked(move |_| {
            crate::installation::respond_to_installation(
                product_id,
                entry_for_send.text().to_string(),
            );
            dialog_for_send.close();
        });
        actions.attach(&cancel, 0, 0, 1, 1);
        actions.attach(&send, 1, 0, 1, 1);
    } else {
        for (index, choice) in choices.iter().enumerate() {
            let response = gtk::Button::with_label(choice);
            response.set_hexpand(true);
            let dialog = dialog.clone();
            let choice = choice.clone();
            response.connect_clicked(move |_| {
                crate::installation::respond_to_installation(product_id, choice.clone());
                dialog.close();
            });
            actions.attach(&response, (index % 2) as i32, (index / 2) as i32, 1, 1);
        }
        actions.attach(&cancel, 0, choices.len().div_ceil(2) as i32, 2, 1);
    }
    root.append(&prompt_details);
    root.append(&actions);
    dialog.set_child(Some(&root));
    dialog.present(Some(window));
}

#[derive(Clone)]
struct DlcInstallerChoice {
    product_id: i64,
    title: String,
    installers: Vec<crate::installation::InstallerCandidate>,
    selected: Rc<RefCell<Option<crate::installation::InstallerCandidate>>>,
    check: gtk::CheckButton,
    already_installed: bool,
}

fn update_dlc_summary(summary: &gtk::Label, choices: &[DlcInstallerChoice], repair: bool) {
    let selected = choices
        .iter()
        .filter(|choice| choice.check.is_active() && choice.check.is_sensitive())
        .count();
    let unavailable = choices
        .iter()
        .filter(|choice| !choice.check.is_sensitive())
        .count();
    summary.set_label(&dlc_summary_text(selected, unavailable, repair));
}

fn dlc_summary_text(selected: usize, unavailable: usize, repair: bool) -> String {
    let suffix = if unavailable == 0 {
        String::new()
    } else if repair {
        format!(" · {unavailable} unavailable")
    } else {
        format!(" · {unavailable} installed")
    };
    format!("DLC · {selected} selected{suffix}")
}

fn update_install_action(
    button: &gtk::Button,
    candidates: &[crate::installation::InstallerCandidate],
    selected: usize,
    base_installed: bool,
    dlc_choices: &[DlcInstallerChoice],
) {
    let selected_dlc = dlc_choices
        .iter()
        .filter(|choice| choice.check.is_active() && choice.check.is_sensitive())
        .collect::<Vec<_>>();
    let supported = if base_installed {
        !selected_dlc.is_empty()
            && selected_dlc.iter().all(|choice| {
                choice.selected.borrow().as_ref().is_some_and(|installer| {
                    installer.method != crate::installation::InstallationMethod::Unsupported
                })
            })
    } else {
        candidates.get(selected).is_some_and(|candidate| {
            candidate.method != crate::installation::InstallationMethod::Unsupported
        }) && selected_dlc.iter().all(|choice| {
            choice.selected.borrow().as_ref().is_some_and(|installer| {
                installer.method != crate::installation::InstallationMethod::Unsupported
            })
        })
    };
    button.set_label("Install");
    button.set_sensitive(supported);
    button.set_tooltip_text(Some(if supported {
        "Start installation"
    } else {
        "The selected installer is unsupported"
    }));
}

fn installer_matches_installed_game(
    candidate: &crate::installation::InstallerCandidate,
    installed: &crate::domain::InstalledGame,
) -> bool {
    let operating_system_matches =
        installed
            .installer_operating_system
            .as_deref()
            .is_none_or(|installed_os| {
                candidate
                    .operating_system
                    .as_deref()
                    .is_some_and(|candidate_os| candidate_os.eq_ignore_ascii_case(installed_os))
            });
    let language_matches =
        installed
            .installer_language
            .as_deref()
            .is_none_or(|installed_language| {
                candidate
                    .language
                    .as_deref()
                    .is_some_and(|candidate_language| {
                        candidate_language.eq_ignore_ascii_case(installed_language)
                    })
            });
    operating_system_matches && language_matches
}

fn versions_match(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.trim().eq_ignore_ascii_case(right.trim()),
        (None, None) => true,
        _ => false,
    }
}

fn install_choice_content(
    icon: Option<&std::path::Path>,
    title: &str,
    choice: &str,
    size: u64,
    selected: bool,
    dropdown: bool,
) -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let artwork = gtk::Picture::new();
    artwork.set_width_request(52);
    artwork.set_height_request(52);
    artwork.set_content_fit(gtk::ContentFit::Cover);
    if let Some(icon) = icon {
        artwork.set_filename(Some(icon));
    }
    content.append(&artwork);
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
    labels.set_hexpand(true);
    let title = gtk::Label::new(Some(title));
    title.set_xalign(0.0);
    title.add_css_class("file-name");
    labels.append(&title);
    let choice = gtk::Label::new(Some(choice));
    choice.set_xalign(0.0);
    choice.add_css_class("dim-label");
    labels.append(&choice);
    content.append(&labels);
    let size = gtk::Label::new(Some(&human_size(size)));
    size.add_css_class("install-choice-size");
    content.append(&size);
    if selected || dropdown {
        let indicator = gtk::Image::from_icon_name(if selected {
            "object-select-symbolic"
        } else {
            "pan-down-symbolic"
        });
        content.append(&indicator);
    }
    content
}

fn update_install_library_free_space(path: &std::path::Path, label: &gtk::Label) {
    let path = path.to_owned();
    let label = label.clone();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let free = std::fs::create_dir_all(&path)
            .ok()
            .and_then(|()| fs2::available_space(&path).ok());
        let _ = sender.send(free);
    });
    glib::timeout_add_local(Duration::from_millis(100), move || {
        match receiver.try_recv() {
            Ok(Some(free)) => {
                label.set_label(&format!("{} free", human_size(free)));
                glib::ControlFlow::Break
            }
            Ok(None) => {
                label.set_label("Unavailable");
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

fn update_install_plan_preview(
    method_row: &adw::ActionRow,
    path_row: &adw::ActionRow,
    candidates: &[crate::installation::InstallerCandidate],
    candidate_index: usize,
    libraries: &[crate::config::GameLibrary],
    library_index: usize,
    slug: &str,
) {
    if let Some(candidate) = candidates.get(candidate_index) {
        let (title, method_description) = match candidate.method {
            crate::installation::InstallationMethod::WindowsCompatibility => (
                "Windows through UMU",
                "A dedicated compatibility environment will use L: for the selected library",
            ),
            crate::installation::InstallationMethod::NativeLinux => (
                "Native Linux installer",
                "The installer will target the selected host library directly",
            ),
            crate::installation::InstallationMethod::Unsupported => (
                "Unsupported on Arch Linux",
                "This installer cannot be executed on the current system",
            ),
        };
        let completeness = if candidate.complete {
            ""
        } else {
            "; missing parts may be reported when the installer runs"
        };
        method_row.set_subtitle(&format!(
            "{method_description} · {} local file{} selected{completeness}",
            candidate.paths.len(),
            if candidate.paths.len() == 1 { "" } else { "s" },
        ));
        method_row.set_title(title);
    }
    path_row.set_subtitle(
        &libraries
            .get(library_index)
            .map(|library| {
                let host = library.path.join(slug).display().to_string();
                if candidates.get(candidate_index).is_some_and(|candidate| {
                    candidate.method
                        == crate::installation::InstallationMethod::WindowsCompatibility
                }) {
                    format!(
                        "{host} · Windows: {}",
                        crate::compatibility::windows_destination(slug)
                    )
                } else {
                    host
                }
            })
            .unwrap_or_else(|| "No game library configured".to_owned()),
    );
}

pub(super) fn show_download_selector(
    w: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
    detail: &DetailPageModel,
) {
    let app = model.borrow();
    let Some(token) = app
        .account_token
        .as_ref()
        .map(|token| token.access_token.clone())
    else {
        return;
    };
    let mut products = vec![DownloadDialogProduct {
        product_id: detail.product_id,
        slug: detail.slug.clone(),
        parent_slug: detail.parent_slug.clone(),
        title: detail.title.clone(),
        artwork: detail
            .artwork
            .clone()
            .or_else(|| detail.icon.clone())
            .or_else(|| detail.detail_artwork.clone()),
        groups: download_selection::group_artifacts(&detail.remote_artifacts),
        is_primary: true,
    }];
    if detail.parent_id.is_none() {
        products.extend(detail.dlcs.iter().filter(|dlc| dlc.owned).map(|dlc| {
            DownloadDialogProduct {
                product_id: dlc.product_id,
                slug: dlc.slug.clone(),
                parent_slug: Some(detail.slug.clone()),
                title: dlc.title.clone(),
                artwork: dlc.artwork.clone().or_else(|| dlc.icon.clone()),
                groups: download_selection::group_artifacts(&dlc.remote_artifacts),
                is_primary: false,
            }
        }));
    }
    let base_installers = products[0]
        .groups
        .iter()
        .filter(|group| group.kind == ArtifactKind::Installer)
        .collect::<Vec<_>>();
    let available_base_os = base_installers
        .iter()
        .filter_map(|group| group.operating_system.as_deref())
        .map(normalize_os)
        .collect::<BTreeSet<_>>();
    let languages = download_selection::available_languages(base_installers.into_iter());
    let selected_languages =
        download_selection::default_languages(&languages, app.config.installer_language.as_deref());
    let mut selected_os = BTreeSet::new();
    if app.config.installer_windows {
        selected_os.insert("windows".into());
    }
    if app.config.installer_linux {
        selected_os.insert("linux".into());
    }
    if app.config.installer_macos {
        selected_os.insert("macos".into());
    }
    selected_os.retain(|os| available_base_os.contains(os));
    let online = app.network_available;
    let download_directory = app.config.download_directory.clone();
    let include_extras = app.config.download_extras_by_default;
    let installed_update = StateStore::open().ok().is_some_and(|store| {
        crate::installation::reconcile_installed_games(&store, &app.config.game_libraries)
            .is_ok_and(|games| {
                games
                    .iter()
                    .any(|game| game.product_id == detail.product_id)
            })
    });
    let include_patches = app.config.download_patches_by_default
        || (installed_update && app.config.prefer_patch_updates);
    drop(app);

    let dialog = gtk::Window::builder()
        .title(format!("Download {}", detail.title))
        .transient_for(&w.window)
        .modal(true)
        .default_width(920)
        .default_height(720)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 18);
    body.set_margin_start(22);
    body.set_margin_end(22);
    body.set_margin_top(18);
    body.set_margin_bottom(24);

    let preferences = gtk::Box::new(gtk::Orientation::Vertical, 10);
    preferences.add_css_class("download-selector-card");
    preferences.add_css_class("compact-download-preferences");
    let title = gtk::Label::new(Some("Installer preferences"));
    title.set_xalign(0.0);
    title.add_css_class("section-title");
    preferences.append(&title);
    let os_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let os_label = gtk::Label::new(Some("Platforms"));
    os_label.set_xalign(0.0);
    os_row.append(&os_label);
    let mut os_buttons = [
        ("windows", "Windows"),
        ("linux", "Linux"),
        ("macos", "macOS"),
    ]
    .into_iter()
    .filter(|(os, _)| available_base_os.contains(*os))
    .map(|(os, label)| {
        let button = gtk::ToggleButton::with_label(label);
        button.set_active(selected_os.contains(os));
        button.add_css_class("compact-preference-toggle");
        (os, button)
    })
    .collect::<Vec<_>>();
    os_buttons.sort_by_key(|(os, _)| download_selection::operating_system_rank(Some(os)));
    for (_, button) in &os_buttons {
        os_row.append(button);
    }
    let language_popover = gtk::Popover::new();
    let language_flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .column_spacing(10)
        .row_spacing(6)
        .max_children_per_line(3)
        .build();
    language_flow.set_margin_start(12);
    language_flow.set_margin_end(12);
    language_flow.set_margin_top(12);
    language_flow.set_margin_bottom(12);
    let mut language_buttons = Vec::new();
    for language in &languages {
        let button = gtk::CheckButton::with_label(language);
        button.set_active(selected_languages.contains(language));
        language_flow.insert(&button, -1);
        language_buttons.push((language.clone(), button));
    }
    if languages.is_empty() {
        let empty = gtk::Label::new(Some("No language-specific installers are listed"));
        empty.add_css_class("dim-label");
        language_flow.insert(&empty, -1);
    }
    language_popover.set_child(Some(&language_flow));
    let language_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let language_label = gtk::Label::new(Some("Languages"));
    language_label.set_xalign(0.0);
    language_row.append(&language_label);
    let language_summary = gtk::Label::new(None);
    let language_menu_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    language_menu_content.append(&language_summary);
    language_menu_content.append(&gtk::Image::from_icon_name("pan-down-symbolic"));
    let language_menu = gtk::MenuButton::builder()
        .popover(&language_popover)
        .child(&language_menu_content)
        .build();
    language_menu.add_css_class("compact-preference-menu");
    language_row.append(&language_menu);
    let selection_row = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    selection_row.append(&os_row);
    selection_row.append(&language_row);
    preferences.append(&selection_row);

    let optional_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let optional_heading = gtk::Label::new(Some("Optional content"));
    optional_heading.set_xalign(0.0);
    optional_row.append(&optional_heading);
    let extras_label = gtk::Label::new(Some("Extras"));
    extras_label.set_xalign(0.0);
    extras_label.add_css_class("dim-label");
    let include_extras_toggle = gtk::Switch::new();
    include_extras_toggle.set_active(include_extras);
    optional_row.append(&extras_label);
    optional_row.append(&include_extras_toggle);
    let patches_label = gtk::Label::new(Some("Compatible patches"));
    patches_label.set_xalign(0.0);
    patches_label.add_css_class("dim-label");
    let include_patches_toggle = gtk::Switch::new();
    include_patches_toggle.set_active(include_patches);
    optional_row.append(&patches_label);
    optional_row.append(&include_patches_toggle);
    let optional_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    optional_spacer.set_hexpand(true);
    optional_row.append(&optional_spacer);
    let view_file_details = gtk::Button::with_label("View file details…");
    view_file_details.add_css_class("flat");
    optional_row.append(&view_file_details);
    preferences.append(&optional_row);
    body.append(&preferences);

    let expert_dialog = gtk::Window::builder()
        .title(format!("File details — {}", detail.title))
        .transient_for(&dialog)
        .modal(true)
        .default_width(960)
        .default_height(720)
        .build();
    let expert_root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let expert_body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    expert_body.set_margin_start(18);
    expert_body.set_margin_end(18);
    expert_body.set_margin_top(16);
    expert_body.set_margin_bottom(20);
    let expert_scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&expert_body)
        .build();
    expert_root.append(&expert_scroll);
    let expert_footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    expert_footer.add_css_class("download-selector-footer");
    let expert_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    expert_spacer.set_hexpand(true);
    expert_footer.append(&expert_spacer);
    let expert_done = gtk::Button::with_label("Done");
    expert_footer.append(&expert_done);
    expert_root.append(&expert_footer);
    expert_dialog.set_child(Some(&expert_root));

    let plan_content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let plan_title = gtk::Label::new(Some("Download plan"));
    plan_title.set_xalign(0.0);
    plan_title.add_css_class("heading");
    body.append(&plan_title);
    body.append(&plan_content);

    let managed_paths = managed_artifact_paths();
    let selected_products = products
        .iter()
        .map(|product| product.product_id)
        .collect::<HashSet<_>>();
    let state = Rc::new(RefCell::new(DownloadDialogState {
        selected_products,
        selected_operating_systems: selected_os,
        selected_languages,
        selected_groups: HashSet::new(),
        include_extras,
        include_patches,
        applying: false,
    }));
    let mut rows = Vec::new();
    let mut warnings = HashMap::new();
    let mut product_content = HashMap::new();
    let mut category_expanders = HashMap::new();
    let mut plan_boxes = HashMap::new();
    let mut product_toggles = HashMap::new();
    let mut dlc_toggles = Vec::new();
    let mut category_controls = Vec::new();
    for product in &products {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 10);
        card.add_css_class("download-selector-card");
        let product_heading = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        product_heading.append(&card_picture(product.artwork.as_ref(), 96, 54));
        let product_toggle = gtk::CheckButton::with_label(&product.title);
        product_toggle.set_active(
            state
                .borrow()
                .selected_products
                .contains(&product.product_id),
        );
        product_toggle.set_sensitive(!product.is_primary);
        product_toggle.add_css_class("section-title");
        product_toggles.insert(product.product_id, product_toggle.clone());
        product_heading.append(&product_toggle);
        card.append(&product_heading);
        if !product.is_primary {
            dlc_toggles.push((product.product_id, product_toggle));
        }
        let warning = gtk::Label::new(None);
        warning.set_xalign(0.0);
        warning.set_wrap(true);
        warning.add_css_class("warning");
        warning.set_visible(false);
        card.append(&warning);
        warnings.insert(product.product_id, warning);
        let plan_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        plan_boxes.insert(product.product_id, plan_box.clone());
        let expert_card = gtk::Box::new(gtk::Orientation::Vertical, 10);
        expert_card.add_css_class("download-selector-card");
        let expert_title = gtk::Label::new(Some(&product.title));
        expert_title.set_xalign(0.0);
        expert_title.add_css_class("section-title");
        expert_card.append(&expert_title);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
        for (kind, heading) in [
            (ArtifactKind::Installer, "Offline installers"),
            (ArtifactKind::Patch, "Patches"),
            (ArtifactKind::Extra, "Extras"),
        ] {
            let groups = product
                .groups
                .iter()
                .filter(|group| group.kind == kind)
                .cloned()
                .map(|group| {
                    let state = dialog_artifact_state(&group, &managed_paths);
                    (group, state)
                })
                .collect::<Vec<_>>();
            if groups.is_empty() {
                continue;
            }
            let downloaded = groups
                .iter()
                .filter(|(_, state)| *state == DialogArtifactState::Downloaded)
                .count();
            let preferred_downloaded = if kind == ArtifactKind::Installer {
                let selection = state.borrow();
                groups
                    .iter()
                    .filter(|(_, status)| *status == DialogArtifactState::Downloaded)
                    .filter(|(group, _)| {
                        download_selection::matches_preferences(
                            group,
                            &selection.selected_operating_systems,
                            &selection.selected_languages,
                        )
                    })
                    .count()
            } else {
                0
            };
            let section = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let heading_text = if preferred_downloaded > 0 {
                format!("{heading}  ·  {preferred_downloaded} preferred downloaded")
            } else if downloaded > 0 {
                format!("{heading}  ·  {downloaded} downloaded")
            } else {
                heading.to_owned()
            };
            let heading_label = gtk::Label::new(Some(&heading_text));
            heading_label.set_xalign(0.0);
            heading_label.set_hexpand(true);
            heading_label.add_css_class("file-name");
            section.append(&heading_label);
            let controls: &[(&str, &str)] = match kind {
                ArtifactKind::Patch => &[
                    ("All compatible", "compatible"),
                    ("All", "all"),
                    ("Clear", "clear"),
                ],
                ArtifactKind::Extra => &[("All", "all"), ("Clear", "clear")],
                ArtifactKind::Installer => &[],
            };
            for (label, mode) in controls {
                let button = gtk::Button::with_label(label);
                button.add_css_class("flat");
                button.add_css_class("compact-selector-action");
                section.append(&button);
                category_controls.push((button, product.product_id, kind, *mode));
            }
            let section_rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
            for (group, artifact_state) in groups {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                row.add_css_class("download-selector-row");
                let check = gtk::CheckButton::new();
                check.set_sensitive(matches!(
                    artifact_state,
                    DialogArtifactState::Available | DialogArtifactState::Resumable
                ));
                row.append(&check);
                if group.operating_system.is_some() || group.language.is_some() {
                    row.append(&artifact_identity_badge(&group.artifacts[0]));
                }
                let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
                labels.set_hexpand(true);
                let name = gtk::Label::new(Some(&group.name));
                name.set_xalign(0.0);
                name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                name.add_css_class("file-name");
                labels.append(&name);
                let mut parts = [
                    group.operating_system.as_deref(),
                    group.language.as_deref(),
                    group.version.as_deref(),
                ]
                .into_iter()
                .flatten()
                .map(str::to_owned)
                .collect::<Vec<_>>();
                if group.artifacts.len() > 1 {
                    parts.push(format!("{} parts", group.artifacts.len()));
                }
                let status = match artifact_state {
                    DialogArtifactState::Downloaded => Some("Downloaded"),
                    DialogArtifactState::Busy => Some("Queued or downloading"),
                    DialogArtifactState::Resumable => Some("Resume or retry"),
                    DialogArtifactState::Available => None,
                };
                if let Some(status) = status {
                    parts.push(status.into());
                }
                let metadata = gtk::Label::new(Some(&parts.join("  ·  ")));
                metadata.set_xalign(0.0);
                metadata.set_ellipsize(gtk::pango::EllipsizeMode::End);
                metadata.add_css_class("dim-label");
                labels.append(&metadata);
                row.append(&labels);
                if let Some(size) = group.total_size {
                    let size = gtk::Label::new(Some(&human_size(size)));
                    size.add_css_class("dim-label");
                    row.append(&size);
                }
                section_rows.append(&row);
                rows.push(DownloadDialogRow {
                    group,
                    check,
                    state: artifact_state,
                });
            }
            let expander = gtk::Expander::new(None);
            expander.set_label_widget(Some(&section));
            expander.set_child(Some(&section_rows));
            expander.set_expanded(false);
            expander.add_css_class("download-selector-expander");
            category_expanders.insert((product.product_id, kind.as_str()), expander.clone());
            content.append(&expander);
        }
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .reveal_child(product.is_primary)
            .child(&plan_box)
            .build();
        card.append(&revealer);
        product_content.insert(product.product_id, revealer);
        plan_content.append(&card);
        expert_card.append(&content);
        expert_body.append(&expert_card);
    }

    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&body)
        .build();
    root.append(&scroll);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    footer.add_css_class("download-selector-footer");
    let summary = gtk::Label::new(None);
    summary.set_xalign(0.0);
    summary.set_hexpand(true);
    footer.append(&summary);
    let install_after = gtk::CheckButton::with_label("Install after downloading");
    install_after.set_sensitive(false);
    install_after.set_tooltip_text(Some(
        "Installation support will be added in a future update",
    ));
    footer.append(&install_after);
    let cancel = gtk::Button::with_label("Cancel");
    footer.append(&cancel);
    let confirm = gtk::Button::with_label("Add to Download Queue");
    confirm.add_css_class("suggested-action");
    footer.append(&confirm);
    root.append(&footer);
    dialog.set_child(Some(&root));
    let widgets = Rc::new(DownloadDialogWidgets {
        rows,
        products,
        warnings,
        product_content,
        category_expanders,
        plan_boxes,
        product_toggles,
        language_summary,
        summary,
        confirm: confirm.clone(),
        authenticated: true,
        online,
        download_directory: download_directory.clone(),
    });

    connect_download_selector_controls(
        &state,
        &widgets,
        os_buttons,
        language_buttons,
        dlc_toggles,
        category_controls,
        (include_extras_toggle, include_patches_toggle),
    );
    for row in &widgets.rows {
        let job_id = row.group.job_id.clone();
        let product_id = row.group.product_id;
        let state = state.clone();
        let widgets = widgets.clone();
        row.check.connect_toggled(move |check| {
            if state.borrow().applying {
                return;
            }
            if check.is_active() {
                let mut selection = state.borrow_mut();
                selection.selected_groups.insert(job_id.clone());
                selection.selected_products.insert(product_id);
            } else {
                state.borrow_mut().selected_groups.remove(&job_id);
            }
            refresh_download_selector(&state, &widgets, false);
        });
    }
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| dialog.close());
    }
    {
        let expert_dialog = expert_dialog.clone();
        view_file_details.connect_clicked(move |_| expert_dialog.present());
    }
    {
        let expert_dialog = expert_dialog.clone();
        expert_done.connect_clicked(move |_| expert_dialog.close());
    }
    {
        let dialog = dialog.clone();
        let state = state.clone();
        let widgets = widgets.clone();
        let status = w.status.clone();
        confirm.connect_clicked(move |_| {
            let selected = selected_download_groups(&state.borrow(), &widgets);
            let added = selected.len();
            for (product, group) in selected {
                let refs = group.artifacts.iter().collect::<Vec<_>>();
                let destination = matching_download_job(&refs)
                    .map(|job| job.destination)
                    .unwrap_or_else(|| {
                        download::destination(
                            &download_directory,
                            product.parent_slug.as_deref().unwrap_or(&product.slug),
                            product.parent_slug.as_ref().map(|_| product.slug.as_str()),
                            &refs,
                        )
                    });
                let (sender, _receiver) = mpsc::channel();
                download::enqueue(download::DownloadRequest {
                    artifacts: group.artifacts.clone(),
                    title: product.title.clone(),
                    access_token: token.clone(),
                    destination,
                    events: sender,
                });
            }
            status.set_label(&format!("Added {added} downloads to the queue"));
            dialog.close();
        });
    }
    refresh_download_selector(&state, &widgets, true);
    dialog.present();
    {
        let dialog = dialog.clone();
        let state = state.clone();
        let widgets = widgets.clone();
        let artifact_states = Rc::new(RefCell::new(download_dialog_artifact_states(&widgets)));
        glib::timeout_add_local(Duration::from_millis(500), move || {
            if !dialog.is_visible() {
                return glib::ControlFlow::Break;
            }
            let current = download_dialog_artifact_states(&widgets);
            if *artifact_states.borrow() != current {
                *artifact_states.borrow_mut() = current;
                refresh_download_selector(&state, &widgets, false);
            }
            glib::ControlFlow::Continue
        });
    }
}

pub(super) fn download_dialog_artifact_states(
    widgets: &DownloadDialogWidgets,
) -> Vec<DialogArtifactState> {
    let managed_paths = managed_artifact_paths();
    widgets
        .rows
        .iter()
        .map(|row| dialog_artifact_state(&row.group, &managed_paths))
        .collect()
}

type DialogCategoryControl = (gtk::Button, i64, ArtifactKind, &'static str);

pub(super) fn connect_download_selector_controls(
    state: &Rc<RefCell<DownloadDialogState>>,
    widgets: &Rc<DownloadDialogWidgets>,
    os_buttons: Vec<(&'static str, gtk::ToggleButton)>,
    language_buttons: Vec<(String, gtk::CheckButton)>,
    dlc_toggles: Vec<(i64, gtk::CheckButton)>,
    category_controls: Vec<DialogCategoryControl>,
    optional_toggles: (gtk::Switch, gtk::Switch),
) {
    for (os, button) in os_buttons {
        let state = state.clone();
        let widgets = widgets.clone();
        button.connect_toggled(move |button| {
            if state.borrow().applying {
                return;
            }
            if button.is_active() {
                state
                    .borrow_mut()
                    .selected_operating_systems
                    .insert(os.into());
            } else {
                state.borrow_mut().selected_operating_systems.remove(os);
            }
            refresh_download_selector(&state, &widgets, true);
        });
    }
    let (include_extras, include_patches) = optional_toggles;
    for (language, button) in language_buttons {
        let state = state.clone();
        let widgets = widgets.clone();
        button.connect_toggled(move |button| {
            if state.borrow().applying {
                return;
            }
            if button.is_active() {
                state
                    .borrow_mut()
                    .selected_languages
                    .insert(language.clone());
            } else {
                state.borrow_mut().selected_languages.remove(&language);
            }
            refresh_download_selector(&state, &widgets, true);
        });
    }
    for (product_id, button) in dlc_toggles {
        let state = state.clone();
        let widgets = widgets.clone();
        button.connect_toggled(move |button| {
            if state.borrow().applying {
                return;
            }
            if button.is_active() {
                state.borrow_mut().selected_products.insert(product_id);
            } else {
                state.borrow_mut().selected_products.remove(&product_id);
            }
            refresh_download_selector(&state, &widgets, true);
        });
    }
    for (button, product_id, kind, mode) in category_controls {
        let state = state.clone();
        let widgets = widgets.clone();
        button.connect_clicked(move |_| {
            let mut selection = state.borrow_mut();
            for row in widgets
                .rows
                .iter()
                .filter(|row| row.group.product_id == product_id && row.group.kind == kind)
                .filter(|row| {
                    matches!(
                        row.state,
                        DialogArtifactState::Available | DialogArtifactState::Resumable
                    )
                })
            {
                let selected = match mode {
                    "all" => true,
                    "compatible" => download_selection::matches_preferences(
                        &row.group,
                        &selection.selected_operating_systems,
                        &selection.selected_languages,
                    ),
                    _ => false,
                };
                if selected {
                    selection.selected_groups.insert(row.group.job_id.clone());
                } else {
                    selection.selected_groups.remove(&row.group.job_id);
                }
            }
            drop(selection);
            refresh_download_selector(&state, &widgets, false);
        });
    }
    {
        let state = state.clone();
        let widgets = widgets.clone();
        include_extras.connect_active_notify(move |button| {
            if state.borrow().applying {
                return;
            }
            state.borrow_mut().include_extras = button.is_active();
            refresh_download_selector(&state, &widgets, true);
        });
    }
    {
        let state = state.clone();
        let widgets = widgets.clone();
        include_patches.connect_active_notify(move |button| {
            if state.borrow().applying {
                return;
            }
            state.borrow_mut().include_patches = button.is_active();
            refresh_download_selector(&state, &widgets, true);
        });
    }
}

pub(super) fn dialog_artifact_state(
    group: &ArtifactGroup,
    managed_paths: &HashSet<ManagedArtifactIdentity>,
) -> DialogArtifactState {
    let refs = group.artifacts.iter().collect::<Vec<_>>();
    if group.artifacts.iter().all(|artifact| {
        let identity = artifact
            .provider_file_id
            .as_ref()
            .unwrap_or(&artifact.download_path);
        managed_paths.contains(&(group.product_id, identity.clone(), artifact.version.clone()))
    }) {
        return DialogArtifactState::Downloaded;
    }
    let Some(job) = matching_download_job(&refs) else {
        return DialogArtifactState::Available;
    };
    if download_job_is_complete(&job) {
        DialogArtifactState::Downloaded
    } else if download::is_active(&job.job_id) || job.state == "queued" {
        DialogArtifactState::Busy
    } else {
        DialogArtifactState::Resumable
    }
}

pub(super) fn refresh_download_selector(
    state: &Rc<RefCell<DownloadDialogState>>,
    widgets: &Rc<DownloadDialogWidgets>,
    apply_defaults: bool,
) {
    let managed_paths = managed_artifact_paths();
    {
        let mut selection = state.borrow_mut();
        selection.applying = true;
        for row in &widgets.rows {
            let current = dialog_artifact_state(&row.group, &managed_paths);
            row.check.set_sensitive(matches!(
                current,
                DialogArtifactState::Available | DialogArtifactState::Resumable
            ));
            if matches!(
                current,
                DialogArtifactState::Downloaded | DialogArtifactState::Busy
            ) {
                selection.selected_groups.remove(&row.group.job_id);
            }
        }
        if apply_defaults {
            let products = selection.selected_products.clone();
            let operating_systems = selection.selected_operating_systems.clone();
            let languages = selection.selected_languages.clone();
            let include_extras = selection.include_extras;
            let include_patches = selection.include_patches;
            for row in &widgets.rows {
                let selectable = matches!(
                    dialog_artifact_state(&row.group, &managed_paths),
                    DialogArtifactState::Available | DialogArtifactState::Resumable
                );
                let selected = selectable
                    && products.contains(&row.group.product_id)
                    && match row.group.kind {
                        ArtifactKind::Extra => include_extras,
                        ArtifactKind::Installer => {
                            download_selection::matches_preferences(
                                &row.group,
                                &operating_systems,
                                &languages,
                            ) && preferred_installer_group(row, &widgets.rows)
                        }
                        ArtifactKind::Patch => {
                            include_patches
                                && download_selection::matches_preferences(
                                    &row.group,
                                    &operating_systems,
                                    &languages,
                                )
                        }
                    };
                if selected {
                    selection.selected_groups.insert(row.group.job_id.clone());
                } else {
                    selection.selected_groups.remove(&row.group.job_id);
                }
            }
        }
    }
    let (selected_products, selected_operating_systems, selected_languages, selected_groups) = {
        let selection = state.borrow();
        (
            selection.selected_products.clone(),
            selection.selected_operating_systems.clone(),
            selection.selected_languages.clone(),
            selection.selected_groups.clone(),
        )
    };
    for (product_id, toggle) in &widgets.product_toggles {
        toggle.set_active(selected_products.contains(product_id));
    }
    let language_summary = if selected_languages.is_empty() {
        "Any language".to_string()
    } else if selected_languages.len() == 1 {
        selected_languages
            .first()
            .cloned()
            .unwrap_or_else(|| "Not specified".to_string())
    } else {
        format!("{} selected", selected_languages.len())
    };
    widgets.language_summary.set_label(&language_summary);
    for row in &widgets.rows {
        row.check
            .set_active(selected_groups.contains(&row.group.job_id));
    }
    for product in &widgets.products {
        let selected = selected_products.contains(&product.product_id);
        if let Some(content) = widgets.product_content.get(&product.product_id) {
            content.set_reveal_child(selected);
        }
        if let Some(plan_box) = widgets.plan_boxes.get(&product.product_id) {
            rebuild_compact_product_plan(
                product,
                plan_box,
                &widgets.rows,
                &selected_groups,
                &selected_operating_systems,
                &selected_languages,
                &managed_paths,
            );
        }
    }
    let mut warning_count = 0;
    let mut valid = true;
    for product in &widgets.products {
        let selected = selected_products.contains(&product.product_id);
        let installers = product
            .groups
            .iter()
            .filter(|group| group.kind == ArtifactKind::Installer)
            .collect::<Vec<_>>();
        let selected_installers = installers
            .iter()
            .filter(|group| selected_groups.contains(&group.job_id))
            .count();
        let available_preferred_os = selected_operating_systems
            .iter()
            .filter(|os| {
                installers.iter().any(|group| {
                    group
                        .operating_system
                        .as_deref()
                        .is_some_and(|available| same_os(available, os))
                })
            })
            .collect::<Vec<_>>();
        let preferred_language_available = selected_languages.is_empty()
            || installers.iter().any(|group| {
                group.operating_system.as_deref().is_some_and(|os| {
                    available_preferred_os
                        .iter()
                        .any(|preferred| same_os(os, preferred))
                }) && group.language.as_deref().is_none_or(|language| {
                    selected_languages
                        .iter()
                        .any(|preferred| language.eq_ignore_ascii_case(preferred))
                })
            });
        let needs_os_fallback =
            selected && !installers.is_empty() && available_preferred_os.is_empty();
        let needs_language_fallback = selected
            && !installers.is_empty()
            && !available_preferred_os.is_empty()
            && !preferred_language_available;
        if let Some(label) = widgets.warnings.get(&product.product_id) {
            if needs_os_fallback {
                warning_count += 1;
                label.set_label(&format!(
                    "No installer is available for your preferred operating systems ({}). Choose an available operating system below.",
                    selected_operating_systems.iter().map(|os| display_os(os)).collect::<Vec<_>>().join(", ")
                ));
                label.set_visible(true);
            } else if needs_language_fallback {
                warning_count += 1;
                label.set_label(&format!(
                    "Your preferred languages ({}) are not available for {}. Choose an available language below.",
                    selected_languages.iter().cloned().collect::<Vec<_>>().join(", "),
                    available_preferred_os.iter().map(|os| display_os(os)).collect::<Vec<_>>().join(", ")
                ));
                label.set_visible(true);
            } else {
                label.set_visible(false);
            }
        }
        if let Some(expander) = widgets
            .category_expanders
            .get(&(product.product_id, ArtifactKind::Installer.as_str()))
            && (needs_os_fallback || needs_language_fallback)
        {
            expander.set_expanded(true);
        }
        let downloaded_installer = widgets.rows.iter().any(|row| {
            row.group.product_id == product.product_id
                && row.group.kind == ArtifactKind::Installer
                && dialog_artifact_state(&row.group, &managed_paths)
                    == DialogArtifactState::Downloaded
        });
        if selected && !installers.is_empty() && selected_installers == 0 && !downloaded_installer {
            valid = false;
        }
    }
    let selected_rows = widgets
        .rows
        .iter()
        .filter(|row| selected_groups.contains(&row.group.job_id))
        .collect::<Vec<_>>();
    let size = selected_rows
        .iter()
        .filter_map(|row| row.group.total_size)
        .sum::<u64>();
    let unknown = selected_rows
        .iter()
        .any(|row| row.group.total_size.is_none());
    widgets.summary.set_label(&format!(
        "{} download{}  ·  {}{}  ·  {} warning{}",
        selected_rows.len(),
        if selected_rows.len() == 1 { "" } else { "s" },
        if unknown { "at least " } else { "" },
        human_size(size),
        warning_count,
        if warning_count == 1 { "" } else { "s" },
    ));
    widgets.confirm.set_sensitive(
        !selected_rows.is_empty()
            && valid
            && widgets.authenticated
            && widgets.online
            && download_directory_available(&widgets.download_directory),
    );
    widgets
        .confirm
        .set_tooltip_text((!widgets.online).then_some("Connect to GOG to add downloads"));
    state.borrow_mut().applying = false;
}

pub(super) fn rebuild_compact_product_plan(
    product: &DownloadDialogProduct,
    container: &gtk::Box,
    rows: &[DownloadDialogRow],
    selected_groups: &HashSet<String>,
    operating_systems: &BTreeSet<String>,
    languages: &BTreeSet<String>,
    managed_paths: &HashSet<ManagedArtifactIdentity>,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    let product_rows = rows
        .iter()
        .filter(|row| row.group.product_id == product.product_id)
        .collect::<Vec<_>>();
    let downloaded_preferred = product_rows
        .iter()
        .filter(|row| row.group.kind == ArtifactKind::Installer)
        .filter(|row| {
            dialog_artifact_state(&row.group, managed_paths) == DialogArtifactState::Downloaded
        })
        .filter(|row| {
            download_selection::matches_preferences(&row.group, operating_systems, languages)
        })
        .count();
    if downloaded_preferred > 0 {
        let downloaded = gtk::Label::new(Some(&format!(
            "✓ {downloaded_preferred} preferred OS/language combination{} already downloaded",
            if downloaded_preferred == 1 { "" } else { "s" },
        )));
        downloaded.set_xalign(0.0);
        downloaded.add_css_class("success");
        container.append(&downloaded);
    }
    for row in product_rows
        .iter()
        .filter(|row| row.group.kind == ArtifactKind::Installer)
        .filter(|row| selected_groups.contains(&row.group.job_id))
        .filter(|row| {
            dialog_artifact_state(&row.group, managed_paths) != DialogArtifactState::Downloaded
        })
    {
        let mut details = format!(
            "Version {}",
            row.group.version.as_deref().unwrap_or("not listed")
        );
        let part_count = row
            .group
            .artifacts
            .iter()
            .filter_map(|artifact| artifact.part_count)
            .max()
            .unwrap_or(row.group.artifacts.len() as u32);
        if part_count > 1 {
            details.push_str(&format!(" · {part_count} parts"));
        }
        let plan = compact_plan_row(
            &format!(
                "{} · {}",
                row.group
                    .operating_system
                    .as_deref()
                    .map(display_os)
                    .unwrap_or("Any OS"),
                row.group.language.as_deref().unwrap_or("Any language"),
            ),
            row.group.total_size,
            &details,
        );
        container.append(&plan);
    }
    for (kind, title) in [
        (ArtifactKind::Extra, "Extras"),
        (ArtifactKind::Patch, "Compatible patches"),
    ] {
        let selected = product_rows
            .iter()
            .filter(|row| row.group.kind == kind)
            .filter(|row| selected_groups.contains(&row.group.job_id))
            .filter(|row| {
                dialog_artifact_state(&row.group, managed_paths) != DialogArtifactState::Downloaded
            })
            .collect::<Vec<_>>();
        if selected.is_empty() {
            continue;
        }
        let size = selected
            .iter()
            .map(|row| row.group.total_size)
            .collect::<Option<Vec<_>>>()
            .map(|sizes| sizes.into_iter().sum());
        container.append(&compact_optional_plan(title, &selected, size));
    }
    if container.first_child().is_none() {
        let empty = gtk::Label::new(Some("No new downloads selected for this product"));
        empty.set_xalign(0.0);
        empty.add_css_class("dim-label");
        container.append(&empty);
    }
}

pub(super) fn compact_optional_plan(
    title: &str,
    selected: &[&&DownloadDialogRow],
    size: Option<u64>,
) -> gtk::Expander {
    let expander = gtk::Expander::new(None);
    expander.add_css_class("compact-optional-plan");
    let summary = compact_plan_row(
        &format!(
            "{title} · {} item{}",
            selected.len(),
            if selected.len() == 1 { "" } else { "s" }
        ),
        size,
        "Click to show included files",
    );
    expander.set_label_widget(Some(&summary));
    let items = gtk::Box::new(gtk::Orientation::Vertical, 3);
    items.add_css_class("compact-optional-items");
    for row in selected {
        let item = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let name = gtk::Label::new(Some(&row.group.name));
        name.set_xalign(0.0);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        name.set_hexpand(true);
        item.append(&name);
        if let Some(size) = row.group.total_size {
            let size = gtk::Label::new(Some(&human_size(size)));
            size.add_css_class("dim-label");
            item.append(&size);
        }
        items.append(&item);
    }
    expander.set_child(Some(&items));
    expander
}

pub(super) fn compact_plan_row(title: &str, size: Option<u64>, subtitle: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("compact-download-plan-row");
    let selected = gtk::Image::from_icon_name("object-select-symbolic");
    selected.add_css_class("accent");
    row.append(&selected);
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
    labels.set_hexpand(true);
    let title = gtk::Label::new(Some(title));
    title.set_xalign(0.0);
    title.add_css_class("file-name");
    labels.append(&title);
    let subtitle = gtk::Label::new(Some(subtitle));
    subtitle.set_xalign(0.0);
    subtitle.add_css_class("dim-label");
    labels.append(&subtitle);
    row.append(&labels);
    if let Some(size) = size {
        let size = gtk::Label::new(Some(&human_size(size)));
        size.add_css_class("dim-label");
        row.append(&size);
    }
    row
}

pub(super) fn preferred_installer_group(
    candidate: &DownloadDialogRow,
    rows: &[DownloadDialogRow],
) -> bool {
    preferred_artifact_group(
        &candidate.group,
        rows.iter()
            .filter(|row| row.group.kind == ArtifactKind::Installer)
            .map(|row| &row.group),
    )
}

pub(super) fn preferred_artifact_group<'a>(
    candidate: &ArtifactGroup,
    groups: impl Iterator<Item = &'a ArtifactGroup>,
) -> bool {
    let candidate_key = (
        candidate.release_sort_key(),
        candidate.version.as_deref().unwrap_or_default(),
    );
    groups
        .filter(|group| group.product_id == candidate.product_id)
        .filter(|group| {
            group.operating_system == candidate.operating_system
                && group.language == candidate.language
        })
        .all(|group| {
            (
                group.release_sort_key(),
                group.version.as_deref().unwrap_or_default(),
            ) <= candidate_key
        })
}

pub(super) fn selected_download_groups<'a>(
    state: &DownloadDialogState,
    widgets: &'a DownloadDialogWidgets,
) -> Vec<(&'a DownloadDialogProduct, &'a ArtifactGroup)> {
    let mut selected = Vec::new();
    for product in &widgets.products {
        if !state.selected_products.contains(&product.product_id) {
            continue;
        }
        for kind in [
            ArtifactKind::Installer,
            ArtifactKind::Patch,
            ArtifactKind::Extra,
        ] {
            selected.extend(
                widgets
                    .rows
                    .iter()
                    .filter(|row| {
                        row.group.product_id == product.product_id && row.group.kind == kind
                    })
                    .filter(|row| state.selected_groups.contains(&row.group.job_id))
                    .map(|row| (product, &row.group)),
            );
        }
    }
    selected
}

pub(super) fn same_os(left: &str, right: &str) -> bool {
    normalize_os(left) == normalize_os(right)
}

pub(super) fn normalize_os(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "mac" | "osx" | "macos" => "macos".into(),
        "win" | "windows" => "windows".into(),
        "linux" => "linux".into(),
        value => value.into(),
    }
}

pub(super) fn display_os(value: &str) -> &str {
    match value.to_ascii_lowercase().as_str() {
        "windows" | "win" => "Windows",
        "linux" => "Linux",
        "mac" | "osx" | "macos" => "macOS",
        _ => value,
    }
}

pub(super) fn download_directory_available(path: &std::path::Path) -> bool {
    if path.is_dir() {
        return path
            .metadata()
            .is_ok_and(|metadata| !metadata.permissions().readonly());
    }
    path.ancestors()
        .skip(1)
        .find(|parent| parent.exists())
        .is_some_and(|parent| {
            parent
                .metadata()
                .is_ok_and(|metadata| metadata.is_dir() && !metadata.permissions().readonly())
        })
}

#[derive(Clone)]
pub(super) struct DetailFileManagement {
    pub(super) menu: gtk::MenuButton,
    pub(super) status: gtk::Label,
    pub(super) progress: gtk::ProgressBar,
}

#[cfg(test)]
mod installer_version_tests {
    use super::{dlc_summary_text, versions_match};

    #[test]
    fn dlc_must_match_the_selected_base_version_exactly() {
        assert!(versions_match(Some("1.3.0.6"), Some("1.3.0.6")));
        assert!(!versions_match(Some("1.3.0.5"), Some("1.3.0.6")));
        assert!(!versions_match(None, Some("1.3.0.6")));
    }

    #[test]
    fn repair_summary_distinguishes_selected_and_unavailable_dlc() {
        assert_eq!(
            dlc_summary_text(3, 1, true),
            "DLC · 3 selected · 1 unavailable"
        );
    }
}

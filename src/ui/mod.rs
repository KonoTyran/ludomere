use crate::{
    auth,
    config::{Config, SidebarSortMode},
    domain::{
        ArtifactKind, Dlc, ExternalLinks, Game, LibraryFile, RemoteArtifact, Screenshot, human_size,
    },
    download,
    download_selection::{self, ArtifactGroup},
    managed, online,
    state::{DownloadJobRecord, DownloadState, ProductActivity, StateStore},
    text,
};
mod account;
mod collections;
mod details;
mod download_chooser;
mod downloads;
mod executable_chooser;
mod files;
mod game_settings;
mod library;
mod settings;
mod style;
mod sync;
mod tray;
mod widgets;
mod window;

use account::*;
use adw::prelude::*;
use collections::*;
use details::*;
use download_chooser::*;
use downloads::*;
use executable_chooser::*;
use files::*;
use game_settings::*;

fn present_cloud_launch_event(
    window: &adw::ApplicationWindow,
    event: crate::installation::LaunchEvent,
) {
    match event {
        crate::installation::LaunchEvent::EnablementRequired { respond } => {
            let dialog = adw::AlertDialog::builder()
                .heading("Enable GOG Cloud Saves?")
                .body("Ludomere can synchronize this Windows game's saves before launch and after exit. You can change this later in game settings.")
                .build();
            dialog.add_responses(&[("disable", "Not Now"), ("enable", "Enable")]);
            dialog.set_default_response(Some("enable"));
            dialog.choose(Some(window), gio::Cancellable::NONE, move |response| {
                respond.send(response == "enable").ok();
            });
        }
        crate::installation::LaunchEvent::PreLaunchConflict { conflicts, respond } => {
            let dialog = adw::AlertDialog::builder()
                .heading("Cloud Save Conflict")
                .body(format!("{} save file(s) changed both locally and in GOG Cloud. Choose which copy to keep.", conflicts.len()))
                .build();
            dialog.add_responses(&[
                ("skip", "Skip and Launch"),
                ("cloud", "Use Cloud"),
                ("local", "Use Local"),
            ]);
            dialog.choose(Some(window), gio::Cancellable::NONE, move |response| {
                let mode = match response.as_str() {
                    "cloud" => Some(crate::domain::CloudSyncMode::ForceDownload),
                    "local" => Some(crate::domain::CloudSyncMode::ForceUpload),
                    _ => None,
                };
                respond.send(mode).ok();
            });
        }
        crate::installation::LaunchEvent::LaunchWithoutSyncRequired { message, respond } => {
            let dialog = adw::AlertDialog::builder()
                .heading("Cloud Saves Unavailable")
                .body(message)
                .build();
            dialog.add_responses(&[("cancel", "Cancel"), ("launch", "Launch Anyway")]);
            dialog.choose(Some(window), gio::Cancellable::NONE, move |response| {
                respond.send(response == "launch").ok();
            });
        }
        crate::installation::LaunchEvent::SyncWarning(message) => {
            let dialog = adw::AlertDialog::builder()
                .heading("Cloud Save Warning")
                .body(message)
                .build();
            dialog.add_response("close", "Close");
            dialog.present(Some(window));
        }
        crate::installation::LaunchEvent::PostExitConflict(_) => {}
        crate::installation::LaunchEvent::PostExitSync(_) => {}
        _ => unreachable!("non-cloud launch event"),
    }
}
use gdk_pixbuf::InterpType;
use gdk_pixbuf::prelude::PixbufLoaderExt;
use gtk::{gdk, gio, glib};
use library::*;
use settings::*;
use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap, HashSet},
    rc::Rc,
    sync::{Mutex, OnceLock, mpsc},
    time::Duration,
};
use sync::*;
pub(crate) use tray::shutdown_tray;
use widgets::content::{empty_dash, expandable_section, lazy_html_section, section, text_excerpt};
use widgets::gallery::screenshot_strip;
use widgets::media::{
    card_picture, install_smooth_wheel_scroll, parallax_detail_hero, picture, scaled_card_texture,
};
pub use window::build_window;

static VERIFICATION_STATES: OnceLock<Mutex<HashMap<i64, VerificationDisplayState>>> =
    OnceLock::new();

#[derive(Clone)]
struct VerificationDisplayState {
    message: String,
    fraction: Option<f64>,
    running: bool,
}

struct AppModel {
    config: Config,
    games: Vec<Game>,
    favorites: HashSet<i64>,
    tags: HashMap<i64, Vec<String>>,
    favorites_only: bool,
    downloaded_only: bool,
    installed_only: bool,
    played_only: bool,
    unplayed_only: bool,
    installed_products: HashSet<i64>,
    playable_products: HashSet<i64>,
    downloaded_products: HashSet<i64>,
    downloaded_installer_products: HashSet<i64>,
    download_jobs: Vec<DownloadJobRecord>,
    windows_only: bool,
    linux_only: bool,
    macos_only: bool,
    cloud_saves_only: bool,
    achievements_only: bool,
    language_filter: Option<String>,
    genre_theme_filters: BTreeSet<String>,
    game_mode_filters: BTreeSet<String>,
    property_filters: BTreeSet<String>,
    card_width: i32,
    query: String,
    selected: Option<i64>,
    account_profile: Option<auth::Profile>,
    account_token: Option<auth::Token>,
    token_refresh_in_progress: bool,
    network_available: bool,
    owned_product_count: usize,
    online_synced_at: Option<i64>,
    product_activity: HashMap<i64, ProductActivity>,
    sidebar_sort_mode: SidebarSortMode,
    sidebar_playable_only: bool,
    collapsed_activity_sections: HashSet<ActivitySectionKey>,
    activity_sections: Vec<SidebarSection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ActivitySectionKey {
    Recent,
    Month { year: i32, month: u32 },
    Year(i32),
    NeverPlayed,
}

#[derive(Debug, Clone)]
struct SidebarGameEntry {
    product_id: i64,
    normalized_title: String,
    activity: ProductActivity,
    section: ActivitySectionKey,
}

#[derive(Debug, Clone)]
struct SidebarSection {
    key: ActivitySectionKey,
    label: String,
    members: Vec<i64>,
}

#[derive(Clone)]
struct DetailPageModel {
    product_id: i64,
    owned: bool,
    parent_id: Option<i64>,
    parent_title: Option<String>,
    parent_slug: Option<String>,
    slug: String,
    title: String,
    release_year: Option<i32>,
    kind: Option<String>,
    description: String,
    changelog: String,
    platform_label: String,
    features: Vec<String>,
    languages: Vec<String>,
    metadata: crate::domain::ProductMetadata,
    galaxy_builds: Vec<crate::domain::GalaxyBuild>,
    location: std::path::PathBuf,
    artwork: Option<std::path::PathBuf>,
    detail_artwork: Option<std::path::PathBuf>,
    hero_logo: Option<std::path::PathBuf>,
    icon: Option<std::path::PathBuf>,
    screenshots: Vec<Screenshot>,
    links: ExternalLinks,
    installers: Vec<LibraryFile>,
    patches: Vec<LibraryFile>,
    extras: Vec<LibraryFile>,
    remote_artifacts: Vec<RemoteArtifact>,
    dlcs: Vec<Dlc>,
    disk_usage: u64,
    favorite: Option<bool>,
}

#[derive(Clone)]
struct DownloadDialogProduct {
    product_id: i64,
    slug: String,
    parent_slug: Option<String>,
    title: String,
    artwork: Option<std::path::PathBuf>,
    groups: Vec<ArtifactGroup>,
    is_primary: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DialogArtifactState {
    Available,
    Downloaded,
    Busy,
    Resumable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GamePrimaryAction {
    Download,
    Install,
    DownloadUpdate,
    InstallUpdate,
    Play,
}

impl GamePrimaryAction {
    fn label(self) -> &'static str {
        match self {
            Self::Download => "Download",
            Self::Install => "Install",
            Self::DownloadUpdate => "Download Update",
            Self::InstallUpdate => "Install Update",
            Self::Play => "Play",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Download => "folder-download-symbolic",
            Self::Install => "system-run-symbolic",
            Self::DownloadUpdate => "folder-download-symbolic",
            Self::InstallUpdate => "system-run-symbolic",
            Self::Play => "media-playback-start-symbolic",
        }
    }
}

fn primary_action_for_state(
    installed: bool,
    installed_update: bool,
    backup_update: bool,
    current_installer_downloaded: bool,
    dlc_action: DlcActionState,
) -> GamePrimaryAction {
    let needs_download = backup_update
        || dlc_action.missing_download
        || (installed_update && !current_installer_downloaded);
    let needs_install =
        dlc_action.missing_install || (installed_update && current_installer_downloaded);

    if needs_download {
        GamePrimaryAction::DownloadUpdate
    } else if installed && needs_install {
        GamePrimaryAction::InstallUpdate
    } else if installed {
        GamePrimaryAction::Play
    } else if current_installer_downloaded {
        GamePrimaryAction::Install
    } else {
        GamePrimaryAction::Download
    }
}

#[derive(Clone)]
struct DownloadDialogRow {
    group: ArtifactGroup,
    check: gtk::CheckButton,
    state: DialogArtifactState,
}

struct DownloadDialogState {
    selected_products: HashSet<i64>,
    selected_operating_systems: BTreeSet<String>,
    selected_languages: BTreeSet<String>,
    selected_groups: HashSet<String>,
    include_extras: bool,
    include_patches: bool,
    applying: bool,
}

#[derive(Clone)]
struct DownloadDialogWidgets {
    rows: Vec<DownloadDialogRow>,
    products: Vec<DownloadDialogProduct>,
    warnings: HashMap<i64, gtk::Label>,
    product_content: HashMap<i64, gtk::Revealer>,
    category_expanders: HashMap<(i64, &'static str), gtk::Expander>,
    plan_boxes: HashMap<i64, gtk::Box>,
    product_toggles: HashMap<i64, gtk::CheckButton>,
    language_summary: gtk::Label,
    summary: gtk::Label,
    confirm: gtk::Button,
    authenticated: bool,
    online: bool,
    download_directory: std::path::PathBuf,
}

impl DetailPageModel {
    fn game(game: Game, favorite: bool) -> Self {
        let release_year = game.release_year();
        let platform_label = game.platform_label();
        Self {
            product_id: game.product_id,
            owned: true,
            parent_id: None,
            parent_title: None,
            parent_slug: None,
            slug: game.slug,
            title: game.title,
            release_year,
            kind: None,
            description: game.description,
            changelog: game.changelog,
            platform_label,
            features: game.features,
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
            patches: game.patches,
            extras: game.extras,
            remote_artifacts: game.remote_artifacts,
            dlcs: game.dlcs,
            disk_usage: game.disk_usage,
            favorite: Some(favorite),
        }
    }

    fn dlc(parent: &Game, dlc: Dlc) -> Self {
        use chrono::Datelike;
        let release_year = dlc.release_date.as_ref().map(|date| date.year());
        let kind = dlc.kind().to_owned();
        let platform_label = dlc.platform_label();
        Self {
            product_id: dlc.product_id,
            owned: dlc.owned,
            parent_id: Some(parent.product_id),
            parent_title: Some(parent.title.clone()),
            parent_slug: Some(parent.slug.clone()),
            slug: dlc.slug,
            title: dlc.title,
            release_year,
            kind: Some(kind),
            description: dlc.description,
            changelog: dlc.changelog,
            platform_label,
            features: Vec::new(),
            languages: dlc.languages,
            metadata: dlc.metadata,
            galaxy_builds: dlc.galaxy_builds,
            location: dlc.location,
            artwork: dlc.artwork,
            detail_artwork: dlc.detail_artwork,
            hero_logo: dlc.hero_logo,
            icon: dlc.icon,
            screenshots: dlc.screenshots,
            links: dlc.links,
            installers: dlc.installers,
            patches: Vec::new(),
            extras: dlc.extras,
            remote_artifacts: dlc.remote_artifacts,
            dlcs: parent.dlcs.clone(),
            disk_usage: dlc.disk_usage,
            favorite: None,
        }
    }
}

struct Widgets {
    window: adw::ApplicationWindow,
    status: gtk::Label,
    status_bar: gtk::Button,
    sync_spinner: gtk::Spinner,
    sync_status: gtk::Label,
    sync_progress: gtk::ProgressBar,
    download_artwork: gtk::Image,
    download_percent: gtk::Label,
    download_status_progress: gtk::ProgressBar,
    game_list: gtk::ListBox,
    home_grid: gtk::FlowBox,
    collections: gtk::Box,
    content: gtk::Stack,
    empty: adw::StatusPage,
    details: gtk::Box,
    details_scroll: gtk::ScrolledWindow,
    downloads: gtk::Box,
    search: gtk::SearchEntry,
    filter_chips: gtk::FlowBox,
    count: gtk::Label,
    sort_toggle: gtk::ToggleButton,
    playable_toggle: gtk::ToggleButton,
    filter_count: gtk::Label,
    filter_button: gtk::MenuButton,
    clear_filters: gtk::Button,
    favorite_filter: gtk::CheckButton,
    downloaded_filter: gtk::CheckButton,
    installed_filter: gtk::CheckButton,
    played_filter: gtk::CheckButton,
    unplayed_filter: gtk::CheckButton,
    windows_filter: gtk::CheckButton,
    linux_filter: gtk::CheckButton,
    macos_filter: gtk::CheckButton,
    cloud_saves_filter: gtk::CheckButton,
    achievements_filter: gtk::CheckButton,
    language_filter: gtk::DropDown,
    language_options: gtk::StringList,
    genre_theme_filter_box: gtk::Box,
    genre_theme_filter_label: gtk::Label,
    game_mode_filter_box: gtk::Box,
    game_mode_filter_label: gtk::Label,
    property_filter_box: gtk::Box,
    property_filter_label: gtk::Label,
    property_filter_search: gtk::SearchEntry,
    property_filter_chips: gtk::FlowBox,
    account_button_name: gtk::Label,
    header_network_icon: gtk::Image,
    header_network_slash: gtk::Label,
    header_network_button: gtk::Button,
    account_offline_indicator: gtk::Image,
    account_button_avatar: adw::Avatar,
    account_avatar: gtk::Picture,
    account_name: gtk::Label,
    account_details: gtk::Label,
    account_library_status: gtk::Label,
    account_connection_status: gtk::Label,
    sign_in: gtk::Button,
    reconnect: gtk::Button,
    sign_out: gtk::Button,
    account_popover: gtk::Popover,
}

pub fn install_css() {
    style::install(CSS);
}

fn find_named_descendant(root: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if widget.widget_name() == name {
            return Some(widget);
        }
        if let Some(found) = find_named_descendant(&widget, name) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn refresh_managed_detail_labels(window: &adw::ApplicationWindow, product_id: i64) {
    let Ok(store) = StateStore::open() else {
        return;
    };
    let Ok(files) = store.managed_files() else {
        return;
    };
    let present = files
        .iter()
        .filter(|file| file.product_id == product_id && file.present)
        .collect::<Vec<_>>();
    let mut paths = present
        .iter()
        .map(|file| file.path.clone())
        .collect::<HashSet<_>>();
    let mut bytes = present.iter().map(|file| file.size).sum::<u64>();
    let mut installers = present
        .iter()
        .filter(|file| file.kind == ArtifactKind::Installer)
        .count();
    if let Ok(jobs) = store.download_jobs() {
        for job in jobs
            .iter()
            .filter(|job| job.product_id == product_id && job.state == "complete")
        {
            let installer = job
                .artifacts
                .iter()
                .any(|artifact| artifact.kind == ArtifactKind::Installer);
            for path in job.completed_files.iter().filter(|path| path.is_file()) {
                if paths.insert(path.clone()) {
                    bytes += path.metadata().map_or(0, |metadata| metadata.len());
                    installers += usize::from(installer);
                }
            }
        }
    }
    let root = window.clone().upcast::<gtk::Widget>();
    if let Some(label) =
        find_named_descendant(&root, &format!("managed-files-summary-{product_id}"))
            .and_downcast::<gtk::Label>()
    {
        let available = label
            .text()
            .split(" · ")
            .next()
            .unwrap_or("Files available")
            .to_owned();
        label.set_label(&format!(
            "{available} · {installers} local installers · {} on disk",
            human_size(bytes)
        ));
    }
    if let Some(label) =
        find_named_descendant(&root, &format!("managed-product-subtitle-{product_id}"))
            .and_downcast::<gtk::Label>()
    {
        let text = label.text();
        let prefix = text
            .rsplit_once(" · ")
            .map_or(text.as_str(), |(prefix, _)| prefix);
        label.set_label(&format!("{prefix} · {}", human_size(bytes)));
    }
}

fn adjust_downloaded_collection_count(label: &gtk::Label, change: i32) {
    let text = label.text();
    let Some((fraction, suffix)) = text.split_once(' ') else {
        return;
    };
    let Some((downloaded, total)) = fraction.split_once('/') else {
        return;
    };
    let (Ok(downloaded), Ok(total)) = (downloaded.parse::<i32>(), total.parse::<i32>()) else {
        return;
    };
    label.set_label(&format!(
        "{}/{} {suffix}",
        (downloaded + change).clamp(0, total),
        total
    ));
}

impl Widgets {
    fn clone_refs(&self) -> Rc<Self> {
        Rc::new(Self {
            window: self.window.clone(),
            status: self.status.clone(),
            status_bar: self.status_bar.clone(),
            sync_spinner: self.sync_spinner.clone(),
            sync_status: self.sync_status.clone(),
            sync_progress: self.sync_progress.clone(),
            download_artwork: self.download_artwork.clone(),
            download_percent: self.download_percent.clone(),
            download_status_progress: self.download_status_progress.clone(),
            game_list: self.game_list.clone(),
            home_grid: self.home_grid.clone(),
            collections: self.collections.clone(),
            content: self.content.clone(),
            empty: self.empty.clone(),
            details: self.details.clone(),
            details_scroll: self.details_scroll.clone(),
            downloads: self.downloads.clone(),
            search: self.search.clone(),
            filter_chips: self.filter_chips.clone(),
            count: self.count.clone(),
            sort_toggle: self.sort_toggle.clone(),
            playable_toggle: self.playable_toggle.clone(),
            filter_count: self.filter_count.clone(),
            filter_button: self.filter_button.clone(),
            clear_filters: self.clear_filters.clone(),
            favorite_filter: self.favorite_filter.clone(),
            downloaded_filter: self.downloaded_filter.clone(),
            installed_filter: self.installed_filter.clone(),
            played_filter: self.played_filter.clone(),
            unplayed_filter: self.unplayed_filter.clone(),
            windows_filter: self.windows_filter.clone(),
            linux_filter: self.linux_filter.clone(),
            macos_filter: self.macos_filter.clone(),
            cloud_saves_filter: self.cloud_saves_filter.clone(),
            achievements_filter: self.achievements_filter.clone(),
            language_filter: self.language_filter.clone(),
            language_options: self.language_options.clone(),
            genre_theme_filter_box: self.genre_theme_filter_box.clone(),
            genre_theme_filter_label: self.genre_theme_filter_label.clone(),
            game_mode_filter_box: self.game_mode_filter_box.clone(),
            game_mode_filter_label: self.game_mode_filter_label.clone(),
            property_filter_box: self.property_filter_box.clone(),
            property_filter_label: self.property_filter_label.clone(),
            property_filter_search: self.property_filter_search.clone(),
            property_filter_chips: self.property_filter_chips.clone(),
            account_button_name: self.account_button_name.clone(),
            header_network_icon: self.header_network_icon.clone(),
            header_network_slash: self.header_network_slash.clone(),
            header_network_button: self.header_network_button.clone(),
            account_offline_indicator: self.account_offline_indicator.clone(),
            account_button_avatar: self.account_button_avatar.clone(),
            account_avatar: self.account_avatar.clone(),
            account_name: self.account_name.clone(),
            account_details: self.account_details.clone(),
            account_library_status: self.account_library_status.clone(),
            account_connection_status: self.account_connection_status.clone(),
            sign_in: self.sign_in.clone(),
            reconnect: self.reconnect.clone(),
            sign_out: self.sign_out.clone(),
            account_popover: self.account_popover.clone(),
        })
    }
}

fn apply_theme(theme: crate::config::Theme) {
    let manager = adw::StyleManager::default();
    manager.set_color_scheme(match theme {
        crate::config::Theme::System => adw::ColorScheme::Default,
        crate::config::Theme::Light => adw::ColorScheme::ForceLight,
        crate::config::Theme::Dark => adw::ColorScheme::ForceDark,
    });
}

const CSS: &str = r#"
.header-app-icon { margin: 0 5px; }
.library-sidebar { background: alpha(@window_bg_color, .96); border-right: 1px solid alpha(@borders, .65); }
.library-sidebar row { min-height: 0; margin: 0; padding: 0; }
.library-sidebar entry, .library-sidebar menubutton > button { min-height: 30px; padding-top: 2px; padding-bottom: 2px; border-radius: 3px; }
.navigation-sidebar row { border-radius: 0; }
.navigation-sidebar row:selected { border-radius: 0; }
.library-view-buttons { margin: 6px 5px 0; }
.home-button { min-height: 32px; padding: 3px 10px; border-radius: 3px; font-weight: 700; }
.collections-button { min-width: 34px; min-height: 32px; padding: 3px; border-radius: 3px; }
.sidebar-toolbar { border-spacing: 4px; }
.sidebar-icon-toggle { min-width: 34px; min-height: 32px; padding: 3px; border-radius: 3px; }
.sidebar-icon-toggle-active { background: alpha(@accent_bg_color, .28); color: @accent_color; }
.activity-section-row { min-height: 24px; background: alpha(@window_fg_color, .055); color: @window_fg_color; }
.activity-section-row:hover { background: alpha(@window_fg_color, .09); }
.activity-section-row > box { min-height: 24px; }
.activity-section-title { font-size: .78em; font-weight: 800; letter-spacing: .08em; color: alpha(@window_fg_color, .72); }
.activity-section-count { font-size: .78em; color: alpha(@window_fg_color, .55); }
.activity-section-disclosure { min-width: 10px; color: alpha(@window_fg_color, .65); }
.collections-page { padding: 28px; }
.collections-heading { font-size: 1.35em; letter-spacing: .10em; color: alpha(@window_fg_color, .82); }
.collections-grid { margin-top: 18px; }
.collection-card { min-width: 174px; min-height: 174px; padding: 0; border-radius: 4px; }
.collection-card-overlay { padding: 16px; background: linear-gradient(to bottom, alpha(#17202a, .24), alpha(#17202a, .90)); }
.collection-card-title { color: white; font-weight: 800; letter-spacing: .10em; }
.collection-card-count { color: alpha(white, .70); font-size: 1.1em; }
.collection-games-heading { margin-bottom: 14px; }
.account-avatar-small { min-width: 26px; min-height: 26px; border-radius: 999px; background: alpha(@window_fg_color, .10); }
.account-avatar { min-width: 64px; min-height: 64px; border-radius: 999px; background: alpha(@window_fg_color, .10); }
.account-library-status { padding: 9px 11px; border-radius: 7px; background: alpha(@accent_bg_color, .10); color: alpha(@window_fg_color, .82); }
.game-card { background: #303030; border-radius: 8px; box-shadow: 0 2px 8px alpha(black, .30); }
.game-card:hover { box-shadow: 0 5px 16px alpha(black, .42); }
.game-grid flowboxchild,
.game-grid flowboxchild:hover,
.game-grid flowboxchild:active,
.game-grid flowboxchild:selected { background: transparent; box-shadow: none; outline: none; padding: 0; }
.game-state-update label { color: #62a8e5; }
.game-state-partial-backup label { color: #d98b45; }
.game-state-backup label { color: #d5b36a; }
.game-state-pending label { color: #78aeed; }
.portrait-frame, .hero-card { border-radius: 10px; background: #20242b; }
.game-card .hero-card { border-radius: 8px 8px 0 0; }
.card-caption { background: #303030; color: white; border-radius: 0 0 8px 8px; padding: 9px 10px; }
.game-icon { border-radius: 2px; background: #20242b; }
.detail-icon { border-radius: 10px; background: #20242b; }
.detail-hero-container { background: #171a20; }
.detail-hero { background: #171a20; }
.detail-hero-content { padding: 82px 36px 24px; background: linear-gradient(to bottom, alpha(#11151c, 0), alpha(#11151c, .88) 72%, alpha(#11151c, .97)); }
.detail-hero-title { color: white; text-shadow: 0 2px 5px black, 0 0 18px alpha(black, .95); }
.detail-hero-logo { margin-bottom: 2px; }
.detail-action-bar { min-height: 50px; padding: 10px 36px; background: linear-gradient(to bottom, #202b37, #17202a); border-top: 1px solid alpha(white, .06); border-bottom: 1px solid alpha(black, .55); }
.detail-action-metadata { color: alpha(white, .62); }
.steam-primary-action { min-width: 180px; min-height: 42px; border-radius: 2px 0 0 2px; background: #47c92f; color: white; font-weight: 700; box-shadow: none; }
.steam-primary-action:hover { background: #58d83b; }
.steam-primary-action.operational-action { background: #287bc1; }
.steam-primary-action.operational-action:hover { background: #3293dc; }
.steam-primary-action.operational-action:disabled { background: #287bc1; color: alpha(white, .65); opacity: .72; }
.detail-primary-actions.download-state .steam-primary-action { background: #287bc1; }
.detail-primary-actions.download-state .steam-primary-action:hover { background: #3293dc; }
.context-primary-action { min-width: 0; min-height: 34px; margin: 2px 0; padding: 0 14px; border-radius: 2px; }
.context-primary-action.download-state { background: #287bc1; }
.context-primary-action.download-state:hover { background: #3293dc; }
.game-management-popover .context-primary-action,
.game-management-popover .context-primary-action label,
.game-management-popover .context-primary-action image { color: white; }
.game-management-popover .context-menu-item,
.game-management-popover .context-menu-item label,
.game-management-popover .context-menu-item image { color: @window_fg_color; }
.hero-transfer-status { min-width: 170px; }
.hero-transfer-heading { color: alpha(white, .68); font-size: .82em; font-weight: 700; letter-spacing: .08em; }
.hero-transfer-detail { color: alpha(white, .52); font-size: .82em; }
.hero-transfer-progress trough { min-height: 3px; background: alpha(black, .65); }
.hero-transfer-progress progress { background: #2f8ed8; }
.detail-activity-stat { min-width: 108px; margin-left: 8px; }
.detail-activity-heading { color: alpha(white, .58); font-size: .82em; letter-spacing: .08em; }
.steam-utility-action { min-width: 44px; min-height: 44px; border-radius: 2px; background: alpha(#405066, .72); color: alpha(white, .88); box-shadow: none; }
.steam-utility-action:hover { background: alpha(#53657d, .92); color: white; }
.card-title { font-weight: 700; font-size: 1.05em; }
.game-title { font-weight: 800; font-size: 2em; }
.section-title { font-weight: 800; font-size: 1.3em; }
.filter-heading { color: @accent_color; font-weight: 800; font-size: .85em; }
.filter-count { background: @accent_bg_color; color: @accent_fg_color; border-radius: 999px; padding: 1px 7px; font-weight: 800; }
.clear-filters { min-width: 28px; min-height: 28px; padding: 2px; border-radius: 999px; }
.active-filter-chips { padding: 2px 0; }
.active-filter-chips flowboxchild { min-width: 0; padding: 0; background: transparent; box-shadow: none; }
.active-filter-chip { min-height: 26px; padding: 2px 7px; border-radius: 3px; background: @accent_bg_color; color: @accent_fg_color; box-shadow: none; }
.active-filter-chip:hover { background: shade(@accent_bg_color, 1.08); }
.active-filter-chip label { font-size: .82em; }
.sidebar-filter-button { min-width: 34px; min-height: 30px; padding: 0; border-radius: 3px; }
.sidebar-filter-button-active > button { background: @accent_bg_color; color: @accent_fg_color; }
.sidebar-filter-popover contents { padding: 3px; border: 0; border-radius: 3px; background: #3d3e45; }
.metadata-filter-popover contents { padding: 0; border-radius: 3px; }
.library-filter-panel { background: #3d3e45; color: #eeeeef; }
.library-filter-panel .section-title { font-size: 1.45em; }
.library-filter-panel .filter-heading { margin-top: 4px; color: #1a9fff; font-size: .78em; letter-spacing: .04em; }
.library-filter-panel checkbutton { min-height: 20px; }
.library-filter-panel dropdown > button,
.library-filter-panel menubutton > button { min-height: 34px; border-radius: 2px; background: alpha(white, .11); }
.inline-metadata-filter { margin-top: 2px; }
.inline-metadata-filter > checkbutton { min-height: 19px; }
.inline-filter-scroll { background: transparent; }
.inline-filter-scroll scrollbar { min-width: 5px; }
.property-filter-search { min-height: 28px; }
.property-filter-suggestions contents { padding: 4px; border-radius: 2px; background: #55565e; }
.property-filter-suggestions checkbutton { min-height: 23px; }
.property-filter-chips flowboxchild { min-width: 0; padding: 0; background: transparent; box-shadow: none; }
.filter-bottom-row { margin: 14px 2px 4px; }
.library-filter-scroll { background: #3d3e45; }
.library-filter-scroll scrollbar { min-width: 6px; }
.header-network-status { min-width: 24px; min-height: 24px; margin: 4px; }
.header-network-slash { font-size: 25px; font-weight: 900; margin-top: -2px; }
.header-network-button { min-width: 34px; min-height: 28px; padding: 0 5px; border-radius: 4px; }
.header-network-button.success { background: #4e9a51; }
.header-network-button.warning { background: #c58b16; }
.header-network-button.error { background: #c01c28; }
.header-network-status image, .header-network-slash { color: white; }
.compact-account-button { min-height: 28px; padding: 0 7px; border-radius: 4px; }
.compact-account-button box { margin: 0; }
.body-copy { line-height: 1.35; }
.long-text-expander { font-weight: 700; }
.long-text-scroll { background: alpha(@card_bg_color, .65); border-radius: 8px; }
.detail-tabs { margin: 0; }
.game-navigation-shell { min-height: 32px; background: #1b2530; border-bottom: 1px solid alpha(black, .65); }
.game-navigation { min-height: 32px; background: transparent; border: 0; padding: 0 4px; }
.game-navigation button { min-height: 32px; padding: 3px 15px; border-radius: 0; background: transparent; color: alpha(white, .60); box-shadow: none; }
.game-navigation button:hover { color: white; background: alpha(#67788d, .24); }
.game-navigation stackswitcher button:checked { color: white; background: #536171; box-shadow: inset 0 -2px #66c0f4; }
.game-navigation .navigation-overflow { min-width: 34px; padding-left: 8px; padding-right: 8px; }
.square-action { min-width: 40px; min-height: 40px; padding: 0; border-radius: 5px; }
.detail-download-action { min-height: 42px; padding: 0 14px; }
.detail-action-menu { min-width: 34px; min-height: 42px; padding: 0; margin-left: 3px; }
.detail-action-menu > button { min-width: 34px; min-height: 42px; padding: 0; border-radius: 0 2px 2px 0; background: #47c92f; color: white; box-shadow: none; }
.detail-action-menu > button:hover { background: #58d83b; }
.detail-primary-actions.download-state .detail-action-menu > button { background: #287bc1; }
.detail-primary-actions.download-state .detail-action-menu > button:hover { background: #3293dc; }
.detail-primary-actions.operational-state .detail-action-menu > button { background: #287bc1; }
.detail-primary-actions.operational-state .detail-action-menu > button:hover { background: #3293dc; }
.folder-action { color: @accent_color; }
.download-selector-card { padding: 14px; border-radius: 10px; background: alpha(@card_bg_color, .68); border: 1px solid alpha(@borders, .55); }
.download-selector-card checkbutton.section-title:disabled { opacity: 1; }
.settings-sidebar { padding: 10px 8px; background: alpha(@headerbar_bg_color, .94); border-right: 1px solid alpha(@borders, .65); }
.settings-sidebar-title { margin: 12px 12px 18px; color: @accent_color; font-weight: 800; letter-spacing: .06em; }
.settings-navigation { background: transparent; }
.settings-navigation row { min-height: 42px; margin: 1px 0; border-radius: 3px; }
.settings-navigation row > box { padding: 8px 12px; }
.settings-navigation row:selected { background: alpha(@accent_bg_color, .24); color: @window_fg_color; }
.settings-navigation image { min-width: 22px; color: alpha(@window_fg_color, .72); }
.settings-account-avatar { margin: 8px 12px 8px 0; border-radius: 5px; background: alpha(@window_fg_color, .10); }
.storage-settings-page { padding: 24px 28px 18px; }
.storage-library-menu > button { min-height: 44px; padding: 5px 12px; border-radius: 4px; background: alpha(@card_bg_color, .82); }
.storage-library-menu popover contents { padding: 8px; min-width: 560px; }
.storage-library-choice { min-height: 42px; padding: 5px 10px; border-radius: 2px; }
.storage-library-path { font-weight: 700; }
.storage-default-library { color: #f2b82e; }
.storage-installer-library { color: #1a9eee; }
.install-choice-menu > button { padding: 8px 10px; min-height: 68px; border-radius: 4px; }
.install-choice-menu popover contents { padding: 6px; min-width: 540px; }
.install-dlc-menu { margin-top: 2px; }
.install-dlc-menu > button { padding: 4px 10px; min-height: 34px; border-radius: 4px; }
.install-dlc-menu popover contents { padding: 4px; min-width: 540px; }
.install-footer-switch { background: transparent; padding: 0; min-height: 34px; }
.install-footer-switch > box { padding: 0; }
.installer-prompt-context { padding: 10px; background: alpha(black, .28); border: 1px solid alpha(@borders, .55); border-radius: 6px; }
.install-choice-row { padding: 6px 8px; min-height: 64px; }
.install-choice-size, .install-library-free { color: alpha(@window_fg_color, .72); font-weight: 700; }
.install-library-list { border-radius: 4px; background: alpha(@card_bg_color, .82); }
.install-library-list row { min-height: 46px; padding: 5px 12px; }
.install-library-list row:selected { background: @accent_bg_color; color: @accent_fg_color; }
.install-library-path { font-weight: 700; }
.storage-capacity-label { font-weight: 700; }
.storage-path-label { margin: 4px 8px 0; color: alpha(@window_fg_color, .55); font-weight: 700; }
.storage-usage-bar { border-radius: 6px; background: alpha(@window_fg_color, .10); }
.storage-legend { margin-top: 1px; }
.storage-legend-dot { min-width: 8px; min-height: 8px; border-radius: 999px; }
.storage-legend-dot.storage-games { background: #1a9eee; }
.storage-legend-dot.storage-installers { background: #ba61d1; }
.storage-legend-dot.storage-extras { background: #40b064; }
.storage-legend-dot.storage-others { background: #f2b82e; }
.storage-legend-dot.storage-free { background: #59606b; }
.storage-legend-title { color: alpha(@window_fg_color, .82); font-weight: 700; font-size: .84em; }
.storage-legend-size { color: alpha(@window_fg_color, .62); font-size: .84em; }
.storage-game-list { background: transparent; }
.storage-game-row { min-height: 66px; padding: 8px 10px; border-bottom: 1px solid alpha(@borders, .30); }
.storage-game-row picture { border-radius: 2px; background: alpha(@window_fg_color, .08); }
.storage-game-size { min-width: 90px; font-weight: 700; }
.compact-download-preferences { padding: 12px; }
.compact-preference-toggle { min-height: 28px; padding: 2px 14px; }
.compact-preference-menu { min-height: 28px; min-width: 150px; padding: 2px 10px; }
.download-selector-expander { padding: 2px 0; }
.download-selector-expander title { padding: 6px 0; }
.download-selector-row { padding: 7px 4px; border-top: 1px solid alpha(@borders, .30); }
.download-selector-footer { padding: 12px 16px; border-top: 1px solid alpha(@borders, .55); background: @headerbar_bg_color; }
.download-plan-expander { padding: 2px 0; }
.compact-download-plan-row { padding: 9px 10px; border-radius: 7px; background: alpha(@window_fg_color, .045); }
.compact-optional-plan { border-radius: 7px; background: alpha(@window_fg_color, .045); }
.compact-optional-plan:hover { background: alpha(@window_fg_color, .075); }
.compact-optional-plan .compact-download-plan-row { background: transparent; }
.compact-optional-items { padding: 0 14px 10px 46px; }
.compact-selector-action { min-height: 24px; padding: 1px 7px; }
.file-management-card { padding: 18px; border-radius: 10px; background: alpha(@card_bg_color, .76); border: 1px solid alpha(@borders, .55); }
.compact-file-action { min-height: 26px; padding: 2px 10px; }
.file-collection { border-radius: 10px; background: alpha(@card_bg_color, .58); border: 1px solid alpha(@borders, .55); }
.file-collection-header { padding: 12px 14px; border-bottom: 1px solid alpha(@borders, .45); }
.file-collection-header .section-title { font-size: 1.08em; }
.file-collection-header .square-action { min-width: 34px; min-height: 34px; }
.artifact-identity { min-width: 34px; }
.artifact-os-icon { min-width: 22px; min-height: 22px; }
.artifact-os-mark { font-size: 19px; font-weight: 700; }
.artifact-language-flag { font-size: 14px; margin-top: 1px; }
.file-row { padding: 10px 14px; border-bottom: 1px solid alpha(@borders, .30); }
.file-row:hover { background: alpha(@accent_bg_color, .07); }
.file-name { font-weight: 650; }
.file-empty { padding: 14px; color: alpha(@window_fg_color, .58); }
.dlc-catalog { background: alpha(@card_bg_color, .35); }
.dlc-catalog-row { padding: 12px; background: alpha(#183044, .40); border-bottom: 1px solid alpha(@borders, .45); }
.dlc-catalog-row:hover { background: alpha(@accent_bg_color, .16); }
.dlc-catalog-title { font-weight: 800; letter-spacing: .04em; }
.dlc-catalog-summary { color: alpha(@window_fg_color, .82); }
.dlc-ownership-badge { padding: 3px 9px; font-size: .72em; font-weight: 800; color: white; }
.dlc-ownership-badge.in-library { background: #168dcc; }
.dlc-ownership-badge.not-owned { background: #b93648; }
.dlc-hero { border-radius: 10px; background: #171a20; }
.tag-chip { background: alpha(@accent_bg_color, .18); border-radius: 999px; padding: 6px 10px; }
.screenshot-thumbnail { padding: 0; min-width: 210px; min-height: 118px; border-radius: 8px; }
.thumbnail-scroll-button { min-width: 38px; min-height: 118px; padding: 0; border-radius: 8px; background: alpha(#111820, .88); color: white; box-shadow: 0 2px 8px alpha(black, .45); }
.screenshot-gallery { background: alpha(black, .96); }
.gallery-hit-area,
.gallery-hit-area:hover,
.gallery-hit-area:active,
.gallery-hit-area:focus { min-width: 0; min-height: 0; border-radius: 0; background: transparent; box-shadow: none; outline: none; color: alpha(white, .72); }
.gallery-controls { background: alpha(#111820, .90); border-radius: 999px; padding: 7px 10px; color: white; }
.gallery-close { margin: 16px; min-width: 38px; min-height: 38px; border-radius: 999px; background: alpha(#111820, .90); color: white; }
.application-status-bar { border-radius: 0; border: 0; border-top: 1px solid alpha(@borders, .55); padding: 4px 14px; min-height: 38px; background: @headerbar_bg_color; }
.application-status-bar:hover { background: alpha(@accent_bg_color, .10); }
.download-status-icon { min-width: 24px; min-height: 24px; }
.downloads-title { font-size: 1.8em; font-weight: 800; }
.download-section-heading { margin-top: 8px; }
.download-section-heading separator { margin-top: 10px; }
.downloads-empty { padding: 6px 14px 24px; color: alpha(@window_fg_color, .60); }
.download-active-card { padding: 18px; background: alpha(@card_bg_color, .75); border: 1px solid alpha(@borders, .55); border-radius: 10px; }
.download-queue-row { padding: 12px; border-bottom: 1px solid alpha(@borders, .35); }
"#;

#[cfg(test)]
mod historical_filename_tests {
    use super::*;

    #[test]
    fn makes_multipart_installer_names_human_readable() {
        let file = LibraryFile {
            name: String::new(),
            path: std::path::PathBuf::from(
                "/downloads/baldurs_gate_iii/installer/windows/english/setup_baldurs_gate_3_release_-_v4.1.1.7209685_-_patch_patch8_hotfix8_(64bit)_(89470)-12.bin",
            ),
            size: 0,
        };
        assert_eq!(
            historical_artifact_title(&file, "Baldur's Gate 3"),
            "Baldur's Gate 3 — release — v4.1.1.7209685 — Patch 8 hotfix8"
        );
        assert_eq!(
            historical_path_metadata(&file, std::path::Path::new("/downloads")).as_deref(),
            Some("Windows · English")
        );
    }

    #[test]
    fn replaces_generic_gog_installer_titles_with_the_product_title() {
        let artifact = RemoteArtifact {
            product_id: 1546956723,
            kind: ArtifactKind::Installer,
            name: "DLC".into(),
            language: Some("English".into()),
            operating_system: Some("linux".into()),
            version: Some("1.14.38".into()),
            release_date: None,
            size_label: Some("561 MB".into()),
            size_bytes: Some(561_000_000),
            part_number: None,
            part_count: None,
            download_path: "/downloads/moonlighter_between_dimensions/en3installer0".into(),
            provider_group_id: None,
            provider_file_id: None,
            provider_category: None,
        };
        assert_eq!(
            artifact_display_title(&artifact, "Moonlighter - Between Dimensions"),
            "Moonlighter - Between Dimensions"
        );
    }
}

#[cfg(test)]
mod primary_action_tests {
    use super::*;

    #[test]
    fn installed_game_without_pending_work_plays() {
        assert_eq!(
            primary_action_for_state(true, false, false, true, DlcActionState::default()),
            GamePrimaryAction::Play
        );
    }

    #[test]
    fn downloaded_update_is_installed_while_missing_update_is_downloaded() {
        assert_eq!(
            primary_action_for_state(true, true, false, true, DlcActionState::default()),
            GamePrimaryAction::InstallUpdate
        );
        assert_eq!(
            primary_action_for_state(true, true, false, false, DlcActionState::default()),
            GamePrimaryAction::DownloadUpdate
        );
    }

    #[test]
    fn uninstalled_game_never_resolves_to_play_or_install_update() {
        assert_eq!(
            primary_action_for_state(false, false, false, true, DlcActionState::default()),
            GamePrimaryAction::Install
        );
        assert_eq!(
            primary_action_for_state(
                false,
                false,
                false,
                true,
                DlcActionState {
                    missing_download: false,
                    missing_install: true,
                },
            ),
            GamePrimaryAction::Install
        );
    }

    #[test]
    fn missing_dlc_files_take_priority_over_installing_downloaded_content() {
        assert_eq!(
            primary_action_for_state(
                true,
                false,
                false,
                true,
                DlcActionState {
                    missing_download: true,
                    missing_install: true,
                },
            ),
            GamePrimaryAction::DownloadUpdate
        );
    }
}

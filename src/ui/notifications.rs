use super::*;
use std::time::Instant;

const LIMIT: usize = 200;
const COMPACT_DURATION: Duration = Duration::from_secs(10);

#[derive(Default)]
struct History {
    entries: VecDeque<String>,
    revision: u64,
    deadline: Option<Instant>,
}

impl History {
    fn push(&mut self, text: String, now: Instant) -> bool {
        if text.trim().is_empty() || self.entries.back() == Some(&text) {
            return false;
        }
        self.entries.push_back(text);
        if self.entries.len() > LIMIT {
            self.entries.pop_front();
        }
        self.revision = self.revision.wrapping_add(1);
        self.deadline = Some(now + COMPACT_DURATION);
        true
    }

    fn expired(&self, revision: u64, now: Instant) -> bool {
        self.revision == revision && self.deadline.is_some_and(|deadline| now >= deadline)
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.deadline = None;
        self.revision = self.revision.wrapping_add(1);
    }
}

#[derive(Clone)]
pub(super) struct Notifications {
    pub root: gtk::Box,
    history: Rc<RefCell<History>>,
    compact: gtk::Label,
    latest: gtk::Label,
    hover: gtk::Popover,
    modal_list: Rc<RefCell<Option<glib::WeakRef<gtk::Box>>>>,
}

impl Notifications {
    pub fn new(window: &adw::ApplicationWindow, compact: &gtk::Label) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        root.set_valign(gtk::Align::Center);
        root.set_halign(gtk::Align::End);
        compact.set_single_line_mode(true);
        compact.set_visible(false);
        root.append(compact);
        let button = gtk::Button::from_icon_name("notifications-symbolic");
        button.set_widget_name("footer-notifications");
        button.update_property(&[gtk::accessible::Property::Label("Notifications")]);
        button.add_css_class("flat");
        root.append(&button);
        let history = Rc::new(RefCell::new(History::default()));
        let modal_list = Rc::new(RefCell::new(None::<glib::WeakRef<gtk::Box>>));
        let latest = gtk::Label::new(Some("No notifications this session"));
        latest.set_wrap(true);
        latest.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        latest.set_xalign(0.0);
        latest.set_max_width_chars(65);
        latest.set_margin_top(10);
        latest.set_margin_bottom(10);
        latest.set_margin_start(12);
        latest.set_margin_end(12);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_width(320)
            .max_content_width(520)
            .propagate_natural_width(true)
            .max_content_height(320)
            .propagate_natural_height(true)
            .child(&latest)
            .build();
        let hover = gtk::Popover::new();
        hover.set_widget_name("latest-notification-popover");
        hover.set_autohide(false);
        hover.set_focusable(false);
        hover.set_position(gtk::PositionType::Top);
        hover.set_child(Some(&scroll));
        hover.set_parent(&button);
        let hovering = Rc::new(std::cell::Cell::new(false));
        for widget in [
            button.clone().upcast::<gtk::Widget>(),
            hover.clone().upcast(),
        ] {
            let motion = gtk::EventControllerMotion::new();
            motion.connect_enter({
                let hover = hover.downgrade();
                let hovering = hovering.clone();
                move |_, _, _| {
                    hovering.set(true);
                    if let Some(hover) = hover.upgrade() {
                        hover.popup();
                    }
                }
            });
            motion.connect_leave({
                let hover = hover.downgrade();
                let hovering = hovering.clone();
                move |_| {
                    hovering.set(false);
                    let hover = hover.clone();
                    let hovering = hovering.clone();
                    glib::timeout_add_local_once(Duration::from_millis(100), move || {
                        if !hovering.get()
                            && let Some(hover) = hover.upgrade()
                        {
                            hover.popdown();
                        }
                    });
                }
            });
            widget.add_controller(motion);
        }
        button.connect_destroy({
            let hover = hover.downgrade();
            move |_| {
                if let Some(hover) = hover.upgrade()
                    && hover.parent().is_some()
                {
                    hover.unparent();
                }
            }
        });
        button.connect_clicked({
            let window = window.downgrade();
            let history = history.clone();
            let modal_list = modal_list.clone();
            let hover = hover.clone();
            move |_| {
                let Some(window) = window.upgrade() else {
                    return;
                };
                hover.popdown();
                present_history(&window, &history, &modal_list);
            }
        });
        compact.connect_label_notify({
            let history = history.clone();
            let modal_list = modal_list.clone();
            let latest = latest.clone();
            move |compact| {
                let text = compact.label().to_string();
                if !history.borrow_mut().push(text.clone(), Instant::now()) {
                    return;
                }
                latest.set_label(&text);
                compact.set_visible(true);
                if let Some(list) = modal_list.borrow().as_ref().and_then(|list| list.upgrade()) {
                    prepend_notification(&list, &text);
                }
                let revision = history.borrow().revision;
                let history = history.clone();
                let compact = compact.downgrade();
                glib::timeout_add_local_once(COMPACT_DURATION, move || {
                    if history.borrow().expired(revision, Instant::now())
                        && let Some(compact) = compact.upgrade()
                    {
                        compact.set_visible(false);
                    }
                });
            }
        });
        Self {
            root,
            history,
            compact: compact.clone(),
            latest,
            hover,
            modal_list,
        }
    }

    pub fn clear(&self) {
        self.history.borrow_mut().clear();
        self.compact.set_label("");
        self.compact.set_visible(false);
        self.latest.set_label("No notifications this session");
        self.hover.popdown();
        if let Some(list) = self
            .modal_list
            .borrow()
            .as_ref()
            .and_then(|list| list.upgrade())
        {
            render_history(&list, &self.history.borrow());
        }
    }

    // Called only by a direct user action; background failures just update history.
    pub fn show_message(&self, window: &adw::ApplicationWindow, message: &str) {
        self.compact.set_label(message);
        self.hover.popdown();
        present_history(window, &self.history, &self.modal_list);
    }
}

fn present_history(
    window: &adw::ApplicationWindow,
    history: &Rc<RefCell<History>>,
    modal_list: &Rc<RefCell<Option<glib::WeakRef<gtk::Box>>>>,
) {
    let dialog = adw::Dialog::builder()
        .title("Notifications")
        .content_width(640)
        .content_height(480)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.append(&adw::HeaderBar::new());
    let hint = gtk::Label::new(Some(
        "Latest 200 notifications from this session. Live progress is not saved.",
    ));
    hint.set_wrap(true);
    hint.set_margin_start(16);
    hint.set_margin_end(16);
    content.append(&hint);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 10);
    list.set_widget_name("notification-history");
    list.set_margin_top(8);
    list.set_margin_bottom(16);
    list.set_margin_start(16);
    list.set_margin_end(16);
    render_history(&list, &history.borrow());
    *modal_list.borrow_mut() = Some(list.downgrade());
    content.append(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build(),
    );
    dialog.set_child(Some(&content));
    dialog.present(Some(window));
}

pub(super) fn failure_message(context: &str, error: &str) -> String {
    let mut redact_next = false;
    let mut safe = String::new();
    for part in error.split_inclusive(char::is_whitespace) {
        let word = part.trim_end_matches(char::is_whitespace);
        let whitespace = &part[word.len()..];
        let lower = word.to_ascii_lowercase();
        let key = lower.trim_start_matches(['\'', '"', '(', '{', '[']);
        let credential = [
            "access_token",
            "refresh_token",
            "client_secret",
            "authorization",
            "password",
            "token",
        ]
        .iter()
        .find_map(|key_name| {
            key.strip_prefix(key_name)
                .filter(|rest| rest.is_empty() || rest.starts_with(['=', ':', '\'', '"']))
        });
        if lower.contains("https://") || lower.contains("http://") {
            safe.push_str("[URL redacted]");
            redact_next = false;
        } else if let Some(rest) = credential {
            safe.push_str("[credential redacted]");
            let value = rest.trim_matches(['=', ':', '\'', '"', ',', '}']);
            redact_next = value.is_empty() || value == "bearer";
        } else if redact_next {
            safe.push_str("[redacted]");
            redact_next = matches!(lower.as_str(), "=" | ":" | "bearer");
        } else {
            safe.push_str(word);
        }
        safe.push_str(whitespace);
    }
    format!("{context}\n{safe}")
}

fn render_history(list: &gtk::Box, history: &History) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    if history.entries.is_empty() {
        let empty = gtk::Label::new(Some("No notifications this session"));
        empty.set_widget_name("empty-notifications");
        list.append(&empty);
    }
    for text in &history.entries {
        prepend_notification(list, text);
    }
}

fn prepend_notification(list: &gtk::Box, text: &str) {
    if let Some(child) = list.first_child()
        && child.widget_name() == "empty-notifications"
    {
        list.remove(&child);
    }
    let row = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let label = gtk::Label::new(Some(text));
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_selectable(true);
    label.set_xalign(0.0);
    row.append(&label);
    row.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    list.prepend(&row);
    if list.observe_children().n_items() > LIMIT as u32
        && let Some(last) = list.last_child()
    {
        list.remove(&last);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires private HOME/all XDG/TMP, display and D-Bus; harness rejects GTK lifecycle warnings"]
    fn notification_popover_follows_anchor_lifetime_and_survives_remapping() {
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
                    .starts_with("/tmp/ludomere-p362-"),
                "{key}"
            );
        }
        adw::init().unwrap();
        #[track_caller]
        fn wait(check: impl Fn() -> bool) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !check() && Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        fn pump(duration: Duration) {
            let deadline = Instant::now() + duration;
            while Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.NotificationLifecycleTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        for present in [false, true] {
            let window = adw::ApplicationWindow::builder()
                .application(&app)
                .default_width(600)
                .default_height(300)
                .build();
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let focus = gtk::Entry::new();
            content.append(&focus);
            let compact = gtk::Label::new(None);
            let notifications = Notifications::new(&window, &compact);
            content.append(&notifications.root);
            window.set_content(Some(&content));
            let button = notifications
                .root
                .last_child()
                .unwrap()
                .downcast::<gtk::Button>()
                .unwrap();
            let weak_root = notifications.root.downgrade();
            let weak_button = button.downgrade();
            let weak_hover = notifications.hover.downgrade();
            assert_eq!(
                notifications.hover.parent().as_ref(),
                Some(button.upcast_ref())
            );
            assert!(!notifications.root.is_realized());
            if present {
                window.present();
                wait(|| notifications.root.is_mapped());
                assert!(focus.grab_focus());
                let original_focus = gtk::prelude::GtkWindowExt::focus(&window);
                assert!(original_focus.is_some());
                let motion = button
                    .observe_controllers()
                    .iter::<glib::Object>()
                    .filter_map(Result::ok)
                    .find_map(|object| object.downcast::<gtk::EventControllerMotion>().ok())
                    .unwrap();
                motion.emit_by_name::<()>("enter", &[&0.0_f64, &0.0_f64]);
                wait(|| notifications.hover.is_mapped());
                assert_eq!(gtk::prelude::GtkWindowExt::focus(&window), original_focus);
                assert!(window.visible_dialog().is_none());
                notifications.hover.popdown();
                wait(|| !notifications.hover.is_mapped());

                // Container removal really unrealizes the root; no lifecycle signals are fabricated.
                content.remove(&notifications.root);
                assert!(!notifications.root.is_realized());
                assert_eq!(
                    notifications.hover.parent().as_ref(),
                    Some(button.upcast_ref())
                );
                content.append(&notifications.root);
                wait(|| notifications.root.is_mapped());
                motion.emit_by_name::<()>("enter", &[&0.0_f64, &0.0_f64]);
                wait(|| notifications.hover.is_mapped());
                notifications.hover.popdown();
                wait(|| !notifications.hover.is_mapped());

                window.set_visible(false);
                wait(|| !notifications.root.is_mapped());
                assert_eq!(
                    notifications.hover.parent().as_ref(),
                    Some(button.upcast_ref())
                );
                window.present();
                wait(|| notifications.root.is_mapped());
                assert!(focus.grab_focus());
                let original_focus = gtk::prelude::GtkWindowExt::focus(&window);
                motion.emit_by_name::<()>("enter", &[&0.0_f64, &0.0_f64]);
                wait(|| notifications.hover.is_mapped());
                assert_eq!(gtk::prelude::GtkWindowExt::focus(&window), original_focus);
                assert!(window.visible_dialog().is_none());
                assert_eq!(window.content().as_ref(), Some(content.upcast_ref()));

                // Queue the production weak leave timeout, then destroy before it can run.
                motion.emit_by_name::<()>("leave", &[]);
            }
            window.destroy();
            drop(button);
            drop(notifications);
            drop(compact);
            drop(focus);
            drop(content);
            drop(window);
            wait(|| {
                weak_root.upgrade().is_none()
                    && weak_button.upgrade().is_none()
                    && weak_hover.upgrade().is_none()
            });
            pump(Duration::from_millis(250));
            assert!(weak_root.upgrade().is_none());
            assert!(weak_button.upgrade().is_none());
            assert!(weak_hover.upgrade().is_none());
        }
    }

    #[test]
    fn failure_details_keep_cause_chain_and_dependency_id_without_credentials() {
        let reason = "Finalizing installation\nunsupported required GOG dependency: ExampleDependency2019_x64\npath /games/the_witcher_3/redist\nrequest=(https://example.invalid/redist?signed=URL_CANARY) access_token= TOKEN_CANARY\nAuthorization: Bearer AUTH_CANARY\nclient_secret=SECRET_CANARY\nCaused by: installer exit status 42";
        for context in [
            "The Witcher 3: Installation failed",
            "The Witcher 3: Depot operation failed",
            "The Witcher 3: Could not run game",
        ] {
            let result = failure_message(context, reason);
            assert!(result.starts_with(context));
            assert!(result.contains("ExampleDependency2019_x64"));
            assert!(result.contains("/games/the_witcher_3/redist"));
            assert!(result.contains("\nCaused by: installer exit status 42"));
            assert!(!result.contains("CANARY"));
        }
        assert!(
            !failure_message("Failed", "authorization=\"Bearer BEARER_CANARY\"").contains("CANARY")
        );
    }

    #[test]
    #[ignore = "requires isolated HOME/XDG, Xvfb and private D-Bus"]
    fn full_failure_opens_only_on_user_action_and_keeps_current_page() {
        adw::init().expect("private display required");
        let app = adw::Application::builder()
            .application_id("io.github.legendarylinux.Ludomere.ErrorTest")
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let window = adw::ApplicationWindow::new(&app);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let page = gtk::Stack::new();
        page.add_named(&gtk::Label::new(Some("Game details")), Some("details"));
        page.add_named(&gtk::Label::new(Some("Downloads")), Some("downloads"));
        page.set_visible_child_name("details");
        content.append(&page);
        let compact = gtk::Label::new(None);
        let notifications = Notifications::new(&window, &compact);
        content.append(&notifications.root);
        window.set_content(Some(&content));
        window.present();
        let message = failure_message(
            "The Witcher 3: Depot operation failed",
            &format!(
                "{}\nunsupported required GOG dependency: TerminalDependencyIdentifier",
                "Diagnostic context. ".repeat(120)
            ),
        );
        compact.set_label(&message);
        assert_eq!(compact.label(), message);
        assert_eq!(compact.layout().line_count(), 1);
        assert_eq!(notifications.latest.label(), message);
        assert!(window.visible_dialog().is_none());
        assert_eq!(page.visible_child_name().as_deref(), Some("details"));
        notifications.show_message(&window, &message);
        assert!(window.visible_dialog().is_some());
        assert_eq!(page.visible_child_name().as_deref(), Some("details"));
        let list = notifications
            .modal_list
            .borrow()
            .as_ref()
            .unwrap()
            .upgrade()
            .unwrap();
        let row = list.first_child().unwrap().downcast::<gtk::Box>().unwrap();
        let text = row.first_child().unwrap().downcast::<gtk::Label>().unwrap();
        assert_eq!(text.label(), message);
        assert!(text.is_selectable());
        notifications.clear();
        assert!(notifications.history.borrow().entries.is_empty());
        assert_eq!(
            list.first_child().unwrap().widget_name(),
            "empty-notifications"
        );
        window.close();
    }

    #[test]
    fn history_bounds_deduplicates_and_does_not_extend_repeated_status() {
        let now = Instant::now();
        let mut history = History::default();
        for i in 0..250 {
            assert!(history.push(format!("Result {i}"), now));
        }
        assert_eq!(history.entries.len(), 200);
        assert_eq!(history.entries.front().unwrap(), "Result 50");
        let revision = history.revision;
        assert!(!history.push("Result 249".into(), now + Duration::from_secs(9)));
        assert!(history.expired(revision, now + Duration::from_secs(10)));
    }

    #[test]
    fn old_timer_cannot_hide_new_message_or_previous_account_state() {
        let now = Instant::now();
        let mut history = History::default();
        history.push("First result".into(), now);
        let first = history.revision;
        history.push("New result".into(), now + Duration::from_secs(9));
        assert!(!history.expired(first, now + Duration::from_secs(10)));
        assert!(!history.expired(history.revision, now + Duration::from_secs(10)));
        let second = history.revision;
        history.clear();
        assert!(history.entries.is_empty());
        assert!(!history.expired(second, now + Duration::from_secs(20)));
    }
}

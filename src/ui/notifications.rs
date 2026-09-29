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
        root.connect_unrealize({
            let hover = hover.clone();
            move |_| hover.unparent()
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
                let dialog = adw::Dialog::builder()
                    .title("Notifications")
                    .content_width(640)
                    .content_height(480)
                    .build();
                let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
                let header = adw::HeaderBar::new();
                content.append(&header);
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
                let scroll = gtk::ScrolledWindow::builder()
                    .hscrollbar_policy(gtk::PolicyType::Never)
                    .vexpand(true)
                    .child(&list)
                    .build();
                content.append(&scroll);
                dialog.set_child(Some(&content));
                dialog.present(Some(&window));
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

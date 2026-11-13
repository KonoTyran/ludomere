use super::*;
use crate::installation::runtime_logs::{RuntimeLog, RuntimeLogTail};
use std::cell::Cell;
use std::time::Instant;

struct Viewer {
    root: glib::WeakRef<gtk::Box>,
    window: glib::WeakRef<adw::ApplicationWindow>,
    model: Rc<RefCell<AppModel>>,
    epoch: u64,
    product_id: i64,
    selector: gtk::DropDown,
    follow: gtk::CheckButton,
    copy: gtk::Button,
    refresh: gtk::Button,
    reading: gtk::Spinner,
    folder: gtk::Button,
    status: gtk::Label,
    folder_status: gtk::Label,
    text: gtk::TextView,
    scroll: gtk::ScrolledWindow,
    logs: RefCell<Vec<RuntimeLog>>,
    selected: RefCell<Option<String>>,
    displayed: RefCell<Option<String>>,
    changing: Cell<bool>,
    running: Cell<bool>,
    force: Cell<bool>,
    next_read: Cell<Instant>,
}

impl Viewer {
    fn valid(&self) -> bool {
        let model = self.model.borrow();
        model.account_epoch == self.epoch && !model.logout_pending
    }

    fn request(self: &Rc<Self>, explicit: bool) {
        if !self.valid() {
            return;
        }
        if explicit {
            self.status.set_label("Reading game logs…");
            self.refresh.set_sensitive(false);
            self.reading.set_visible(true);
            self.reading.start();
        }
        if self.running.replace(true) {
            return;
        }
        let requested = self.selected.borrow().clone();
        let product_id = self.product_id;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<_> {
                let logs = crate::installation::runtime_logs::list_runtime_logs(product_id)?;
                let name = requested
                    .filter(|name| logs.iter().any(|log| &log.name == name))
                    .or_else(|| logs.first().map(|log| log.name.clone()));
                let tail = name
                    .as_deref()
                    .map(|name| {
                        crate::installation::runtime_logs::read_runtime_log(product_id, name)
                    })
                    .transpose()?
                    .map(|mut tail| {
                        tail.text = notifications::failure_message("", &tail.text)
                            .trim_start_matches('\n')
                            .to_owned();
                        tail
                    });
                Ok((logs, name, tail))
            })();
            let _ = sender.send(result);
        });
        let viewer = self.clone();
        let selection = self.selected.borrow().clone();
        glib::timeout_add_local(Duration::from_millis(40), move || {
            if !viewer.valid() || viewer.root.upgrade().is_none() {
                viewer.clear();
                return glib::ControlFlow::Break;
            }
            match receiver.try_recv() {
                Ok(result) => {
                    viewer.read_finished();
                    if *viewer.selected.borrow() != selection {
                        viewer.request(true);
                        return glib::ControlFlow::Break;
                    }
                    match result {
                        Ok((logs, name, tail)) => viewer.apply(logs, name, tail),
                        Err(error) => viewer.status.set_label(&notifications::failure_message(
                            "Could not read game logs. Refresh to retry.",
                            &format!("{error:#}"),
                        )),
                    }
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    viewer.read_finished();
                    viewer
                        .status
                        .set_label("Log reader stopped. Refresh to retry.");
                    glib::ControlFlow::Break
                }
            }
        });
    }

    fn read_finished(&self) {
        self.running.set(false);
        self.next_read.set(Instant::now() + Duration::from_secs(1));
        self.reading.stop();
        self.reading.set_visible(false);
        self.refresh.set_sensitive(true);
    }

    fn apply(&self, logs: Vec<RuntimeLog>, name: Option<String>, tail: Option<RuntimeLogTail>) {
        self.changing.set(true);
        if self
            .logs
            .borrow()
            .iter()
            .map(|log| &log.name)
            .collect::<Vec<_>>()
            != logs.iter().map(|log| &log.name).collect::<Vec<_>>()
        {
            let labels = logs
                .iter()
                .map(|log| {
                    let date = chrono::DateTime::from_timestamp(log.modified, 0)
                        .map(|date| {
                            date.with_timezone(&chrono::Local)
                                .format("%b %-d, %Y %H:%M:%S")
                                .to_string()
                        })
                        .unwrap_or_else(|| "Unknown date".to_owned());
                    if log.name == format!("{}.log", self.product_id) {
                        format!("Legacy latest log — {date}")
                    } else {
                        format!("Saved run — {date}")
                    }
                })
                .collect::<Vec<_>>();
            self.selector.set_model(Some(&gtk::StringList::new(
                &labels.iter().map(String::as_str).collect::<Vec<_>>(),
            )));
        }
        self.selector.set_selected(
            logs.iter()
                .position(|log| Some(&log.name) == name.as_ref())
                .map_or(gtk::INVALID_LIST_POSITION, |index| index as u32),
        );
        self.selector.set_sensitive(!logs.is_empty());
        self.folder.set_sensitive(!logs.is_empty());
        *self.logs.borrow_mut() = logs;
        *self.selected.borrow_mut() = name.clone();
        self.changing.set(false);
        let Some(tail) = tail else {
            self.text.buffer().set_text("");
            self.copy.set_sensitive(false);
            self.status
                .set_label("No game-run logs yet. Launch this game to create one.");
            return;
        };
        let force = self.force.replace(false) || *self.displayed.borrow() != name;
        if !update_buffer(
            &self.text.buffer(),
            &tail.text,
            self.follow.is_active(),
            force,
        ) {
            self.status
                .set_label("Live view paused to preserve your reading position or selection.");
            return;
        }
        *self.displayed.borrow_mut() = name;
        self.copy.set_sensitive(!tail.text.is_empty());
        self.status.set_label(&if tail.truncated {
            format!(
                "Showing the latest {}. The complete saved log is available in the log folder.",
                human_size(256 * 1024)
            )
        } else if tail.text.is_empty() {
            "This launch has not written any output yet.".to_owned()
        } else {
            "Saved launch output.".to_owned()
        });
        if self.follow.is_active() {
            let text = self.text.downgrade();
            glib::idle_add_local_once(move || {
                if let Some(text) = text.upgrade() {
                    text.scroll_to_iter(&mut text.buffer().end_iter(), 0.0, false, 0.0, 1.0);
                }
            });
        }
    }

    fn clear(&self) {
        self.reading.stop();
        self.reading.set_visible(false);
        self.text.buffer().set_text("");
        self.logs.borrow_mut().clear();
        self.selector.set_model(None::<&gtk::StringList>);
        self.selector.set_sensitive(false);
        self.copy.set_sensitive(false);
        self.refresh.set_sensitive(false);
        self.folder.set_sensitive(false);
        self.follow.set_sensitive(false);
        self.folder_status.set_label("");
        self.folder_status.set_visible(false);
        self.status
            .set_label("Account changed. Go to Home, then reopen this game to view logs.");
    }
}

fn update_buffer(buffer: &gtk::TextBuffer, text: &str, follow: bool, force: bool) -> bool {
    if !force && (!follow || buffer.has_selection()) {
        return false;
    }
    let previous = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
    if let Some(append) = text.strip_prefix(previous.as_str()) {
        if !append.is_empty() {
            buffer.insert(&mut buffer.end_iter(), append);
        }
    } else if text != previous.as_str() {
        buffer.set_text(text);
    }
    true
}

pub(super) fn runtime_log_view(
    window: &adw::ApplicationWindow,
    model: &Rc<RefCell<AppModel>>,
    product_id: i64,
) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_widget_name("runtime-log-view");
    let title = gtk::Label::new(Some("Game-run logs"));
    title.set_xalign(0.0);
    title.add_css_class("section-title");
    root.append(&title);
    let selector = gtk::DropDown::from_strings(&[]);
    selector.set_widget_name("runtime-log-selector");
    selector.set_hexpand(true);
    selector.set_sensitive(false);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    controls.append(&selector);
    let reading = gtk::Spinner::new();
    reading.set_widget_name("runtime-log-reading");
    reading.set_visible(false);
    controls.append(&reading);
    let refresh = gtk::Button::with_label("Refresh");
    let copy = gtk::Button::with_label("Copy");
    copy.set_sensitive(false);
    let folder = gtk::Button::with_label("Open log folder");
    folder.set_sensitive(false);
    folder.set_tooltip_text(Some("Available after a game-run log has been saved"));
    for (button, name) in [
        (&refresh, "runtime-log-refresh"),
        (&copy, "runtime-log-copy"),
        (&folder, "runtime-log-folder"),
    ] {
        button.set_widget_name(name);
        controls.append(button);
    }
    root.append(&controls);
    let follow = gtk::CheckButton::with_label("Follow live output");
    follow.set_active(true);
    root.append(&follow);
    let status = gtk::Label::new(Some("Loading saved launch logs…"));
    status.set_widget_name("runtime-log-status");
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.set_selectable(true);
    root.append(&status);
    let folder_status = gtk::Label::new(None);
    folder_status.set_widget_name("runtime-log-folder-status");
    folder_status.set_xalign(0.0);
    folder_status.set_wrap(true);
    folder_status.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    folder_status.set_selectable(true);
    folder_status.set_visible(false);
    root.append(&folder_status);
    let text = gtk::TextView::new();
    text.set_widget_name("runtime-log-text");
    text.set_editable(false);
    text.set_monospace(true);
    text.set_wrap_mode(gtk::WrapMode::WordChar);
    text.set_left_margin(8);
    text.set_right_margin(8);
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(320)
        .vexpand(true)
        .child(&text)
        .build();
    root.append(&scroll);
    let viewer = Rc::new(Viewer {
        root: root.downgrade(),
        window: window.downgrade(),
        model: model.clone(),
        epoch: model.borrow().account_epoch,
        product_id,
        selector,
        follow,
        copy,
        refresh,
        reading,
        folder,
        status,
        folder_status,
        text,
        scroll,
        logs: RefCell::new(Vec::new()),
        selected: RefCell::new(None),
        displayed: RefCell::new(None),
        changing: Cell::new(false),
        running: Cell::new(false),
        force: Cell::new(false),
        next_read: Cell::new(Instant::now()),
    });
    viewer.selector.connect_selected_notify({
        let viewer = Rc::downgrade(&viewer);
        move |selector| {
            let Some(viewer) = viewer.upgrade() else {
                return;
            };
            if viewer.changing.get() || !viewer.valid() {
                return;
            }
            *viewer.selected.borrow_mut() = viewer
                .logs
                .borrow()
                .get(selector.selected() as usize)
                .map(|log| log.name.clone());
            viewer.force.set(true);
            viewer.next_read.set(Instant::now());
            viewer.request(true);
        }
    });
    viewer.refresh.connect_clicked({
        let viewer = Rc::downgrade(&viewer);
        move |_| {
            if let Some(viewer) = viewer.upgrade() {
                viewer.force.set(true);
                viewer.request(true);
            }
        }
    });
    viewer.follow.connect_toggled({
        let viewer = Rc::downgrade(&viewer);
        move |follow| {
            if follow.is_active()
                && let Some(viewer) = viewer.upgrade()
            {
                let buffer = viewer.text.buffer();
                buffer.place_cursor(&buffer.end_iter());
                viewer.force.set(true);
                viewer.request(true);
            }
        }
    });
    viewer.copy.connect_clicked({
        let viewer = Rc::downgrade(&viewer);
        move |_| {
            if let Some(viewer) = viewer.upgrade()
                && viewer.valid()
            {
                let buffer = viewer.text.buffer();
                viewer.text.clipboard().set_text(&buffer.text(
                    &buffer.start_iter(),
                    &buffer.end_iter(),
                    false,
                ));
            }
        }
    });
    viewer.folder.connect_clicked({
        let viewer = Rc::downgrade(&viewer);
        move |_| {
            if let Some(viewer) = viewer.upgrade() && viewer.valid() && let Some(window) = viewer.window.upgrade() {
                viewer.folder_status.set_label("");
                viewer.folder_status.set_visible(false);
                if !viewer.valid() {
                    return;
                }
                let weak = Rc::downgrade(&viewer);
                let finished = move |result: Result<(), glib::Error>| {
                    if let Some(viewer) = weak.upgrade() && viewer.valid() {
                        if let Err(error) = result {
                            let message = notifications::failure_message("Could not open the log folder. Check your desktop file manager and try again.", &error.to_string());
                            viewer.folder_status.set_label(&message);
                            viewer.folder_status.set_visible(true);
                            viewer.folder.set_label("Retry opening log folder");
                            viewer.folder.set_tooltip_text(Some(&message));
                        } else {
                            viewer.folder_status.set_label("");
                            viewer.folder_status.set_visible(false);
                            viewer.folder.set_label("Open log folder");
                            viewer.folder.set_tooltip_text(None);
                        }
                    }
                };
                #[cfg(test)]
                if let Some(result) = tests::FOLDER_RESULTS.with_borrow_mut(|pending| {
                    pending.as_mut().map(|pending| pending.pop_front().expect("missing inert folder launch result"))
                }) {
                    glib::idle_add_local_once(move || finished(result));
                    return;
                }
                let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(crate::identity::runtime_logs())));
                launcher.launch(Some(&window), gio::Cancellable::NONE, finished);
            }
        }
    });
    let scrolling = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    scrolling.set_propagation_phase(gtk::PropagationPhase::Capture);
    scrolling.connect_scroll({
        let viewer = Rc::downgrade(&viewer);
        move |_, _, dy| {
            if dy != 0.0
                && let Some(viewer) = viewer.upgrade()
            {
                viewer.follow.set_active(false);
            }
            glib::Propagation::Proceed
        }
    });
    viewer.scroll.add_controller(scrolling);
    let dragging = gtk::GestureClick::new();
    dragging.set_propagation_phase(gtk::PropagationPhase::Capture);
    dragging.connect_pressed({
        let viewer = Rc::downgrade(&viewer);
        move |_, _, _, _| {
            if let Some(viewer) = viewer.upgrade() {
                viewer.follow.set_active(false);
            }
        }
    });
    viewer.scroll.vscrollbar().add_controller(dragging);
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed({
        let viewer = Rc::downgrade(&viewer);
        move |_, key, _, _| {
            if matches!(
                key,
                gtk::gdk::Key::Up
                    | gtk::gdk::Key::Down
                    | gtk::gdk::Key::Home
                    | gtk::gdk::Key::End
                    | gtk::gdk::Key::Page_Up
                    | gtk::gdk::Key::Page_Down
            ) && let Some(viewer) = viewer.upgrade()
            {
                viewer.follow.set_active(false);
            }
            glib::Propagation::Proceed
        }
    });
    viewer.text.add_controller(keys);
    let mut rooted = false;
    glib::timeout_add_local(Duration::from_millis(200), move || {
        let Some(root) = viewer.root.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if !viewer.valid() {
            viewer.clear();
            return glib::ControlFlow::Break;
        }
        if root.root().is_some() {
            rooted = true;
        } else if rooted {
            return glib::ControlFlow::Break;
        }
        if root.is_mapped() && Instant::now() >= viewer.next_read.get() {
            viewer.request(false);
        }
        glib::ControlFlow::Continue
    });
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    type FolderResult = Result<(), glib::Error>;

    thread_local! {
        pub(super) static FOLDER_RESULTS: RefCell<Option<VecDeque<FolderResult>>> = const { RefCell::new(None) };
    }

    #[test]
    #[ignore = "requires private p399 HOME/all XDG/TMP and GTK; only synthetic logs, folder launches fail-closed inert"]
    fn folder_failures_survive_real_reader_updates_and_clear_on_retry_or_retirement() {
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
                    .starts_with("/tmp/ludomere-p399-")
            );
        }
        assert!(!crate::identity::database().exists());
        assert!(!Config::path().exists());
        FOLDER_RESULTS.with_borrow_mut(|pending| *pending = Some(VecDeque::new()));
        adw::init().unwrap();
        #[track_caller]
        fn wait_until(check: impl Fn() -> bool) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !check() && Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        let app = adw::Application::builder()
            .application_id("io.github.ludomere.LogFolderFeedbackTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let window = adw::ApplicationWindow::builder()
            .application(&app)
            .default_width(800)
            .default_height(650)
            .build();
        let model = Rc::new(RefCell::new(AppModel::default()));
        let path = crate::identity::runtime_logs().join("9399001.log");
        assert!(path.starts_with(std::env::var("XDG_DATA_HOME").unwrap()));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "initial output\n").unwrap();
        let view = runtime_log_view(&window, &model, 9399001);
        let folder = find_named_descendant(view.upcast_ref(), "runtime-log-folder")
            .and_downcast::<gtk::Button>()
            .unwrap();
        let status = find_named_descendant(view.upcast_ref(), "runtime-log-status")
            .and_downcast::<gtk::Label>()
            .unwrap();
        let folder_status = find_named_descendant(view.upcast_ref(), "runtime-log-folder-status")
            .and_downcast::<gtk::Label>()
            .unwrap();
        let refresh = find_named_descendant(view.upcast_ref(), "runtime-log-refresh")
            .and_downcast::<gtk::Button>()
            .unwrap();
        let copy = find_named_descendant(view.upcast_ref(), "runtime-log-copy")
            .and_downcast::<gtk::Button>()
            .unwrap();
        let text = find_named_descendant(view.upcast_ref(), "runtime-log-text")
            .and_downcast::<gtk::TextView>()
            .unwrap();
        let follow = status
            .prev_sibling()
            .unwrap()
            .downcast::<gtk::CheckButton>()
            .unwrap();
        let buffer = text.buffer();
        window.set_content(Some(&view));
        window.present();
        wait_until(|| folder.is_mapped() && folder.is_sensitive() && copy.is_sensitive());
        assert!(!folder_status.get_visible());
        let fail = || {
            FOLDER_RESULTS.with_borrow_mut(|pending| {
                pending.as_mut().unwrap().push_back(Err(glib::Error::new(
                    gio::IOErrorEnum::Failed,
                    "Synthetic file manager refused access_token=P399_SECRET",
                )));
            })
        };
        fail();
        folder.emit_clicked();
        wait_until(|| folder_status.is_mapped());
        let message = folder_status.label();
        assert!(message.contains("Synthetic file manager refused"));
        assert!(!message.contains("P399_SECRET"));
        assert!(folder_status.is_selectable());
        assert!(folder_status.wraps());
        assert_eq!(folder_status.wrap_mode(), gtk::pango::WrapMode::WordChar);
        assert_eq!(folder.label().as_deref(), Some("Retry opening log folder"));
        assert_eq!(folder.tooltip_text().as_deref(), Some(message.as_str()));
        for output in [
            "initial output\nsecond\n",
            "initial output\nsecond\nthird\n",
        ] {
            std::fs::write(&path, output).unwrap();
            wait_until(|| buffer.text(&buffer.start_iter(), &buffer.end_iter(), false) == output);
            assert_eq!(folder_status.label(), message);
            assert!(folder_status.is_mapped());
            assert!(copy.is_sensitive());
        }
        // Ordinary polling preserves selection and the independent folder message.
        buffer.select_range(&buffer.start_iter(), &buffer.iter_at_offset(7));
        std::fs::write(&path, "initial output\nsecond\nthird\nfourth\n").unwrap();
        wait_until(|| status.label().contains("Live view paused"));
        let (start, end) = buffer.selection_bounds().unwrap();
        assert_eq!(buffer.text(&start, &end, false), "initial");
        assert_eq!(folder_status.label(), message);
        follow.set_active(false);
        refresh.emit_clicked();
        wait_until(|| {
            buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .contains("fourth")
        });
        assert!(!follow.is_active());
        assert_eq!(folder_status.label(), message);
        assert!(folder_status.is_mapped());

        FOLDER_RESULTS.with_borrow_mut(|pending| pending.as_mut().unwrap().push_back(Ok(())));
        folder.emit_clicked();
        assert!(folder_status.label().is_empty());
        assert!(!folder_status.get_visible());
        wait_until(|| folder.label().as_deref() == Some("Open log folder"));
        assert!(folder.tooltip_text().is_none());
        fail();
        folder.emit_clicked();
        wait_until(|| folder_status.is_mapped());
        assert_eq!(folder_status.label(), message);
        model.borrow_mut().account_epoch += 1;
        wait_until(|| status.label().contains("Account changed"));
        assert!(!folder.is_sensitive());
        assert!(folder_status.label().is_empty());
        assert!(!folder_status.get_visible());

        // The queued completion shares production validity checks, so retirement wins.
        let view = runtime_log_view(&window, &model, 9399001);
        let folder = find_named_descendant(view.upcast_ref(), "runtime-log-folder")
            .and_downcast::<gtk::Button>()
            .unwrap();
        let folder_status = find_named_descendant(view.upcast_ref(), "runtime-log-folder-status")
            .and_downcast::<gtk::Label>()
            .unwrap();
        let status = find_named_descendant(view.upcast_ref(), "runtime-log-status")
            .and_downcast::<gtk::Label>()
            .unwrap();
        window.set_content(Some(&view));
        wait_until(|| folder.is_mapped() && folder.is_sensitive());
        fail();
        folder.emit_clicked();
        assert!(!folder_status.get_visible());
        model.borrow_mut().account_epoch += 1;
        wait_until(|| status.label().contains("Account changed"));
        assert!(!folder.is_sensitive());
        assert!(folder_status.label().is_empty());
        assert!(!folder_status.get_visible());
        // Clearing previous feedback can synchronously retire the account before dispatch.
        let view = runtime_log_view(&window, &model, 9399001);
        let folder = find_named_descendant(view.upcast_ref(), "runtime-log-folder")
            .and_downcast::<gtk::Button>()
            .unwrap();
        let folder_status = find_named_descendant(view.upcast_ref(), "runtime-log-folder-status")
            .and_downcast::<gtk::Label>()
            .unwrap();
        let status = find_named_descendant(view.upcast_ref(), "runtime-log-status")
            .and_downcast::<gtk::Label>()
            .unwrap();
        window.set_content(Some(&view));
        wait_until(|| folder.is_mapped() && folder.is_sensitive());
        fail();
        folder.emit_clicked();
        wait_until(|| folder_status.is_mapped());
        let retire = folder_status.connect_notify_local(Some("label"), {
            let model = model.clone();
            move |label, _| {
                if label.label().is_empty() {
                    model.borrow_mut().account_epoch += 1;
                }
            }
        });
        FOLDER_RESULTS.with_borrow_mut(|pending| pending.as_mut().unwrap().push_back(Ok(())));
        folder.emit_clicked();
        folder_status.disconnect(retire);
        FOLDER_RESULTS.with_borrow_mut(|pending| {
            let pending = pending.as_mut().unwrap();
            assert_eq!(pending.len(), 1, "retired click must not dispatch a launch");
            assert!(pending.pop_front().unwrap().is_ok());
        });
        wait_until(|| status.label().contains("Account changed"));
        assert!(folder_status.label().is_empty());
        assert!(!folder_status.get_visible());
        assert!(
            FOLDER_RESULTS
                .with_borrow_mut(Option::take)
                .unwrap()
                .is_empty()
        );
        assert!(!crate::identity::database().exists());
        assert!(!Config::path().exists());
        window.destroy();
    }

    #[test]
    #[ignore = "requires isolated HOME/XDG, Xvfb and private D-Bus"]
    fn live_log_updates_preserve_reading_selection_until_explicit_refresh() {
        gtk::init().expect("private display required");
        let buffer = gtk::TextBuffer::new(None);
        assert!(update_buffer(&buffer, "first\nsecond\n", true, false));
        buffer.select_range(&buffer.iter_at_offset(0), &buffer.iter_at_offset(5));
        assert!(!update_buffer(
            &buffer,
            "first\nsecond\nthird\n",
            true,
            false
        ));
        let (start, end) = buffer.selection_bounds().unwrap();
        assert_eq!(buffer.text(&start, &end, false), "first");
        assert!(!update_buffer(&buffer, "rotated tail", false, false));
        assert_eq!(
            buffer.text(&buffer.start_iter(), &buffer.end_iter(), false),
            "first\nsecond\n"
        );
        assert!(update_buffer(&buffer, "rotated tail", false, true));
        assert_eq!(
            buffer.text(&buffer.start_iter(), &buffer.end_iter(), false),
            "rotated tail"
        );
        let end = buffer.end_iter();
        buffer.place_cursor(&end);
        assert!(update_buffer(
            &buffer,
            "rotated tail\nnew <literal> output",
            true,
            false
        ));
        assert_eq!(
            buffer.text(&buffer.start_iter(), &buffer.end_iter(), false),
            "rotated tail\nnew <literal> output"
        );
    }
}

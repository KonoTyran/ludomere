use crate::{domain::Screenshot, screenshots};
use adw::prelude::*;
#[cfg(test)]
use gtk::gio;
use gtk::glib;
use std::{rc::Rc, sync::mpsc, time::Duration};

pub(in crate::ui) fn screenshot_strip(
    product_id: i64,
    screenshot_items: &[Screenshot],
    window: &adw::ApplicationWindow,
) -> gtk::Box {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 10);
    section.set_hexpand(true);
    section.set_halign(gtk::Align::Fill);
    let heading = gtk::Label::new(Some("Screenshots"));
    heading.set_xalign(0.0);
    heading.add_css_class("section-title");
    section.append(&heading);

    if screenshot_items.is_empty() {
        let empty = gtk::Label::new(Some("No screenshots available"));
        empty.set_xalign(0.0);
        empty.add_css_class("dim-label");
        section.append(&empty);
        return section;
    }

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let mut pictures = Vec::new();
    for (index, screenshot) in screenshot_items.iter().cloned().enumerate() {
        let (tile, picture) = screenshot_image(product_id, screenshot, false);
        pictures.push(picture.clone());
        let button = gtk::Button::new();
        button.add_css_class("screenshot-thumbnail");
        button.set_tooltip_text(Some("Open screenshot gallery"));
        picture.set_content_fit(gtk::ContentFit::Cover);
        picture.set_size_request(210, 118);
        tile.set_child(gtk::Widget::NONE);
        button.set_child(Some(&picture));
        button.set_sensitive(false);
        tile.set_child(Some(&button));
        let ready_button = button.downgrade();
        picture.connect_css_classes_notify(move |picture| {
            if let Some(button) = ready_button.upgrade() {
                button.set_sensitive(picture.has_css_class("image-ready"));
            }
        });
        let gallery_items = screenshot_items.to_vec();
        let window = window.clone();
        button.connect_clicked(move |_| {
            show_screenshot_gallery(&window, product_id, &gallery_items, index)
        });
        row.append(&tile);
    }
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .min_content_width(0)
        .min_content_height(140)
        .propagate_natural_width(false)
        .child(&row)
        .build();
    scroll.set_hexpand(true);
    scroll.set_halign(gtk::Align::Fill);
    scroll.set_propagate_natural_height(true);
    let thumbnail_overlay = gtk::Overlay::new();
    thumbnail_overlay.set_hexpand(true);
    thumbnail_overlay.set_halign(gtk::Align::Fill);
    thumbnail_overlay.set_size_request(-1, 140);
    thumbnail_overlay.set_overflow(gtk::Overflow::Hidden);
    thumbnail_overlay.set_child(Some(&scroll));
    let previous = gtk::Button::from_icon_name("go-previous-symbolic");
    previous.set_tooltip_text(Some("Previous screenshots"));
    previous.set_halign(gtk::Align::Start);
    previous.set_valign(gtk::Align::Center);
    previous.add_css_class("thumbnail-scroll-button");
    let next = gtk::Button::from_icon_name("go-next-symbolic");
    next.set_tooltip_text(Some("More screenshots"));
    next.set_halign(gtk::Align::End);
    next.set_valign(gtk::Align::Center);
    next.add_css_class("thumbnail-scroll-button");
    {
        let adjustment = scroll.hadjustment();
        previous.connect_clicked(move |_| {
            adjustment.set_value((adjustment.value() - 220.0).max(adjustment.lower()));
        });
    }
    {
        let adjustment = scroll.hadjustment();
        next.connect_clicked(move |_| {
            let maximum = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
            adjustment.set_value((adjustment.value() + 220.0).min(maximum));
        });
    }
    thumbnail_overlay.add_overlay(&previous);
    thumbnail_overlay.add_overlay(&next);
    let update_navigation: Rc<dyn Fn()> = Rc::new({
        let adjustment = scroll.hadjustment().downgrade();
        let previous = previous.downgrade();
        let next = next.downgrade();
        let pictures = pictures
            .iter()
            .map(gtk::Picture::downgrade)
            .collect::<Vec<_>>();
        move || {
            let (Some(adjustment), Some(previous), Some(next)) =
                (adjustment.upgrade(), previous.upgrade(), next.upgrade())
            else {
                return;
            };
            let ready = pictures
                .iter()
                .filter_map(glib::WeakRef::upgrade)
                .any(|picture| picture.has_css_class("image-ready"));
            let maximum = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
            previous.set_visible(ready && adjustment.value() > adjustment.lower() + 1.0);
            next.set_visible(ready && adjustment.value() < maximum - 1.0);
        }
    });
    update_navigation();
    {
        let update = update_navigation.clone();
        scroll.hadjustment().connect_changed(move |_| update());
    }
    {
        let update = update_navigation.clone();
        scroll
            .hadjustment()
            .connect_value_changed(move |_| update());
    }
    for picture in pictures {
        let update = update_navigation.clone();
        picture.connect_css_classes_notify(move |_| update());
    }
    section.append(&thumbnail_overlay);
    section
}

fn screenshot_image(
    product_id: i64,
    screenshot: Screenshot,
    full: bool,
) -> (gtk::Overlay, gtk::Picture) {
    let overlay = gtk::Overlay::new();
    let picture = gtk::Picture::new();
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_hexpand(full);
    picture.set_vexpand(full);
    overlay.set_child(Some(&picture));
    let state = gtk::Box::new(gtk::Orientation::Vertical, 6);
    state.set_halign(gtk::Align::Center);
    state.set_valign(gtk::Align::Center);
    let spinner = gtk::Spinner::new();
    let label = gtk::Label::new(None);
    label.set_wrap(true);
    label.set_max_width_chars(22);
    let retry = gtk::Button::with_label("Retry screenshot");
    state.append(&spinner);
    state.append(&label);
    state.append(&retry);
    overlay.add_overlay(&state);
    picture.connect_css_classes_notify({
        let state = state.downgrade();
        let spinner = spinner.downgrade();
        let label = label.downgrade();
        let retry = retry.downgrade();
        move |picture| {
            let (Some(state), Some(spinner), Some(label), Some(retry)) = (
                state.upgrade(),
                spinner.upgrade(),
                label.upgrade(),
                retry.upgrade(),
            ) else {
                return;
            };
            let loading = picture.has_css_class("image-loading");
            state.set_visible(!picture.has_css_class("image-ready"));
            spinner.set_visible(loading);
            spinner.set_spinning(loading);
            label.set_label(if loading {
                "Loading screenshot…"
            } else {
                "Screenshot could not load"
            });
            retry.set_visible(!loading);
        }
    });
    let load: Rc<dyn Fn()> = Rc::new({
        let picture = picture.downgrade();
        move || {
            let Some(current) = picture.upgrade() else {
                return;
            };
            super::media::set_picture_status(&current, "image-loading", "Loading screenshot…");
            let (sender, receiver) = mpsc::channel();
            let screenshot = screenshot.clone();
            std::thread::spawn(move || {
                let _ = sender.send(screenshots::cached_image(product_id, &screenshot, full));
            });
            let picture = picture.clone();
            glib::timeout_add_local(Duration::from_millis(50), move || {
                let Some(picture) = picture.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                match receiver.try_recv() {
                    Ok(Ok(path)) => {
                        super::media::set_card_picture(
                            &picture,
                            &path,
                            if full { 1600 } else { 210 },
                            if full { 900 } else { 118 },
                        );
                        glib::ControlFlow::Break
                    }
                    Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => {
                        super::media::set_picture_status(
                            &picture,
                            "image-error",
                            "Screenshot could not load. Retry this screenshot.",
                        );
                        glib::ControlFlow::Break
                    }
                    Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                }
            });
        }
    });
    {
        let load = load.clone();
        retry.connect_clicked(move |button| {
            // Retry hides this button. Keep focus inside the gallery before starting
            // the request, rather than moving it when background work completes.
            if full
                && let Some(gallery) = button
                    .ancestor(gtk::Stack::static_type())
                    .and_then(|stack| stack.parent())
            {
                gallery.grab_focus();
            }
            load();
        });
    }
    load();
    (overlay, picture)
}

fn show_screenshot_gallery(
    window: &adw::ApplicationWindow,
    product_id: i64,
    screenshots_list: &[Screenshot],
    initial_index: usize,
) {
    if screenshots_list.is_empty() {
        return;
    }
    let overlay = gtk::Overlay::new();
    overlay.add_css_class("screenshot-gallery");
    overlay.set_focusable(true);
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
    stack.set_transition_duration(180);
    let mut pictures = Vec::new();
    for (index, screenshot) in screenshots_list.iter().cloned().enumerate() {
        let (image, picture) = screenshot_image(product_id, screenshot, true);
        picture.set_widget_name(&format!("gallery-image-{index}"));
        pictures.push(picture);
        stack.add_named(&image, Some(&index.to_string()));
    }
    overlay.set_child(Some(&stack));

    let image_navigation = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    image_navigation.set_hexpand(true);
    image_navigation.set_vexpand(true);
    let previous_image = gtk::Button::from_icon_name("go-previous-symbolic");
    previous_image.set_hexpand(true);
    previous_image.set_tooltip_text(Some("Previous screenshot"));
    previous_image.add_css_class("gallery-hit-area");
    if let Some(icon) = previous_image.child() {
        icon.set_halign(gtk::Align::Start);
        icon.set_margin_start(18);
    }
    let next_image = gtk::Button::from_icon_name("go-next-symbolic");
    next_image.set_hexpand(true);
    next_image.set_tooltip_text(Some("Next screenshot"));
    next_image.add_css_class("gallery-hit-area");
    if let Some(icon) = next_image.child() {
        icon.set_halign(gtk::Align::End);
        icon.set_margin_end(18);
    }
    image_navigation.append(&previous_image);
    image_navigation.append(&next_image);
    overlay.add_overlay(&image_navigation);
    let update_hit_areas: Rc<dyn Fn()> = Rc::new({
        let navigation = image_navigation.downgrade();
        let stack = stack.downgrade();
        let pictures = pictures
            .iter()
            .map(gtk::Picture::downgrade)
            .collect::<Vec<_>>();
        move || {
            let (Some(navigation), Some(stack)) = (navigation.upgrade(), stack.upgrade()) else {
                return;
            };
            let ready = stack
                .visible_child_name()
                .and_then(|name| name.parse::<usize>().ok())
                .and_then(|index| pictures.get(index))
                .and_then(glib::WeakRef::upgrade)
                .is_some_and(|picture| picture.has_css_class("image-ready"));
            navigation.set_visible(ready && pictures.len() > 1);
        }
    });
    update_hit_areas();
    {
        let update = update_hit_areas.clone();
        stack.connect_visible_child_name_notify(move |_| update());
    }
    for picture in pictures {
        let update = update_hit_areas.clone();
        picture.connect_css_classes_notify(move |_| update());
    }

    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    controls.set_halign(gtk::Align::Center);
    controls.set_valign(gtk::Align::End);
    controls.set_margin_bottom(18);
    controls.add_css_class("gallery-controls");
    let previous = gtk::Button::from_icon_name("go-previous-symbolic");
    let counter = gtk::Label::new(None);
    let next = gtk::Button::from_icon_name("go-next-symbolic");
    controls.append(&previous);
    controls.append(&counter);
    controls.append(&next);
    overlay.add_overlay(&controls);

    let close = gtk::Button::from_icon_name("window-close-symbolic");
    close.set_halign(gtk::Align::End);
    close.set_valign(gtk::Align::Start);
    close.set_margin_top(16);
    close.set_margin_end(16);
    close.add_css_class("gallery-close");
    overlay.add_overlay(&close);

    let dialog = adw::Dialog::builder()
        .content_width(1100)
        .content_height(720)
        .child(&overlay)
        .build();
    let index = Rc::new(std::cell::Cell::new(
        initial_index.min(screenshots_list.len().saturating_sub(1)),
    ));
    update_gallery_position(&stack, &counter, index.get(), screenshots_list.len());
    connect_gallery_navigation(
        &previous,
        &stack,
        &counter,
        &index,
        screenshots_list.len(),
        false,
    );
    connect_gallery_navigation(
        &previous_image,
        &stack,
        &counter,
        &index,
        screenshots_list.len(),
        false,
    );
    connect_gallery_navigation(
        &next,
        &stack,
        &counter,
        &index,
        screenshots_list.len(),
        true,
    );
    connect_gallery_navigation(
        &next_image,
        &stack,
        &counter,
        &index,
        screenshots_list.len(),
        true,
    );
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(move |_, key, _, _| {
        match key {
            gtk::gdk::Key::Left => previous.emit_clicked(),
            gtk::gdk::Key::Right => next.emit_clicked(),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    overlay.add_controller(keys);
    {
        let dialog = dialog.clone();
        close.connect_clicked(move |_| {
            dialog.close();
        });
    }

    dialog.present(Some(window));
}

fn update_gallery_position(stack: &gtk::Stack, counter: &gtk::Label, index: usize, count: usize) {
    stack.set_visible_child_name(&index.to_string());
    counter.set_label(&format!("{} / {count}", index + 1));
}

fn connect_gallery_navigation(
    button: &gtk::Button,
    stack: &gtk::Stack,
    counter: &gtk::Label,
    index: &Rc<std::cell::Cell<usize>>,
    count: usize,
    forward: bool,
) {
    button.set_visible(count > 1);
    button.set_sensitive(count > 1);
    if count < 2 {
        return;
    }
    let stack = stack.clone();
    let counter = counter.clone();
    let index = index.clone();
    button.connect_clicked(move |_| {
        let next_index = if forward {
            (index.get() + 1) % count
        } else if index.get() == 0 {
            count - 1
        } else {
            index.get() - 1
        };
        index.set(next_index);
        update_gallery_position(&stack, &counter, next_index, count);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires isolated HOME/XDG, Xvfb and private D-Bus; exercises real empty/failed screenshot layout"]
    fn empty_and_failed_screenshots_keep_library_information_below_the_strip() {
        adw::init().expect("requires an isolated GTK display");
        let app = adw::Application::builder()
            .application_id("io.github.legendarylinux.ludomere.ScreenshotLayoutTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gio::Cancellable>).unwrap();
        let window = adw::ApplicationWindow::new(&app);
        window.set_default_size(700, 500);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let empty = screenshot_strip(1, &[], &window);
        assert!(empty.last_child().unwrap().is::<gtk::Label>());
        body.append(&empty);
        let failed = screenshot_strip(1, &vec![Screenshot::default(); 4], &window);
        body.append(&failed);
        let information = gtk::Label::new(Some("Library information"));
        body.append(&information);
        window.set_content(Some(&body));
        window.present();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut settled = false;
        while std::time::Instant::now() < deadline {
            glib::MainContext::default().iteration(false);
            let mut pending = vec![failed.clone().upcast::<gtk::Widget>()];
            let mut failures = 0;
            while let Some(widget) = pending.pop() {
                failures += usize::from(widget.has_css_class("image-error"));
                if let Some(button) = widget.downcast_ref::<gtk::Button>()
                    && matches!(
                        button.tooltip_text().as_deref(),
                        Some("Previous screenshots" | "More screenshots")
                    )
                {
                    assert!(!button.is_visible());
                }
                let mut child = widget.first_child();
                while let Some(current) = child {
                    child = current.next_sibling();
                    pending.push(current);
                }
            }
            if failures == 4 && failed.height() >= 140 {
                settled = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            settled,
            "all invalid screenshot URLs must become visible failure states"
        );
        let strip = failed.compute_bounds(&body).unwrap();
        let following = information.compute_bounds(&body).unwrap();
        assert!(following.y() >= strip.y() + strip.height());
        window.close();
    }

    #[test]
    #[ignore = "requires isolated HOME/XDG, Xvfb and private D-Bus; exercises the real screenshot dialog"]
    fn screenshot_keys_reuse_wraparound_button_navigation() {
        adw::init().expect("requires an isolated GTK display");
        let app = adw::Application::builder()
            .application_id("io.github.legendarylinux.ludomere.GalleryTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gio::Cancellable>).unwrap();
        let window = adw::ApplicationWindow::new(&app);
        window.present();
        // Empty URLs fail locally: this test needs the real navigation controls, not a server.
        let screenshots = vec![Screenshot::default(); 3];
        show_screenshot_gallery(&window, 1, &screenshots, 0);
        let dialog = window.visible_dialog().unwrap();
        let overlay = dialog.child().unwrap().downcast::<gtk::Overlay>().unwrap();
        let stack = overlay.child().unwrap().downcast::<gtk::Stack>().unwrap();
        let keys = overlay
            .observe_controllers()
            .iter::<glib::Object>()
            .filter_map(Result::ok)
            .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
            .unwrap();
        for (key, expected) in [
            (gtk::gdk::Key::Right, "1"),
            (gtk::gdk::Key::Right, "2"),
            (gtk::gdk::Key::Right, "0"),
            (gtk::gdk::Key::Left, "2"),
        ] {
            assert!(keys.emit_by_name::<bool>(
                "key-pressed",
                &[&key, &0u32, &gtk::gdk::ModifierType::empty()]
            ));
            assert_eq!(stack.visible_child_name().as_deref(), Some(expected));
        }
        assert!(!keys.emit_by_name::<bool>(
            "key-pressed",
            &[&gtk::gdk::Key::a, &0u32, &gtk::gdk::ModifierType::empty()]
        ));
        assert_eq!(stack.visible_child_name().as_deref(), Some("2"));
        dialog.close();
        window.close();
    }

    #[test]
    #[ignore = "requires isolated GTK display and D-Bus"]
    fn single_screenshot_navigation_is_not_actionable() {
        adw::init().unwrap();
        let button = gtk::Button::new();
        let stack = gtk::Stack::new();
        stack.add_named(&gtk::Label::new(None), Some("0"));
        let counter = gtk::Label::new(None);
        let index = Rc::new(std::cell::Cell::new(0));
        connect_gallery_navigation(&button, &stack, &counter, &index, 1, true);
        assert!(!button.is_visible());
        assert!(!button.is_sensitive());
        button.emit_clicked();
        assert_eq!(index.get(), 0);
        assert!(
            counter.label().is_empty(),
            "single-image arrows must have no callback"
        );
    }
}

use crate::{domain::Screenshot, screenshots};
use adw::prelude::*;
use gtk::{gio, glib};
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

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    for (index, screenshot) in screenshot_items.iter().cloned().enumerate() {
        let button = gtk::Button::new();
        button.add_css_class("screenshot-thumbnail");
        button.set_tooltip_text(Some("Open screenshot gallery"));
        let picture = gtk::Picture::new();
        picture.set_content_fit(gtk::ContentFit::Cover);
        picture.set_size_request(210, 118);
        button.set_child(Some(&picture));
        let gallery_items = screenshot_items.to_vec();
        let window = window.clone();
        button.connect_clicked(move |_| {
            show_screenshot_gallery(&window, product_id, &gallery_items, index)
        });
        row.append(&button);

        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(screenshots::cached_image(product_id, &screenshot, false));
        });
        glib::timeout_add_local(Duration::from_millis(50), move || {
            match receiver.try_recv() {
                Ok(Ok(path)) => {
                    picture.set_file(Some(&gio::File::for_path(path)));
                    glib::ControlFlow::Break
                }
                Ok(Err(error)) => {
                    tracing::warn!(%error, "could not load screenshot thumbnail");
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => glib::ControlFlow::Break,
            }
        });
    }
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .min_content_width(0)
        .propagate_natural_width(false)
        .child(&row)
        .build();
    scroll.set_hexpand(true);
    scroll.set_halign(gtk::Align::Fill);
    scroll.set_propagate_natural_height(true);
    let thumbnail_overlay = gtk::Overlay::new();
    thumbnail_overlay.set_hexpand(true);
    thumbnail_overlay.set_halign(gtk::Align::Fill);
    thumbnail_overlay.set_child(Some(&scroll));
    let previous = gtk::Button::from_icon_name("go-previous-symbolic");
    previous.set_tooltip_text(Some("Previous screenshots"));
    previous.set_halign(gtk::Align::Start);
    previous.set_valign(gtk::Align::Fill);
    previous.add_css_class("thumbnail-scroll-button");
    let next = gtk::Button::from_icon_name("go-next-symbolic");
    next.set_tooltip_text(Some("More screenshots"));
    next.set_halign(gtk::Align::End);
    next.set_valign(gtk::Align::Fill);
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
    section.append(&thumbnail_overlay);
    section
}

fn show_screenshot_gallery(
    window: &adw::ApplicationWindow,
    product_id: i64,
    screenshots_list: &[Screenshot],
    initial_index: usize,
) {
    let overlay = gtk::Overlay::new();
    overlay.add_css_class("screenshot-gallery");
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
    stack.set_transition_duration(180);
    for (index, _) in screenshots_list.iter().enumerate() {
        let picture = gtk::Picture::new();
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_widget_name(&format!("gallery-image-{index}"));
        stack.add_named(&picture, Some(&index.to_string()));
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
    {
        let dialog = dialog.clone();
        close.connect_clicked(move |_| {
            dialog.close();
        });
    }

    let (sender, receiver) = mpsc::channel();
    let downloads = screenshots_list.to_vec();
    std::thread::spawn(move || {
        for (index, screenshot) in downloads.iter().enumerate() {
            let result = screenshots::cached_image(product_id, screenshot, true);
            let _ = sender.send((index, result));
        }
    });
    let pictures: Vec<gtk::Picture> = stack
        .pages()
        .iter::<gtk::StackPage>()
        .filter_map(Result::ok)
        .filter_map(|page| page.child().downcast::<gtk::Picture>().ok())
        .collect();
    let expected = pictures.len();
    let received = Rc::new(std::cell::Cell::new(0usize));
    glib::timeout_add_local(Duration::from_millis(50), move || {
        match receiver.try_recv() {
            Ok((image_index, Ok(path))) => {
                if let Some(picture) = pictures.get(image_index) {
                    picture.set_file(Some(&gio::File::for_path(path)));
                }
                let done = received.get() + 1;
                received.set(done);
                if done == expected {
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            }
            Ok((_, Err(error))) => {
                tracing::warn!(%error, "could not load full screenshot");
                let done = received.get() + 1;
                received.set(done);
                if done == expected {
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => glib::ControlFlow::Break,
        }
    });
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

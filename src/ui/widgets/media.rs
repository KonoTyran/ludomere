use adw::prelude::*;
use gdk_pixbuf::{InterpType, Pixbuf};
use gtk::{gdk, gio, glib};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::PathBuf,
    rc::Rc,
};

type CardTextureKey = (PathBuf, i32, i32, u64, u128);

thread_local! {
    static CARD_TEXTURE_CACHE: RefCell<HashMap<CardTextureKey, gdk::Texture>> =
        RefCell::new(HashMap::new());
}

pub(in crate::ui) fn picture(
    path: Option<&PathBuf>,
    width: i32,
    height: i32,
    class: &str,
) -> gtk::Picture {
    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_can_shrink(true);
    picture.set_hexpand(false);
    if width > 0 {
        picture.set_width_request(width);
    }
    if height > 0 {
        picture.set_height_request(height);
    }
    picture.add_css_class(class);
    if let Some(path) = path {
        picture.set_file(Some(&gio::File::for_path(path)));
    }
    picture
}

pub(in crate::ui) fn detail_hero_picture(path: Option<&PathBuf>) -> gtk::Picture {
    let picture = gtk::Picture::new();
    picture.set_widget_name("detail-hero-image");
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_can_shrink(true);
    picture.set_height_request(320);
    picture.set_hexpand(true);
    picture.set_halign(gtk::Align::Fill);
    picture.set_valign(gtk::Align::Center);
    picture.add_css_class("detail-hero");
    if let Some(path) = path {
        picture.set_file(Some(&gio::File::for_path(path)));
    }
    picture
}

pub(in crate::ui) fn install_smooth_wheel_scroll(scrolled: &gtk::ScrolledWindow) {
    const WHEEL_STEP: f64 = 110.0;
    const EASING: f64 = 0.24;

    let adjustment = scrolled.vadjustment();
    let target = Rc::new(Cell::new(adjustment.value()));
    let animating = Rc::new(Cell::new(false));
    let controller = gtk::EventControllerScroll::new(
        gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE,
    );
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let scrolled = scrolled.clone();
        let adjustment = adjustment.clone();
        let target = target.clone();
        let animating = animating.clone();
        controller.connect_scroll(move |_, _, dy| {
            let maximum = (adjustment.upper() - adjustment.page_size()).max(0.0);
            let origin = if animating.get() {
                target.get()
            } else {
                adjustment.value()
            };
            target.set((origin + dy * WHEEL_STEP).clamp(0.0, maximum));
            if !animating.replace(true) {
                let adjustment = adjustment.clone();
                let target = target.clone();
                let animating = animating.clone();
                scrolled.add_tick_callback(move |_, _| {
                    let current = adjustment.value();
                    let destination = target.get();
                    let remaining = destination - current;
                    if remaining.abs() < 0.5 {
                        adjustment.set_value(destination);
                        animating.set(false);
                        glib::ControlFlow::Break
                    } else {
                        adjustment.set_value(current + remaining * EASING);
                        glib::ControlFlow::Continue
                    }
                });
            }
            glib::Propagation::Stop
        });
    }
    scrolled.add_controller(controller);
}

pub(in crate::ui) fn parallax_detail_hero(
    path: Option<&PathBuf>,
    adjustment: &gtk::Adjustment,
) -> gtk::ScrolledWindow {
    const VIEWPORT_HEIGHT: i32 = 320;
    const ARTWORK_OVERFLOW: f64 = 160.0;
    const PARALLAX_RATE: f64 = 0.50;

    let picture = detail_hero_picture(path);
    let artwork = gtk::Box::new(gtk::Orientation::Vertical, 0);
    artwork.set_height_request(VIEWPORT_HEIGHT + ARTWORK_OVERFLOW as i32);
    let overflow_space = gtk::Box::new(gtk::Orientation::Vertical, 0);
    overflow_space.set_height_request(ARTWORK_OVERFLOW as i32);
    overflow_space.set_vexpand(false);
    artwork.append(&overflow_space);
    artwork.append(&picture);
    let viewport = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .min_content_height(VIEWPORT_HEIGHT)
        .max_content_height(VIEWPORT_HEIGHT)
        .propagate_natural_height(false)
        .hexpand(true)
        .child(&artwork)
        .build();
    let artwork_adjustment = viewport.vadjustment();
    artwork_adjustment.connect_changed(move |artwork_adjustment| {
        tracing::debug!(
            upper = artwork_adjustment.upper(),
            page_size = artwork_adjustment.page_size(),
            value = artwork_adjustment.value(),
            "hero parallax viewport allocated"
        );
    });
    {
        let artwork_adjustment = artwork_adjustment.clone();
        viewport.add_tick_callback(move |_, _| {
            let maximum = (artwork_adjustment.upper() - artwork_adjustment.page_size()).max(0.0);
            if maximum <= 0.0 {
                return glib::ControlFlow::Continue;
            }
            artwork_adjustment.set_value(maximum);
            tracing::debug!(
                upper = artwork_adjustment.upper(),
                page_size = artwork_adjustment.page_size(),
                value = artwork_adjustment.value(),
                "hero parallax viewport initialized"
            );
            glib::ControlFlow::Break
        });
    }
    {
        let artwork_adjustment = artwork_adjustment.downgrade();
        let handler = Rc::new(RefCell::new(None));
        let handler_for_callback = handler.clone();
        let handler_id = adjustment.connect_value_changed(move |adjustment| {
            let Some(artwork_adjustment) = artwork_adjustment.upgrade() else {
                if let Some(handler_id) = handler_for_callback.borrow_mut().take() {
                    adjustment.disconnect(handler_id);
                }
                return;
            };
            let maximum = (artwork_adjustment.upper() - artwork_adjustment.page_size()).max(0.0);
            let value = (maximum - adjustment.value() * PARALLAX_RATE).max(0.0);
            artwork_adjustment.set_value(value);
            tracing::trace!(
                page_scroll = adjustment.value(),
                artwork_displacement = maximum - value,
                artwork_adjustment = value,
                artwork_maximum = maximum,
                "hero parallax position changed"
            );
        });
        *handler.borrow_mut() = Some(handler_id);
    }
    viewport
}

pub(in crate::ui) fn card_picture(path: Option<&PathBuf>, width: i32, height: i32) -> gtk::Picture {
    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_can_shrink(true);
    picture.set_size_request(width, height);
    picture.set_hexpand(false);
    picture.set_vexpand(false);
    picture.add_css_class("hero-card");

    if let Some(path) = path
        && let Some(texture) = scaled_card_texture(path, width, height)
    {
        picture.set_paintable(Some(&texture));
    }
    picture
}

pub(in crate::ui) fn scaled_card_texture(
    path: &PathBuf,
    width: i32,
    height: i32,
) -> Option<gdk::Texture> {
    let metadata = path.metadata().ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos());
    let key = (path.clone(), width, height, metadata.len(), modified);
    if let Some(texture) = CARD_TEXTURE_CACHE.with(|cache| cache.borrow().get(&key).cloned()) {
        return Some(texture);
    }
    let source = Pixbuf::from_file(path).ok()?;
    let source_width = source.width();
    let source_height = source.height();
    let target_ratio = width as f64 / height as f64;
    let source_ratio = source_width as f64 / source_height as f64;
    let (x, y, crop_width, crop_height) = if source_ratio > target_ratio {
        let crop_width = (source_height as f64 * target_ratio).round() as i32;
        (
            (source_width - crop_width) / 2,
            0,
            crop_width,
            source_height,
        )
    } else {
        let crop_height = (source_width as f64 / target_ratio).round() as i32;
        (
            0,
            (source_height - crop_height) / 2,
            source_width,
            crop_height,
        )
    };
    let cropped = source.new_subpixbuf(x, y, crop_width, crop_height);
    let scaled = cropped.scale_simple(width, height, InterpType::Bilinear)?;
    let texture = gdk::Texture::for_pixbuf(&scaled);
    CARD_TEXTURE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.retain(|(cached_path, cached_width, cached_height, _, _), _| {
            cached_path != path || *cached_width != width || *cached_height != height
        });
        cache.insert(key, texture.clone());
    });
    Some(texture)
}

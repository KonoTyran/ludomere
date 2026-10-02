use adw::prelude::*;
use gdk_pixbuf::{InterpType, Pixbuf};
use gtk::{gdk, gio, glib};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    rc::Rc,
};

type CardTextureKey = (PathBuf, i32, i32);
type DecodedCard = (Vec<u8>, bool, i32);

struct CardPictures {
    pending: HashMap<CardTextureKey, Vec<glib::WeakRef<gtk::Picture>>>,
    queue: VecDeque<CardTextureKey>,
    sender: std::sync::mpsc::Sender<(u64, CardTextureKey)>,
    receiver: std::sync::mpsc::Receiver<(u64, CardTextureKey, Option<DecodedCard>)>,
    generation: u64,
    active: usize,
    polling: bool,
}

impl CardPictures {
    fn new() -> Self {
        let (sender, jobs) = std::sync::mpsc::channel::<(u64, CardTextureKey)>();
        let jobs = std::sync::Arc::new(std::sync::Mutex::new(jobs));
        let (results, receiver) = std::sync::mpsc::channel();
        for _ in 0..2 {
            let jobs = jobs.clone();
            let results = results.clone();
            std::thread::spawn(move || {
                loop {
                    let Ok((generation, key)) = jobs.lock().unwrap().recv() else {
                        break;
                    };
                    let decoded = decode_card(&key.0, key.1, key.2);
                    if results.send((generation, key, decoded)).is_err() {
                        break;
                    }
                }
            });
        }
        Self {
            pending: HashMap::new(),
            queue: VecDeque::new(),
            sender,
            receiver,
            active: 0,
            generation: 0,
            polling: false,
        }
    }
}

thread_local! {
    static CARD_TEXTURE_CACHE: RefCell<HashMap<CardTextureKey, gdk::Texture>> =
        RefCell::new(HashMap::new());
    static CARD_PICTURES: RefCell<CardPictures> = RefCell::new(CardPictures::new());
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
    controller.set_propagation_phase(gtk::PropagationPhase::Bubble);
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

    if let Some(path) = path {
        set_card_picture(&picture, path, width, height);
    }
    picture
}

pub(in crate::ui) fn set_card_picture(
    picture: &gtk::Picture,
    path: &Path,
    width: i32,
    height: i32,
) {
    let key = (path.to_path_buf(), width.max(1), height.max(1));
    CARD_PICTURES.with(|state| {
        for pictures in state.borrow_mut().pending.values_mut() {
            pictures.retain(|candidate| candidate.upgrade().is_some_and(|value| value != *picture));
        }
    });
    if let Some(texture) = CARD_TEXTURE_CACHE.with(|cache| cache.borrow().get(&key).cloned()) {
        picture.set_paintable(Some(&texture));
        set_picture_status(picture, "image-ready", "Image loaded");
        return;
    }
    set_picture_status(picture, "image-loading", "Loading image…");
    CARD_PICTURES.with(|state| {
        let mut state = state.borrow_mut();
        if !state.pending.contains_key(&key) {
            state.queue.push_back(key.clone());
        }
        state
            .pending
            .entry(key)
            .or_default()
            .push(picture.downgrade());
        if state.polling {
            return;
        }
        state.polling = true;
        glib::timeout_add_local(std::time::Duration::from_millis(16), || {
            CARD_PICTURES.with(|state| {
                let mut state = state.borrow_mut();
                for _ in 0..16 {
                    let Ok((generation, key, decoded)) = state.receiver.try_recv() else {
                        break;
                    };
                    state.active -= 1;
                    if generation != state.generation {
                        continue;
                    }
                    let pictures = state.pending.remove(&key).unwrap_or_default();
                    if pictures.is_empty() {
                        continue;
                    }
                    if let Some((pixels, alpha, stride)) = decoded {
                        let pixbuf = Pixbuf::from_bytes(
                            &glib::Bytes::from_owned(pixels),
                            gdk_pixbuf::Colorspace::Rgb,
                            alpha,
                            8,
                            key.1,
                            key.2,
                            stride,
                        );
                        let texture = gdk::Texture::for_pixbuf(&pixbuf);
                        for picture in pictures.into_iter().filter_map(|picture| picture.upgrade())
                        {
                            picture.set_paintable(Some(&texture));
                            set_picture_status(&picture, "image-ready", "Image loaded");
                        }
                        CARD_TEXTURE_CACHE.with(|cache| {
                            let mut cache = cache.borrow_mut();
                            // Limit memory when the user changes card sizes repeatedly.
                            if cache.len() >= 1024 {
                                cache.clear();
                            }
                            cache.insert(key, texture);
                        });
                    } else {
                        for picture in pictures.into_iter().filter_map(|picture| picture.upgrade())
                        {
                            set_picture_status(
                                &picture,
                                "image-error",
                                "Image could not be decoded",
                            );
                        }
                    }
                }
                while state.active < 2 && !state.queue.is_empty() {
                    let next = state
                        .queue
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, key)| {
                            state
                                .pending
                                .get(*key)
                                .into_iter()
                                .flatten()
                                .filter_map(|picture| picture.upgrade())
                                .map(|picture| picture_viewport_priority(&picture))
                                .min()
                                .unwrap_or(4)
                        })
                        .map(|(index, _)| index)
                        .unwrap_or(0);
                    let key = state.queue.remove(next).unwrap();
                    if state.pending.get(&key).is_none_or(|pictures| {
                        pictures.iter().all(|picture| picture.upgrade().is_none())
                    }) {
                        state.pending.remove(&key);
                        continue;
                    }
                    if state.sender.send((state.generation, key)).is_ok() {
                        state.active += 1;
                    }
                }
                if state.active == 0 && state.queue.is_empty() {
                    state.polling = false;
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            })
        });
    });
}

pub(in crate::ui) fn clear_card_texture_cache() {
    CARD_TEXTURE_CACHE.with(|cache| cache.borrow_mut().clear());
    CARD_PICTURES.with(|state| {
        let mut state = state.borrow_mut();
        state.queue.clear();
        state.pending.clear();
        state.generation = state.generation.wrapping_add(1);
    });
}

pub(in crate::ui) fn set_picture_status(picture: &gtk::Picture, status: &str, message: &str) {
    picture.set_tooltip_text(Some(message));
    for class in [
        "image-pending",
        "image-loading",
        "image-ready",
        "image-error",
        "image-unavailable",
    ] {
        if class != status {
            picture.remove_css_class(class);
        }
    }
    picture.add_css_class(status);
}

fn picture_viewport_priority(picture: &gtk::Picture) -> u8 {
    if !picture.is_mapped() {
        return 3;
    }
    let Some(scroll) = picture
        .ancestor(gtk::ScrolledWindow::static_type())
        .and_downcast::<gtk::ScrolledWindow>()
    else {
        return 0;
    };
    let Some(bounds) = picture.compute_bounds(&scroll) else {
        return 3;
    };
    if bounds.y() + bounds.height() >= 0.0 && bounds.y() <= scroll.height() as f32 {
        0
    } else if bounds.y() + bounds.height() >= -(scroll.height() as f32)
        && bounds.y() <= 2.0 * scroll.height() as f32
    {
        1
    } else {
        2
    }
}

fn decode_card(path: &PathBuf, width: i32, height: i32) -> Option<DecodedCard> {
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
    Some((
        scaled.read_pixel_bytes().to_vec(),
        scaled.has_alpha(),
        scaled.rowstride(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires isolated Xvfb and private D-Bus; exercises GTK image queue and card indicators"]
    fn delayed_card_batches_settle_failures_and_discard_rebound_results() {
        adw::init().expect("requires an isolated GTK display");
        let directory = tempfile::tempdir().unwrap();
        let images = (0..120)
            .map(|id| {
                let path = directory.path().join(format!("{id}.png"));
                image::RgbImage::from_pixel(4, 4, image::Rgb([id, 40, 60]))
                    .save(&path)
                    .unwrap();
                path
            })
            .collect::<Vec<_>>();
        let cards = (0..120)
            .map(|id| {
                let game = crate::domain::Game {
                    product_id: id,
                    title: format!("Game {id}"),
                    ..Default::default()
                };
                let card = crate::ui::game_card(&game, false, 140);
                crate::ui::apply_card_cover_state(
                    card.upcast_ref(),
                    Some(&crate::ui::CoverState::Pending),
                );
                card
            })
            .collect::<Vec<_>>();
        let pictures = cards
            .iter()
            .map(|card| {
                crate::ui::find_named_descendant(card.upcast_ref(), "card-art")
                    .and_downcast::<gtk::Picture>()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert!(
            pictures
                .iter()
                .all(|picture| picture.has_css_class("image-pending"))
        );
        for (picture, image) in pictures[..50].iter().zip(&images[..50]) {
            set_card_picture(picture, image, 140, 78);
        }
        let drain = || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while CARD_PICTURES.with(|state| state.borrow().polling) {
                assert!(
                    std::time::Instant::now() < deadline,
                    "image decoder queue stalled"
                );
                glib::MainContext::default().iteration(false);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        };
        drain();
        assert!(
            pictures[..50]
                .iter()
                .all(|picture| picture.has_css_class("image-ready"))
        );
        assert!(
            pictures[50..]
                .iter()
                .all(|picture| picture.has_css_class("image-pending"))
        );
        std::thread::sleep(std::time::Duration::from_millis(40));
        let corrupt = directory.path().join("corrupt.png");
        std::fs::write(&corrupt, b"broken image").unwrap();
        for (index, picture) in pictures[50..].iter().enumerate() {
            set_card_picture(
                picture,
                if index == 3 {
                    &corrupt
                } else {
                    &images[index + 50]
                },
                140,
                78,
            );
        }
        drain();
        assert!(pictures[53].has_css_class("image-error"));
        assert!(
            pictures
                .iter()
                .enumerate()
                .all(|(id, picture)| id == 53 || picture.has_css_class("image-ready"))
        );
        let status =
            crate::ui::find_named_descendant(cards[53].upcast_ref(), "card-image-status").unwrap();
        assert!(status.is_visible());
        assert!(
            !status
                .first_child()
                .unwrap()
                .downcast::<gtk::Spinner>()
                .unwrap()
                .is_spinning()
        );
        // Rebinding to a cached image must detach an older failed request.
        set_card_picture(&pictures[53], &corrupt, 140, 78);
        set_card_picture(&pictures[53], &images[0], 140, 78);
        drain();
        assert!(pictures[53].has_css_class("image-ready"));
        // A cache clear also invalidates any results still in flight.
        set_card_picture(&pictures[53], &corrupt, 140, 78);
        clear_card_texture_cache();
        set_card_picture(&pictures[53], &images[53], 140, 78);
        drain();
        assert!(pictures[53].has_css_class("image-ready"));
        assert!(!status.is_visible());
    }

    #[test]
    fn cover_worker_crops_center_without_a_gtk_context() {
        let directory = tempfile::tempdir().unwrap();
        for (width, height) in [(12, 4), (4, 12)] {
            let path = directory.path().join(format!("{width}-{height}.png"));
            image::RgbImage::from_fn(width, height, |x, y| {
                if (width == 12 && (4..8).contains(&x)) || (height == 12 && (4..8).contains(&y)) {
                    image::Rgb([0, 0, 255])
                } else {
                    image::Rgb([255, 0, 0])
                }
            })
            .save(&path)
            .unwrap();
            let (pixels, alpha, stride) =
                std::thread::spawn(move || decode_card(&path, 2, 2).unwrap())
                    .join()
                    .unwrap();
            assert!(!alpha);
            for y in 0..2 {
                for x in 0..2 {
                    let offset = y * stride as usize + x * 3;
                    assert_eq!(&pixels[offset..offset + 3], &[0, 0, 255]);
                }
            }
        }
    }

    #[test]
    fn corrupt_and_missing_covers_do_not_stop_workers() {
        let directory = tempfile::tempdir().unwrap();
        let corrupt = directory.path().join("corrupt.png");
        std::fs::write(&corrupt, b"not an image").unwrap();
        let valid = directory.path().join("valid.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([20, 40, 60, 128]))
            .save(&valid)
            .unwrap();
        let workers = CardPictures::new();
        for path in [corrupt, directory.path().join("missing.png"), valid.clone()] {
            workers.sender.send((0, (path, 2, 2))).unwrap();
        }
        let results = (0..3)
            .map(|_| {
                let (_, key, decoded) = workers
                    .receiver
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                (key, decoded)
            })
            .collect::<HashMap<_, _>>();
        assert_eq!(
            results.values().filter(|result| result.is_none()).count(),
            2
        );
        let (pixels, alpha, _) = results.get(&(valid, 2, 2)).unwrap().as_ref().unwrap();
        assert!(*alpha);
        assert_eq!(&pixels[..4], &[20, 40, 60, 128]);
    }
}

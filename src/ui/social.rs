use super::*;

pub(super) fn rebuild_social_page(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>) {
    clear(&w.social);
    w.social.add_css_class("collections-page");
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = gtk::Label::new(Some("SOCIAL"));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.add_css_class("collections-heading");
    heading.append(&title);
    let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh.set_tooltip_text(Some("Refresh GOG friends and presence"));
    let w_refresh = w.clone();
    let model_refresh = model.clone();
    refresh.connect_clicked(move |button| {
        button.set_sensitive(false);
        start_social_sync(&w_refresh, &model_refresh);
    });
    heading.append(&refresh);
    w.social.append(&heading);

    let state = model.borrow();
    let status = match (state.network_available, state.social_synced_at) {
        (false, Some(timestamp)) => format!("Offline · cached {}", format_social_time(timestamp)),
        (_, Some(timestamp)) => format!("Last synchronized {}", format_social_time(timestamp)),
        _ => "Not synchronized".into(),
    };
    let status = gtk::Label::new(Some(&status));
    status.set_xalign(0.0);
    status.add_css_class("dim-label");
    w.social.append(&status);

    let requests = gtk::Label::new(Some(&match state.invitation_count {
        Some(0) => "No incoming friend requests".into(),
        Some(count) => format!("{count} incoming friend request(s)"),
        None => "Friend request details are unavailable from the validated endpoint".into(),
    }));
    requests.set_xalign(0.0);
    requests.set_margin_top(12);
    w.social.append(&requests);

    let search = gtk::SearchEntry::builder()
        .placeholder_text("User search unavailable until its endpoint is validated")
        .sensitive(false)
        .build();
    search.set_margin_top(14);
    w.social.append(&search);

    let friends = state.friends.clone();
    drop(state);
    let group = adw::PreferencesGroup::new();
    group.set_title("Friends");
    group.set_margin_top(16);
    if friends.is_empty() {
        let empty = adw::ActionRow::builder()
            .title("No cached friends")
            .subtitle("Refresh while signed in to GOG.")
            .build();
        group.add(&empty);
    }
    for cached in friends {
        let (presence, current_game) = presence_summary(cached.presence.as_ref());
        let subtitle = current_game.map_or(presence.clone(), |game| format!("{presence} · {game}"));
        let row = adw::ActionRow::builder()
            .title(&cached.friend.username)
            .subtitle(&subtitle)
            .activatable(true)
            .build();
        let indicator = gtk::Image::from_icon_name(if presence == "Offline" {
            "user-offline-symbolic"
        } else {
            "user-available-symbolic"
        });
        row.add_prefix(&indicator);
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        let w = w.clone();
        let model = model.clone();
        row.connect_activated(move |_| show_friend_profile(&w, &model, &cached));
        group.add(&row);
    }
    w.social.append(&group);
}

pub(super) fn start_social_sync(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>) {
    let Some(token) = model.borrow().account_token.clone() else {
        rebuild_social_page(w, model);
        return;
    };
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<_> {
            let client = crate::gog::client()?;
            let friends = crate::gog::friends::list(&client, &token)?;
            let ids = friends
                .iter()
                .map(|friend| friend.user_id.clone())
                .collect::<Vec<_>>();
            let presence = crate::gog::presence::statuses(&client, &token, &ids)?;
            let invitations = crate::gog::friends::invitation_count(&client, &token)?;
            let store = StateStore::open()?;
            store.replace_social_friends(&token.user_id, &friends, &presence)?;
            let cached = store.cached_friends(&token.user_id)?;
            Ok((cached, invitations, chrono::Utc::now().timestamp()))
        })();
        let _ = sender.send(result);
    });
    let w = w.clone();
    let model = model.clone();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        match receiver.try_recv() {
            Ok(Ok((friends, invitations, synchronized_at))) => {
                let mut state = model.borrow_mut();
                state.friends = friends;
                state.invitation_count = Some(invitations);
                state.social_synced_at = Some(synchronized_at);
                drop(state);
                rebuild_social_page(&w, &model);
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                tracing::warn!(%error, "GOG social synchronization failed");
                rebuild_social_page(&w, &model);
                show_status(
                    &w,
                    "Could not refresh GOG social data; cached friends remain available",
                );
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

fn show_friend_profile(
    w: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
    cached: &crate::state::CachedFriend,
) {
    clear(&w.social);
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let back = gtk::Button::from_icon_name("go-previous-symbolic");
    let w_back = w.clone();
    let model_back = model.clone();
    back.connect_clicked(move |_| rebuild_social_page(&w_back, &model_back));
    heading.append(&back);
    let title = gtk::Label::new(Some(&cached.friend.username));
    title.add_css_class("collections-heading");
    heading.append(&title);
    w.social.append(&heading);
    let (presence, current_game) = presence_summary(cached.presence.as_ref());
    let group = adw::PreferencesGroup::new();
    group.set_margin_top(18);
    group.add(
        &adw::ActionRow::builder()
            .title("Status")
            .subtitle(&presence)
            .build(),
    );
    group.add(
        &adw::ActionRow::builder()
            .title("Current game")
            .subtitle(current_game.as_deref().unwrap_or("Not published"))
            .build(),
    );
    group.add(
        &adw::ActionRow::builder()
            .title("Achievement and session comparison")
            .subtitle("Unavailable until the comparison/session endpoints pass validation")
            .build(),
    );
    group.add(
        &adw::ActionRow::builder()
            .title("Blocked-user management")
            .subtitle("Unavailable until block-list and mutation endpoints pass validation")
            .build(),
    );
    w.social.append(&group);
}

fn presence_summary(
    presence: Option<&crate::gog::presence::GogPresence>,
) -> (String, Option<String>) {
    let Some(presence) = presence else {
        return ("Offline".into(), None);
    };
    let status = presence
        .data
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Online");
    let status = if status.eq_ignore_ascii_case("offline") {
        "Offline".into()
    } else {
        status.to_owned()
    };
    let game = presence
        .data
        .pointer("/game/title")
        .or_else(|| presence.data.get("game_title"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    (status, game)
}

fn format_social_time(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "at an unknown time".into())
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

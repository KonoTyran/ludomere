use super::*;

pub(super) fn achievement_page(model: &Rc<RefCell<AppModel>>, product_id: i64) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let spinner = gtk::Spinner::new();
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.set_hexpand(true);
    let refresh = gtk::Button::with_label("Refresh achievements");
    controls.append(&spinner);
    controls.append(&status);
    controls.append(&refresh);
    page.append(&controls);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    page.append(&content);
    let page_epoch = model.borrow().account_epoch;
    let page_session = online::account_session();
    {
        let page = page.downgrade();
        let model = model.clone();
        let content = content.clone();
        let status = status.clone();
        let spinner = spinner.clone();
        let refresh = refresh.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            if page.upgrade().is_none() {
                return glib::ControlFlow::Break;
            }
            if model.borrow().account_epoch == page_epoch
                && !model.borrow().logout_pending
                && online::account_session() == page_session
            {
                return glib::ControlFlow::Continue;
            }
            while let Some(child) = content.first_child() {
                content.remove(&child);
            }
            spinner.stop();
            spinner.set_visible(false);
            refresh.set_sensitive(false);
            status.set_label(
                "Account changed. Go to Home, then reopen this game to view the current account's achievements.",
            );
            glib::ControlFlow::Break
        });
    }
    let request: Rc<dyn Fn()> = Rc::new({
        let model = model.clone();
        let page = page.downgrade();
        let content = content.clone();
        let status = status.clone();
        let spinner = spinner.clone();
        let refresh = refresh.downgrade();
        move || {
            let Some(refresh) = refresh.upgrade() else {
                return;
            };
            let (account, token, online, epoch, generation, session) = {
                let state = model.borrow();
                if state.logout_pending
                    || state.account_epoch != page_epoch
                    || online::account_session() != page_session
                {
                    return;
                }
                (
                    state
                        .account_token
                        .as_ref()
                        .map(|token| token.user_id.clone())
                        .or_else(|| state.account_profile.as_ref().map(|p| p.user_id.clone())),
                    state.account_token.clone(),
                    state.network_available,
                    state.account_epoch,
                    state.detail_generation,
                    online::account_session(),
                )
            };
            let Some(account) = account else {
                status.set_label("Sign in to GOG to view your achievements.");
                return;
            };
            refresh.set_sensitive(false);
            spinner.set_spinning(true);
            spinner.set_visible(true);
            status.set_label("Loading achievements…");
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let _activity = match crate::profile_reset::begin_activity("loading achievements") {
                    Ok(activity) => activity,
                    Err(error) => {
                        let _ = sender.send((true, Err(error)));
                        return;
                    }
                };
                let cached = online::with_account_session(session, || {
                    StateStore::open()?.cached_achievements(&account, product_id)
                });
                let has_cached = cached.as_ref().is_ok_and(Option::is_some);
                let cache_error = cached.as_ref().err().map(|error| format!("{error:#}"));
                let _ = sender.send((false, cached));
                let result = if online {
                    token.as_ref().map_or_else(
                        || Err(anyhow::anyhow!("Sign in again to refresh achievements.")),
                        |token| {
                            crate::gog::achievements::refresh(token, product_id, session).map(Some)
                        },
                    )
                } else {
                    Err(anyhow::anyhow!(if has_cached {
                        "Offline. Cached achievements remain available."
                    } else if cache_error.is_some() {
                        "Offline. Cached achievements could not be loaded."
                    } else {
                        "Offline. No cached achievements are available."
                    }))
                };
                let result = result.map_err(|error| match cache_error {
                    Some(cache_error) => {
                        error.context(format!("Could not read cached achievements: {cache_error}"))
                    }
                    None => error,
                });
                let _ = sender.send((true, result));
            });
            let model = model.clone();
            let page = page.clone();
            let content = content.clone();
            let status = status.clone();
            let spinner = spinner.clone();
            let refresh = refresh.clone();
            glib::timeout_add_local(Duration::from_millis(50), move || {
                if page.upgrade().is_none()
                    || model.borrow().account_epoch != epoch
                    || model.borrow().detail_generation != generation
                    || model.borrow().logout_pending
                    || online::account_session() != session
                {
                    return glib::ControlFlow::Break;
                }
                let (terminal, result) = match receiver.try_recv() {
                    Ok(value) => value,
                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(_) => (
                        true,
                        Err(anyhow::anyhow!("Achievement loading stopped. Retry.")),
                    ),
                };
                match result {
                    Ok(Some(cached)) => {
                        render(&content, &cached);
                        if terminal {
                            status.set_label("Achievements up to date");
                        }
                    }
                    Ok(None) => {}
                    Err(error) if terminal => status.set_label(
                        super::notifications::failure_message("", &format!("{error:#}"))
                            .trim_start(),
                    ),
                    Err(_) => {}
                }
                if terminal {
                    spinner.set_spinning(false);
                    spinner.set_visible(false);
                    refresh.set_sensitive(true);
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
        }
    });
    refresh.connect_clicked({
        let request = request.clone();
        move |_| request()
    });
    request();
    page
}

fn render(content: &gtk::Box, cached: &crate::state::CachedAchievements) {
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }
    let unlocked = cached
        .achievements
        .iter()
        .filter(|a| a.unlocked_at.is_some())
        .count();
    let summary = gtk::Label::new(Some(&format!(
        "{unlocked} unlocked · cached {}",
        chrono::DateTime::from_timestamp(cached.updated_at, 0)
            .map(|time| time
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string())
            .unwrap_or_else(|| "at an unknown time".into())
    )));
    summary.set_xalign(0.0);
    summary.add_css_class("dim-label");
    content.append(&summary);
    let group = adw::PreferencesGroup::new();
    if cached.achievements.is_empty() {
        group.add(
            &adw::ActionRow::builder()
                .title("No achievements returned by GOG")
                .build(),
        );
    }
    for achievement in &cached.achievements {
        if !achievement.visible && achievement.unlocked_at.is_none() {
            continue;
        }
        let state = achievement
            .unlocked_at
            .as_ref()
            .map_or_else(|| "Locked".into(), |date| format!("Unlocked {date}"));
        let mut description = vec![state];
        if !achievement.description.is_empty() {
            description.push(achievement.description.clone());
        }
        if let Some(progress) = achievement.progress {
            description.push(achievement.progress_max.map_or_else(
                || format!("Progress: {progress}"),
                |maximum| format!("Progress: {progress} / {maximum}"),
            ));
        }
        if let Some(rarity) = achievement.rarity {
            description.push(format!("Rarity: {rarity}%"));
        }
        let row = adw::ActionRow::builder()
            .title(if achievement.name.is_empty() {
                &achievement.key
            } else {
                &achievement.name
            })
            .subtitle(description.join(" · "))
            .build();
        row.set_use_markup(false);
        row.add_prefix(&gtk::Image::from_icon_name(
            if achievement.unlocked_at.is_some() {
                "emblem-ok-symbolic"
            } else {
                "changes-prevent-symbolic"
            },
        ));
        group.add(&row);
    }
    content.append(&group);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires isolated HOME/all XDG and private GTK; offline synthetic account only"]
    fn offline_achievements_explain_cache_errors_retry_and_account_changes() {
        assert!(
            std::env::var("HOME")
                .unwrap()
                .starts_with("/tmp/ludomere-p274-")
        );
        adw::init().unwrap();
        fn wait_until(check: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !check() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(check());
        }
        fn text(widget: &gtk::Widget) -> String {
            let mut value = widget
                .downcast_ref::<gtk::Label>()
                .map(|label| label.label().to_string())
                .unwrap_or_default();
            let mut child = widget.first_child();
            while let Some(current) = child {
                value.push_str(&text(&current));
                child = current.next_sibling();
            }
            value
        }
        let model = Rc::new(RefCell::new(AppModel {
            account_profile: Some(auth::Profile {
                user_id: "synthetic-account".into(),
                ..Default::default()
            }),
            network_available: false,
            ..AppModel::default()
        }));
        let database = crate::identity::database();
        assert!(database.starts_with(std::env::var_os("XDG_DATA_HOME").unwrap()));
        std::fs::create_dir_all(&database).unwrap();
        let page = achievement_page(&model, 9274001);
        let controls = page.first_child().unwrap();
        let spinner = controls
            .first_child()
            .and_downcast::<gtk::Spinner>()
            .unwrap();
        let status = spinner.next_sibling().and_downcast::<gtk::Label>().unwrap();
        let refresh = controls.last_child().and_downcast::<gtk::Button>().unwrap();
        let content = page.last_child().and_downcast::<gtk::Box>().unwrap();
        assert!(spinner.is_spinning());
        assert!(!refresh.is_sensitive());
        wait_until(|| refresh.is_sensitive());
        assert!(!spinner.is_spinning());
        assert!(
            status
                .label()
                .contains("Could not read cached achievements")
        );
        assert!(status.label().contains("Offline"));
        std::fs::remove_dir(&database).unwrap();
        refresh.emit_clicked();
        wait_until(|| refresh.is_sensitive());
        assert_eq!(
            status.label(),
            "Offline. No cached achievements are available."
        );
        StateStore::open()
            .unwrap()
            .replace_achievements(
                "synthetic-account",
                9274001,
                &[crate::gog::achievements::Achievement {
                    id: "synthetic".into(),
                    key: "synthetic".into(),
                    name: "Synthetic achievement".into(),
                    description: "Offline fixture".into(),
                    visible: true,
                    unlocked_at: None,
                    progress: None,
                    progress_max: None,
                    rarity: None,
                }],
            )
            .unwrap();
        refresh.emit_clicked();
        wait_until(|| refresh.is_sensitive());
        assert_eq!(
            status.label(),
            "Offline. Cached achievements remain available."
        );
        assert!(text(content.upcast_ref()).contains("Synthetic achievement"));
        let reservation = crate::profile_reset::reserve().unwrap();
        refresh.emit_clicked();
        wait_until(|| refresh.is_sensitive());
        assert!(status.label().contains("Profile reset"));
        assert!(text(content.upcast_ref()).contains("Synthetic achievement"));
        drop(reservation);
        online::invalidate_library_session();
        wait_until(|| status.label().contains("Account changed"));
        assert!(!refresh.is_sensitive());
        assert!(!spinner.is_spinning());
        assert!(content.first_child().is_none());
        refresh.emit_clicked();
        assert!(status.label().contains("Account changed"));
        assert!(content.first_child().is_none());
    }
}

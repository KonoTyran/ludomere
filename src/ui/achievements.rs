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
            if model.borrow().account_epoch == page_epoch {
                return glib::ControlFlow::Continue;
            }
            while let Some(child) = content.first_child() {
                content.remove(&child);
            }
            spinner.stop();
            spinner.set_visible(false);
            refresh.set_sensitive(false);
            status.set_label(
                "Account changed. Reopen this game to view the current account's achievements.",
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
                if state.logout_pending || state.account_epoch != page_epoch {
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
                let cached = StateStore::open()
                    .and_then(|store| store.cached_achievements(&account, product_id));
                let _ = sender.send((false, cached));
                let result = if online {
                    token.as_ref().map_or_else(
                        || Err(anyhow::anyhow!("Sign in again to refresh achievements.")),
                        |token| {
                            crate::gog::achievements::refresh(token, product_id, session).map(Some)
                        },
                    )
                } else {
                    Err(anyhow::anyhow!(
                        "Offline. Cached achievements remain available."
                    ))
                };
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
                    Err(error) if terminal => status.set_label(&error.to_string()),
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

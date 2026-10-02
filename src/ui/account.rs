use super::*;

fn account_result_is_current(
    expected: (u64, u64),
    current: (u64, u64),
    logout_pending: bool,
) -> bool {
    expected == current && !logout_pending
}

pub(super) fn show_gog_login(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>) {
    use webkit6::prelude::*;
    if model.borrow().logout_pending {
        return;
    }
    let epoch = model.borrow().account_epoch;

    let web_view = webkit6::WebView::builder()
        .network_session(&webkit6::NetworkSession::new_ephemeral())
        .build();
    web_view.load_uri(&auth::login_url());
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(
        "Sign in to GOG",
        "Secure GOG login",
    )));
    root.append(&header);
    root.append(&web_view);
    web_view.set_vexpand(true);
    let dialog = adw::Dialog::builder()
        .content_width(900)
        .content_height(700)
        .child(&root)
        .build();
    {
        let dialog = dialog.clone();
        let w = w.clone();
        let model = model.clone();
        web_view.connect_decide_policy(move |_, decision, _| {
            if model.borrow().account_epoch != epoch || model.borrow().logout_pending {
                decision.ignore();
                dialog.close();
                return true;
            }
            let uri = decision
                .clone()
                .downcast::<webkit6::NavigationPolicyDecision>()
                .ok()
                .and_then(|navigation| navigation.navigation_action())
                .and_then(|action| action.request())
                .and_then(|request| request.uri());
            let Some(code) = uri.as_deref().and_then(auth::authorization_code) else {
                return false;
            };
            decision.ignore();
            dialog.close();
            begin_account_exchange(&w, &model, code);
            true
        });
    }
    dialog.present(Some(&w.window));
}

pub(super) fn begin_account_exchange(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>, code: String) {
    if model.borrow().logout_pending {
        return;
    }
    // A new login must not lend its credentials to an older game's cloud callbacks.
    auth::invalidate_session();
    let epoch = model.borrow().account_epoch;
    show_progress(w, "Signing in to GOG…");
    w.sign_in.set_sensitive(false);
    let (sender, receiver) = mpsc::channel();
    let auth_session = auth::session();
    std::thread::spawn(move || {
        let _ = sender.send(auth::exchange_code(&code, auth_session));
    });
    poll_account_result(w, model, receiver, epoch, auth_session);
}

pub(super) fn start_account_restore(
    w: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
    _store: &Rc<StateStore>,
) {
    let epoch = model.borrow().account_epoch;
    let (sender, receiver) = mpsc::channel();
    let auth_session = auth::session();
    std::thread::spawn(move || {
        let _ = sender.send(auth::restore(auth_session));
    });
    let w = w.clone();
    let model = model.clone();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        if !account_result_is_current(
            (epoch, auth_session),
            (model.borrow().account_epoch, auth::session()),
            model.borrow().logout_pending,
        ) {
            return glib::ControlFlow::Break;
        }
        match receiver.try_recv() {
            Ok(Ok(Some((token, profile)))) => {
                cache_and_display_profile(&w, &model, token.clone(), profile);
                start_owned_library_sync(&w, &model, token, false, false);
                show_status(&w, "Signed in to GOG");
                glib::ControlFlow::Break
            }
            Ok(Ok(None)) => {
                download::set_authenticated(false);
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                tracing::warn!(message = %auth::sign_in_error_message(&error), "could not restore GOG session");
                download::set_authenticated(false);
                model.borrow_mut().token_refresh_in_progress = false;
                update_header_network_indicator(&w, &model.borrow());
                w.account_library_status
                    .set_label("GOG session unavailable\nAutomatic renewal will retry");
                show_status(
                    &w,
                    "Could not renew the GOG session; sign in again or wait for retry",
                );
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => glib::ControlFlow::Break,
        }
    });
}

pub(super) fn poll_account_result(
    w: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
    receiver: mpsc::Receiver<anyhow::Result<(auth::Token, auth::Profile)>>,
    epoch: u64,
    auth_session: u64,
) {
    let w = w.clone();
    let model = model.clone();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        if !account_result_is_current(
            (epoch, auth_session),
            (model.borrow().account_epoch, auth::session()),
            model.borrow().logout_pending,
        ) {
            return glib::ControlFlow::Break;
        }
        match receiver.try_recv() {
            Ok(Ok((token, profile))) => {
                cache_and_display_profile(&w, &model, token.clone(), profile);
                start_owned_library_sync(&w, &model, token, true, false);
                w.sign_in.set_sensitive(true);
                if w.live_status.label() == "Signing in to GOG…" {
                    show_progress(&w, "");
                }
                show_status(&w, "Signed in to GOG");
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                w.sign_in.set_sensitive(true);
                if w.live_status.label() == "Signing in to GOG…" {
                    show_progress(&w, "");
                }
                show_status(&w, &auth::sign_in_error_message(&error));
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => {
                w.sign_in.set_sensitive(true);
                if w.live_status.label() == "Signing in to GOG…" {
                    show_progress(&w, "");
                    show_status(&w, "Sign-in did not finish. Try signing in again.");
                }
                glib::ControlFlow::Break
            }
        }
    });
}

pub(super) fn cache_and_display_profile(
    w: &Widgets,
    model: &Rc<RefCell<AppModel>>,
    token: auth::Token,
    profile: auth::Profile,
) {
    let profile_to_cache = profile.clone();
    let auth_session = auth::session();
    std::thread::spawn(move || {
        if let Err(error) = auth::cache_profile_if_current(&profile_to_cache, auth_session) {
            tracing::warn!(%error, "could not cache GOG profile");
        }
    });
    let mut state = model.borrow_mut();
    if state.account_profile.as_ref().map(|value| &value.user_id) != Some(&profile.user_id) {
        w.notifications.clear();
        show_progress(w, "");
        invalidate_section_requests(&mut state);
    }
    state.account_profile = Some(profile.clone());
    state.account_token = Some(token);
    if let Some(token) = state.account_token.as_ref() {
        download::recover(token.access_token.clone());
    }
    state.token_refresh_in_progress = false;
    update_account_widgets(w, Some(&profile));
    update_header_network_indicator(w, &state);
}

pub(super) fn start_token_renewal_monitor(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>) {
    let w = w.clone();
    let model = model.clone();
    glib::timeout_add_local(Duration::from_secs(60), move || {
        let token = {
            let mut state = model.borrow_mut();
            if state.token_refresh_in_progress || state.logout_pending {
                return glib::ControlFlow::Continue;
            }
            let needs_refresh = state
                .account_token
                .as_ref()
                .map_or(state.account_profile.is_some(), |token| {
                    token.expires_at <= chrono::Utc::now().timestamp() + 5 * 60
                });
            if !needs_refresh {
                return glib::ControlFlow::Continue;
            }
            state.token_refresh_in_progress = true;
            state.account_token.clone()
        };
        let epoch = model.borrow().account_epoch;
        let (sender, receiver) = mpsc::channel();
        let auth_session = auth::session();
        std::thread::spawn(move || {
            let result = match token {
                Some(token) => auth::refresh(&token, auth_session).map(Some),
                None => auth::restore(auth_session),
            };
            let _ = sender.send(result);
        });
        let w = w.clone();
        let model = model.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            if !account_result_is_current(
                (epoch, auth_session),
                (model.borrow().account_epoch, auth::session()),
                model.borrow().logout_pending,
            ) {
                return glib::ControlFlow::Break;
            }
            match receiver.try_recv() {
                Ok(Ok(Some((token, profile)))) => {
                    cache_and_display_profile(&w, &model, token, profile);
                    update_account_library_status(&w, &model.borrow());
                    glib::ControlFlow::Break
                }
                Ok(Ok(None)) => {
                    model.borrow_mut().token_refresh_in_progress = false;
                    update_header_network_indicator(&w, &model.borrow());
                    glib::ControlFlow::Break
                }
                Ok(Err(error)) => {
                    tracing::warn!(message = %auth::sign_in_error_message(&error), "automatic GOG token renewal failed");
                    model.borrow_mut().token_refresh_in_progress = false;
                    update_header_network_indicator(&w, &model.borrow());
                    w.account_library_status
                        .set_label("GOG session unavailable\nAutomatic renewal will retry");
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    model.borrow_mut().token_refresh_in_progress = false;
                    glib::ControlFlow::Break
                }
            }
        });
        glib::ControlFlow::Continue
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_restore_or_renewal_cannot_replace_a_new_login_in_the_same_ui_epoch() {
        let (send, receive) = mpsc::channel();
        // The cached account can remain unchanged, so only the auth generation advances.
        let current = (7, 12);
        send.send(((7, 12), "signed in")).unwrap();
        send.send(((7, 11), "restore found no credentials"))
            .unwrap();
        send.send(((7, 11), "renewal failed")).unwrap();
        send.send(((6, 12), "old page result")).unwrap();
        drop(send);
        let mut status = "signing in";
        for (expected, result) in receive {
            if account_result_is_current(expected, current, false) {
                status = result;
            }
        }
        assert_eq!(status, "signed in");
        assert!(!account_result_is_current(current, current, true));
        assert!(!account_result_is_current(current, (7, 13), false));
    }
}

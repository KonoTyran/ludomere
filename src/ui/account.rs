use super::*;

pub(super) fn show_gog_login(w: &Rc<Widgets>, model: &Rc<RefCell<AppModel>>) {
    use webkit6::prelude::*;

    let web_view = webkit6::WebView::new();
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
    show_status(w, "Signing in to GOG…");
    w.sign_in.set_sensitive(false);
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(auth::exchange_code(&code));
    });
    poll_account_result(w, model, receiver);
}

pub(super) fn start_account_restore(
    w: &Rc<Widgets>,
    model: &Rc<RefCell<AppModel>>,
    _store: &Rc<StateStore>,
) {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(auth::restore());
    });
    let w = w.clone();
    let model = model.clone();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        match receiver.try_recv() {
            Ok(Ok(Some((token, profile)))) => {
                cache_and_display_profile(&w, &model, token.clone(), profile);
                start_owned_library_sync(&w, &model, token, false, false);
                glib::ControlFlow::Break
            }
            Ok(Ok(None)) => {
                download::set_authenticated(false);
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                tracing::warn!(%error, "could not restore GOG session");
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
) {
    let w = w.clone();
    let model = model.clone();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        match receiver.try_recv() {
            Ok(Ok((token, profile))) => {
                cache_and_display_profile(&w, &model, token.clone(), profile);
                start_owned_library_sync(&w, &model, token, true, false);
                w.sign_in.set_sensitive(true);
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                w.sign_in.set_sensitive(true);
                show_status(&w, &format!("GOG sign-in failed: {error}"));
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => {
                w.sign_in.set_sensitive(true);
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
    std::thread::spawn(move || {
        if let Ok(store) = StateStore::open()
            && let Err(error) = store.cache_profile(&profile_to_cache)
        {
            tracing::warn!(%error, "could not cache GOG profile");
        }
    });
    let mut state = model.borrow_mut();
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
            if state.token_refresh_in_progress {
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
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = match token {
                Some(token) => auth::refresh(&token).map(Some),
                None => auth::restore(),
            };
            let _ = sender.send(result);
        });
        let w = w.clone();
        let model = model.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
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
                    tracing::warn!(%error, "automatic GOG token renewal failed");
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

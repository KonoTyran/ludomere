use super::*;

fn account_result_is_current(
    expected: (u64, u64),
    current: (u64, u64),
    logout_pending: bool,
) -> bool {
    expected == current && !logout_pending
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LoginPageState {
    Loading,
    Ready,
    Failed,
    Closed,
}

#[derive(Clone)]
struct LoginPageFeedback {
    row: gtk::Box,
    spinner: gtk::Spinner,
    status: gtk::Label,
    retry: gtk::Button,
    state: Rc<std::cell::Cell<LoginPageState>>,
}

impl LoginPageFeedback {
    fn new() -> Self {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.set_margin_start(16);
        row.set_margin_end(16);
        row.set_margin_top(8);
        row.set_margin_bottom(8);
        let spinner = gtk::Spinner::new();
        let status = gtk::Label::new(None);
        status.set_wrap(true);
        status.set_xalign(0.0);
        status.set_hexpand(true);
        let retry = gtk::Button::with_label("Retry");
        row.append(&spinner);
        row.append(&status);
        row.append(&retry);
        let feedback = Self {
            row,
            spinner,
            status,
            retry,
            state: Rc::new(std::cell::Cell::new(LoginPageState::Loading)),
        };
        feedback.show(LoginPageState::Loading);
        feedback
    }

    fn show(&self, state: LoginPageState) {
        if self.state.get() == LoginPageState::Closed {
            return;
        }
        self.state.set(state);
        self.row.set_visible(matches!(
            state,
            LoginPageState::Loading | LoginPageState::Failed
        ));
        self.spinner.set_spinning(state == LoginPageState::Loading);
        self.spinner.set_visible(state == LoginPageState::Loading);
        self.retry.set_visible(state == LoginPageState::Failed);
        self.status.set_label(match state {
            LoginPageState::Loading => "Loading GOG sign-in…",
            LoginPageState::Failed => "The GOG sign-in page is unavailable. Check your connection, then select Retry to reload it.",
            LoginPageState::Ready | LoginPageState::Closed => "",
        });
    }

    fn load_changed(&self, event: webkit6::LoadEvent) {
        match event {
            webkit6::LoadEvent::Started => self.show(LoginPageState::Loading),
            webkit6::LoadEvent::Finished if self.state.get() == LoginPageState::Loading => {
                self.show(LoginPageState::Ready)
            }
            _ => {}
        }
    }

    fn load_failed(&self, error: &glib::Error) {
        if !error.matches(webkit6::NetworkError::Cancelled)
            && !error.matches(gio::IOErrorEnum::Cancelled)
        {
            self.show(LoginPageState::Failed);
        }
    }

    fn connect_retry(&self, active: impl Fn() -> bool + 'static, retry: impl Fn() + 'static) {
        let row = self.row.downgrade();
        let spinner = self.spinner.clone();
        let status = self.status.clone();
        let state = self.state.clone();
        self.retry.connect_clicked(move |button| {
            let Some(row) = row.upgrade() else {
                return;
            };
            let feedback = Self {
                row,
                spinner: spinner.clone(),
                status: status.clone(),
                retry: button.clone(),
                state: state.clone(),
            };
            if feedback.state.get() != LoginPageState::Failed {
                return;
            }
            if !active() {
                feedback.show(LoginPageState::Closed);
                return;
            }
            feedback.show(LoginPageState::Loading);
            retry();
        });
    }
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
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(
        "Sign in to GOG",
        "Secure GOG login",
    )));
    root.append(&header);
    let feedback = LoginPageFeedback::new();
    root.append(&feedback.row);
    root.append(&web_view);
    web_view.set_vexpand(true);
    let dialog = adw::Dialog::builder()
        .content_width(900)
        .content_height(700)
        .child(&root)
        .build();
    feedback.connect_retry(
        {
            let model = model.clone();
            move || model.borrow().account_epoch == epoch && !model.borrow().logout_pending
        },
        {
            let web_view = web_view.downgrade();
            move || {
                if let Some(web_view) = web_view.upgrade() {
                    // Restart from the public login endpoint, never replay an OAuth callback URL.
                    web_view.load_uri(&auth::login_url());
                }
            }
        },
    );
    web_view.connect_load_changed({
        let feedback = feedback.clone();
        move |_, event| feedback.load_changed(event)
    });
    web_view.connect_load_failed({
        let feedback = feedback.clone();
        move |_, _, _, error| {
            feedback.load_failed(error);
            // Keep WebKit's failing URI/error out of the UI: they can contain OAuth credentials.
            true
        }
    });
    web_view.connect_web_process_terminated({
        let feedback = feedback.clone();
        move |_, _| feedback.show(LoginPageState::Failed)
    });
    dialog.connect_closed({
        let feedback = feedback.clone();
        let web_view = web_view.downgrade();
        move |_| {
            feedback.show(LoginPageState::Closed);
            if let Some(web_view) = web_view.upgrade() {
                web_view.stop_loading();
            }
        }
    });
    {
        let dialog = dialog.clone();
        let w = w.clone();
        let model = model.clone();
        let feedback = feedback.clone();
        web_view.connect_decide_policy(move |_, decision, _| {
            if feedback.state.get() == LoginPageState::Closed
                || model.borrow().account_epoch != epoch
                || model.borrow().logout_pending
            {
                decision.ignore();
                feedback.show(LoginPageState::Closed);
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
            feedback.show(LoginPageState::Closed);
            dialog.close();
            begin_account_exchange(&w, &model, code);
            true
        });
    }
    dialog.present(Some(&w.window));
    web_view.load_uri(&auth::login_url());
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
    #[ignore = "requires private GTK display; uses synthetic events, never creates a WebView"]
    fn login_page_feedback_handles_retry_failure_cancel_and_redirect_teardown() {
        adw::init().unwrap();
        let feedback = LoginPageFeedback::new();
        let attempts = Rc::new(std::cell::Cell::new(0));
        let active = Rc::new(std::cell::Cell::new(true));
        feedback.connect_retry(
            {
                let active = active.clone();
                move || active.get()
            },
            {
                let attempts = attempts.clone();
                move || attempts.set(attempts.get() + 1)
            },
        );
        assert!(feedback.row.is_visible());
        assert!(feedback.spinner.is_spinning());
        assert!(!feedback.retry.is_visible());
        feedback.load_changed(webkit6::LoadEvent::Finished);
        assert!(!feedback.row.is_visible());
        feedback.load_changed(webkit6::LoadEvent::Started);
        feedback.load_failed(&glib::Error::new(
            webkit6::NetworkError::Cancelled,
            "expected navigation cancellation",
        ));
        assert!(feedback.spinner.is_spinning());
        assert!(!feedback.retry.is_visible());
        feedback.load_failed(&glib::Error::new(
            webkit6::NetworkError::Transport,
            "https://example.invalid/?code=secret-token",
        ));
        feedback.load_changed(webkit6::LoadEvent::Finished);
        assert!(feedback.row.is_visible());
        assert!(!feedback.spinner.is_spinning());
        assert!(feedback.retry.is_visible());
        assert!(!feedback.status.label().contains("secret-token"));
        feedback.retry.emit_clicked();
        feedback.retry.emit_clicked();
        assert_eq!(
            attempts.get(),
            1,
            "retry is single-flight while navigation starts"
        );
        assert!(feedback.spinner.is_spinning());
        feedback.show(LoginPageState::Failed); // Same transition as a crashed WebKit process.
        active.set(false);
        feedback.retry.emit_clicked();
        assert_eq!(attempts.get(), 1, "stale account must not reload login");
        assert!(!feedback.row.is_visible());
        assert!(!feedback.spinner.is_spinning());

        let redirected = LoginPageFeedback::new();
        redirected.show(LoginPageState::Closed); // Successful OAuth redirect or dialog close.
        redirected.load_changed(webkit6::LoadEvent::Started);
        redirected.load_failed(&glib::Error::new(
            webkit6::NetworkError::Failed,
            "late callback",
        ));
        redirected.show(LoginPageState::Failed);
        assert!(!redirected.row.is_visible());
        assert!(!redirected.spinner.is_spinning());

        let row = feedback.row.downgrade();
        drop(feedback);
        assert!(
            row.upgrade().is_none(),
            "retry wiring must not retain its parent row"
        );
    }

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

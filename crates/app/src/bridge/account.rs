//! `Account` singleton (docs/CONTRATO.md): Spotify sign-in state. A view of
//! `engine::account::Account`; sign-in runs in the system browser on the runtime.
//! By default (L1) there is ONE login, librespot's: it starts the audio engine (the Connect
//! device "Velo") and the library is read through that session. With a personal Spotify app
//! the Web API keeps its own PKCE login and `enableAudio()` is the second, audio-only login.

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(bool, signed_in, READ, NOTIFY)]
        #[qproperty(QString, display_name, READ, NOTIFY)]
        #[qproperty(QUrl, avatar_url, READ, NOTIFY)]
        #[qproperty(bool, is_premium, READ, NOTIFY)]
        #[qproperty(QString, client_id, READ, NOTIFY)]
        #[qproperty(bool, busy, READ, NOTIFY)]
        #[qproperty(QString, app_warning, READ, NOTIFY)]
        #[qproperty(bool, audio_ready, READ, NOTIFY)]
        #[qproperty(bool, audio_busy, READ, NOTIFY)]
        type Account = super::AccountRust;

        #[qsignal]
        fn sign_in_failed(self: Pin<&mut Account>, reason: &QString);

        #[qsignal]
        fn audio_failed(self: Pin<&mut Account>, reason: &QString);

        #[qinvokable]
        fn sign_in(self: &Account);

        #[qinvokable]
        fn cancel_sign_in(self: &Account);

        #[qinvokable]
        fn sign_out(self: &Account);

        #[qinvokable]
        fn set_client_id(self: &Account, id: &QString);

        #[qinvokable]
        fn authorize_client(self: &Account, id: &QString);

        #[qinvokable]
        fn use_spotify_account(self: &Account);

        #[qinvokable]
        fn enable_audio(self: &Account);
    }

    impl cxx_qt::Threading for Account {}
    impl cxx_qt::Initialize for Account {}
}

use core::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QUrl};

use crate::engine::account::AccountView;
use crate::engine::audio::AudioView;
use crate::engine::core;

pub struct AccountRust {
    signed_in: bool,
    display_name: QString,
    avatar_url: QUrl,
    is_premium: bool,
    client_id: QString,
    busy: bool,
    app_warning: QString,
    audio_ready: bool,
    audio_busy: bool,
}

impl Default for AccountRust {
    fn default() -> Self {
        let v = core().env.account.snapshot();
        let audio = *core().env.audio.view.borrow();
        Self {
            audio_ready: audio.ready,
            audio_busy: audio.busy,
            signed_in: v.signed_in,
            display_name: QString::from(v.display_name.as_str()),
            avatar_url: QUrl::from(v.avatar_url.as_str()),
            is_premium: v.is_premium,
            client_id: QString::from(v.client_id.as_str()),
            busy: v.busy,
            app_warning: QString::from(v.app_warning.as_str()),
        }
    }
}

impl cxx_qt::Initialize for ffi::Account {
    fn initialize(self: Pin<&mut Self>) {
        let env = core().env.clone();

        // State: every change of the engine's view is applied on the Qt thread.
        let thread = self.qt_thread();
        let mut view = env.account.view.subscribe();
        env.rt.spawn(async move {
            loop {
                let v = view.borrow_and_update().clone();
                if thread.queue(move |q| q.apply(&v)).is_err() {
                    break;
                }
                if view.changed().await.is_err() {
                    break;
                }
            }
        });

        // Velo's own audio engine: state, then failures (a signal).
        let thread = self.qt_thread();
        let mut audio = env.audio.view.subscribe();
        env.rt.spawn(async move {
            loop {
                let v = *audio.borrow_and_update();
                if thread.queue(move |q| q.apply_audio(v)).is_err() {
                    break;
                }
                if audio.changed().await.is_err() {
                    break;
                }
            }
        });
        let thread = self.qt_thread();
        let mut audio_failures = env.audio.failures.subscribe();
        env.rt.spawn(async move {
            loop {
                match audio_failures.recv().await {
                    Ok(reason) => {
                        let reason = QString::from(reason.as_str());
                        if thread.queue(move |q| q.audio_failed(&reason)).is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        // Sign-in failures are a signal, not state.
        let thread = self.qt_thread();
        let mut failures = env.account.failures.subscribe();
        env.rt.spawn(async move {
            while let Ok(reason) = failures.recv().await {
                let reason = QString::from(reason.as_str());
                if thread.queue(move |q| q.sign_in_failed(&reason)).is_err() {
                    break;
                }
            }
        });
    }
}

impl ffi::Account {
    fn apply(mut self: Pin<&mut Self>, v: &AccountView) {
        macro_rules! set {
            ($field:ident, $notify:ident, $value:expr) => {{
                let value = $value;
                if self.$field != value {
                    self.as_mut().rust_mut().$field = value;
                    self.as_mut().$notify();
                }
            }};
        }
        set!(signed_in, signed_in_changed, v.signed_in);
        set!(
            display_name,
            display_name_changed,
            QString::from(v.display_name.as_str())
        );
        set!(
            avatar_url,
            avatar_url_changed,
            QUrl::from(v.avatar_url.as_str())
        );
        set!(is_premium, is_premium_changed, v.is_premium);
        set!(
            client_id,
            client_id_changed,
            QString::from(v.client_id.as_str())
        );
        set!(busy, busy_changed, v.busy);
        set!(
            app_warning,
            app_warning_changed,
            QString::from(v.app_warning.as_str())
        );
    }

    fn apply_audio(mut self: Pin<&mut Self>, v: AudioView) {
        set_ro!(self, audio_ready, audio_ready_changed, v.ready);
        set_ro!(self, audio_busy, audio_busy_changed, v.busy);
    }

    /// Opens the browser once for the audio login, then starts the Connect device "Velo".
    pub fn enable_audio(&self) {
        core().env.audio.enable();
    }

    pub fn sign_in(&self) {
        core().env.account.sign_in();
    }

    pub fn cancel_sign_in(&self) {
        core().env.account.cancel_sign_in();
    }

    /// Signs out of the Web API and forgets the audio login too ("Velo" leaves Connect).
    pub fn sign_out(&self) {
        core().env.account.sign_out();
        core().env.audio.sign_out();
    }

    pub fn set_client_id(&self, id: &QString) {
        core().env.account.set_client_id(id.to_string());
    }

    pub fn authorize_client(&self, id: &QString) {
        core().env.account.authorize_client(id.to_string());
    }

    /// The default sign-in: the Spotify account through librespot (one login, audio + library).
    pub fn use_spotify_account(&self) {
        core().env.account.use_spotify_account();
    }
}

// SPDX-License-Identifier: GPL-3.0-or-later
//
// Making an account and getting back into one: registration and its emailed
// code, checking a login is free, restoring a forgotten password, and signing
// in through another service.
//
// Every request here is form-encoded and carries no token, as `auth/signIn`
// does. Each step answers with a code the caller words: the service's own
// numbers for a taken login, a bad code, an expired one.

use serde::Deserialize;

use crate::client::Client;
use crate::error::Result;
use crate::models::{Profile, ProfileToken};

/// What a step of registration or restoring answers with.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AuthStep {
    /// The service's code: 0 on success, the step's own numbers otherwise —
    /// see each method.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub code: i32,
    /// Ties the next step to this one: sent back with the emailed code.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub hash: String,
    /// When the emailed code stops working, in seconds since the epoch.
    #[serde(
        alias = "timestamp_expires",
        deserialize_with = "crate::serde_ext::nullable"
    )]
    pub code_timestamp_expires: i64,
    /// Logins the service offers instead of one that is taken.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub suggested_logins: Vec<String>,
    /// Signed in, when the step finished the job.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub profile: Option<Profile>,
    #[serde(
        alias = "profileToken",
        deserialize_with = "crate::serde_ext::nullable"
    )]
    pub profile_token: Option<ProfileToken>,
    /// Only for checking a login.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub available: bool,
}

/// A sign-in by QR code, as the service answers each step of it.
///
/// Since 11.0 a new device can be signed in from a phone that already is:
/// the device shows a code, the phone scans it, the person types the PIN the
/// device shows, and the device — asking all along — is handed the session.
/// The way in for accounts made through Google, VK, Telegram or Yandex, whose
/// tokens only the official app can get.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct QrSignIn {
    /// The service's code: 0, or why the step was refused — see
    /// [`QrStatus`] and the method.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub code: i32,
    /// Names the sign-in: what the QR code carries. 32 letters and digits.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub key: String,
    /// Proves to the service that the one asking is the device that started
    /// it. Never shown.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub secret: String,
    /// What the person types on the phone.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub pin: String,
    /// Seconds until the code stops working.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub expires_in: i64,
    /// Seconds to wait between asks.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub poll_interval: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    status: i32,
    /// Where the phone is, once it scanned.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub device: String,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub country_code: String,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub attempts_left: Option<i32>,
    /// Signed in, once confirmed.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub profile: Option<Profile>,
    #[serde(
        alias = "profileToken",
        deserialize_with = "crate::serde_ext::nullable"
    )]
    pub profile_token: Option<ProfileToken>,
}

/// Where a QR sign-in stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QrStatus {
    /// Shown, not yet scanned.
    Waiting,
    /// Scanned; the person is typing the PIN.
    Scanned,
    /// Confirmed on the phone: the session is in the answer.
    Confirmed,
    /// Turned down on the phone.
    Declined,
    /// The code ran out; a new one is needed.
    Expired,
    /// The phone got the PIN wrong too often.
    TooManyAttempts,
    /// Asked too often.
    TooManyRequests,
    /// A code the client does not know.
    Other(i32),
}

impl QrSignIn {
    /// What the QR code has to say for the official app to take it.
    #[must_use]
    pub fn link(&self) -> String {
        format!("anixart://sign-in/qr?key={}", self.key)
    }

    /// Where the sign-in stands. The service's body `code` names the ways
    /// it can end — 2 expired, 3 declined, 4 too many PIN attempts, 6 too
    /// many requests — and `status` the steps of one going well.
    #[must_use]
    pub fn status(&self) -> QrStatus {
        match (self.code, self.status) {
            (0, 0) => QrStatus::Waiting,
            (0, 1) => QrStatus::Scanned,
            (0, 2) => QrStatus::Confirmed,
            (2, _) => QrStatus::Expired,
            (3, _) => QrStatus::Declined,
            (4, _) => QrStatus::TooManyAttempts,
            (6, _) => QrStatus::TooManyRequests,
            (code, _) => QrStatus::Other(code),
        }
    }
}

/// A service an account can sign in with. Its token comes from that
/// service's own sign-in, made for the official Android app; this client
/// has no way to get one, and offers these only so the protocol is whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Google,
    Vk,
    Telegram,
    Yandex,
}

impl Provider {
    fn path(self) -> &'static str {
        match self {
            Self::Google => "auth/google",
            Self::Vk => "auth/vk",
            Self::Telegram => "auth/telegram",
            Self::Yandex => "auth/yandex",
        }
    }

    /// The form field the provider's token goes in.
    fn field(self) -> &'static str {
        match self {
            Self::Google => "googleIdToken",
            Self::Vk => "vkAccessToken",
            Self::Telegram => "telegramIdToken",
            Self::Yandex => "yandexAccessToken",
        }
    }
}

impl Client {
    /// Whether a login is free, with the service's suggestions when not.
    ///
    /// `POST auth/checkLogin`
    pub async fn check_login(&self, login: &str) -> Result<AuthStep> {
        self.auth_step("auth/checkLogin", vec![("login", login.to_owned())])
            .await
    }

    /// Starts registering: the service emails a code. Codes: 2 a bad login,
    /// 3 a bad email, 4 a bad password, 5 a taken login, 6 a taken email,
    /// 7 a code already sent, 8 one that cannot be, 9 an email service not
    /// accepted, 10 too many registrations.
    ///
    /// `POST auth/signUp`
    pub async fn sign_up(&self, login: &str, email: &str, password: &str) -> Result<AuthStep> {
        self.auth_step(
            "auth/signUp",
            vec![
                ("login", login.to_owned()),
                ("email", email.to_owned()),
                ("password", password.to_owned()),
            ],
        )
        .await
    }

    /// Finishes registering with the emailed code, which signs in. Codes as
    /// for signing up, and 7 a wrong code, 8 an expired one, 9 a bad hash.
    ///
    /// `POST auth/verify`
    pub async fn sign_up_verify(
        &self,
        login: &str,
        email: &str,
        password: &str,
        hash: &str,
        code: &str,
    ) -> Result<AuthStep> {
        self.auth_step(
            "auth/verify",
            vec![
                ("login", login.to_owned()),
                ("email", email.to_owned()),
                ("password", password.to_owned()),
                ("hash", hash.to_owned()),
                ("code", code.to_owned()),
            ],
        )
        .await
    }

    /// Sends the registration code again.
    ///
    /// `POST auth/resend`
    pub async fn sign_up_resend(
        &self,
        login: &str,
        email: &str,
        password: &str,
        hash: &str,
    ) -> Result<AuthStep> {
        self.auth_step(
            "auth/resend",
            vec![
                ("login", login.to_owned()),
                ("email", email.to_owned()),
                ("password", password.to_owned()),
                ("hash", hash.to_owned()),
            ],
        )
        .await
    }

    /// Starts restoring a forgotten password: `data` is the account's login
    /// or email, and a code is emailed. 2 no such account.
    ///
    /// `POST auth/restore`
    pub async fn restore(&self, data: &str) -> Result<AuthStep> {
        self.auth_step("auth/restore", vec![("data", data.to_owned())])
            .await
    }

    /// Sends the restore code again.
    ///
    /// `POST auth/restore/resend`
    pub async fn restore_resend(&self, data: &str, password: &str, hash: &str) -> Result<AuthStep> {
        self.auth_step(
            "auth/restore/resend",
            vec![
                ("data", data.to_owned()),
                ("password", password.to_owned()),
                ("hash", hash.to_owned()),
            ],
        )
        .await
    }

    /// Sets the new password with the emailed code, which signs in. 3 a bad
    /// password, 4 a wrong code, 5 an expired one.
    ///
    /// `POST auth/restore/verify`
    pub async fn restore_verify(
        &self,
        data: &str,
        password: &str,
        hash: &str,
        code: &str,
    ) -> Result<AuthStep> {
        self.auth_step(
            "auth/restore/verify",
            vec![
                ("data", data.to_owned()),
                ("password", password.to_owned()),
                ("hash", hash.to_owned()),
                ("code", code.to_owned()),
            ],
        )
        .await
    }

    /// Signs in with another service's token.
    ///
    /// `POST auth/{google,vk,telegram,yandex}`
    pub async fn sign_in_with(&self, provider: Provider, token: &str) -> Result<AuthStep> {
        self.auth_step(provider.path(), vec![(provider.field(), token.to_owned())])
            .await
    }

    /// Registers with another service's token, naming the new account.
    ///
    /// The same paths as [`Self::sign_in_with`], with a login and email.
    pub async fn sign_up_with(
        &self,
        provider: Provider,
        token: &str,
        login: &str,
        email: &str,
    ) -> Result<AuthStep> {
        self.auth_step(
            provider.path(),
            vec![
                ("login", login.to_owned()),
                ("email", email.to_owned()),
                (provider.field(), token.to_owned()),
            ],
        )
        .await
    }

    /// Starts a QR sign-in for this device, named `device` on the phone that
    /// confirms it.
    ///
    /// `POST auth/qr/create`
    pub async fn qr_sign_in_create(&self, device: &str) -> Result<QrSignIn> {
        self.send_any_code(self.post("auth/qr/create").form([("device", device)]))
            .await
    }

    /// Asks where a QR sign-in stands; once confirmed the answer carries the
    /// profile and its token. Ask no more often than
    /// [`QrSignIn::poll_interval`].
    ///
    /// `POST auth/qr/status`
    pub async fn qr_sign_in_status(&self, key: &str, secret: &str) -> Result<QrSignIn> {
        self.send_any_code(
            self.post("auth/qr/status")
                .form([("key", key), ("secret", secret)]),
        )
        .await
    }

    /// Exchanges the account's session for a push-messaging one. Only of use
    /// to a client registered with that messaging service.
    ///
    /// `POST auth/firebase`
    pub async fn firebase(&self) -> Result<serde_json::Value> {
        self.require_token()?;
        self.send(self.post("auth/firebase").with_token()).await
    }

    /// Every step answers success or a code of its own, which is handed back
    /// rather than turned into an error: the caller is the one that says what
    /// a taken login means.
    async fn auth_step(
        &self,
        path: &'static str,
        fields: Vec<(&'static str, String)>,
    ) -> Result<AuthStep> {
        self.send_any_code(self.post(path).form(fields)).await
    }
}

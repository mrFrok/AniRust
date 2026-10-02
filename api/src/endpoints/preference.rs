// SPDX-License-Identifier: GPL-3.0-or-later
//
// The account's own settings: what it looks like, who can see what, and how
// it is reached.

use serde::Deserialize;
use serde_json::json;

use crate::client::{Ack, Client, Upload};
use crate::error::Result;

/// The account's settings, as `profile/preference/my` gives them.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Preferences {
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub status: String,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub avatar: String,
    /// The account's email with most of it hidden, to say which it is.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub email_hint: String,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub vk_page: String,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub tg_page: String,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub inst_page: String,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub tt_page: String,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub discord_page: String,
    /// Who may see what, each as a [`Privacy`] number.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub privacy_counts: i32,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub privacy_stats: i32,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub privacy_social: i32,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub privacy_friend_requests: i32,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub is_incognito: bool,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub pinned_section_id: Option<i32>,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub selected_theme_id: Option<i64>,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub episode_channel_widgets_hidden: bool,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub is_login_changed: bool,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub is_change_login_banned: bool,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub is_change_avatar_banned: bool,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub is_google_bound: bool,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub is_vk_bound: bool,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub is_telegram_bound: bool,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub is_yandex_bound: bool,
}

/// Which privacy setting [`Client::privacy_edit`] changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Privacy {
    /// The counts of the account's lists.
    Counts,
    /// Its statistics.
    Stats,
    /// Its links elsewhere.
    Social,
    /// Who may send it friend requests.
    FriendRequests,
}

impl Privacy {
    fn path(self) -> &'static str {
        match self {
            Self::Counts => "profile/preference/privacy/counts/edit",
            Self::Stats => "profile/preference/privacy/stats/edit",
            Self::Social => "profile/preference/privacy/social/edit",
            Self::FriendRequests => "profile/preference/privacy/friendRequests/edit",
        }
    }
}

/// When the login may next be changed.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LoginChangeInfo {
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub is_change_available: bool,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub last_change_at: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub next_change_available_at: i64,
}

/// What a settings change answers when it can say more than yes: a code, the
/// hash an emailed confirmation is tied to, logins suggested instead, or a
/// new session token after a password change.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SettingStep {
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub code: i32,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub hash: String,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub timestamp_expires: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub suggested_logins: Vec<String>,
    /// After a password change, the session to carry on with: the old one
    /// is ended by it.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub token: String,
}

/// A service whose account can be bound to this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    Google,
    Vk,
    Telegram,
    Yandex,
}

impl Binding {
    fn segment(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Vk => "vk",
            Self::Telegram => "telegram",
            Self::Yandex => "yandex",
        }
    }

    fn field(self) -> &'static str {
        match self {
            Self::Google | Self::Telegram => "idToken",
            Self::Vk | Self::Yandex => "accessToken",
        }
    }
}

impl Client {
    /// Replaces the account's picture.
    ///
    /// `POST profile/preference/avatar/edit`, multipart. The app sends the
    /// file as a part named `image` under its own file name, and an empty
    /// text part named `name` beside it; both are sent here the same way.
    /// `mime` is the picture's type, e.g. `image/png` — the app sends
    /// `image/*` and lets the server look, but a real type costs nothing.
    pub async fn avatar_edit(
        &self,
        file_name: &str,
        mime: &'static str,
        bytes: Vec<u8>,
    ) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.post("profile/preference/avatar/edit")
                    .with_token()
                    .upload(Upload {
                        part: "image",
                        file_name: file_name.to_owned(),
                        mime,
                        bytes,
                        fields: vec![("name", String::new())],
                    }),
            )
            .await?;
        Ok(())
    }

    /// Removes the account's picture.
    ///
    /// `GET profile/preference/avatar/delete`
    pub async fn avatar_delete(&self) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(self.get("profile/preference/avatar/delete").with_token())
            .await?;
        Ok(())
    }

    /// The account's settings.
    ///
    /// `GET profile/preference/my`
    pub async fn preferences(&self) -> Result<Preferences> {
        self.require_token()?;
        self.send(self.get("profile/preference/my").with_token())
            .await
    }

    /// The line the account writes about itself.
    ///
    /// `POST profile/preference/status/edit`
    pub async fn status_edit(&self, status: &str) -> Result<()> {
        self.preference_json(
            "profile/preference/status/edit",
            json!({ "status": status }),
        )
        .await
    }

    /// The account's links elsewhere.
    ///
    /// `GET profile/preference/social`
    pub async fn my_socials(&self) -> Result<crate::endpoints::Socials> {
        self.require_token()?;
        self.send(self.get("profile/preference/social").with_token())
            .await
    }

    /// `POST profile/preference/social/edit`. The request class declares
    /// camelCase fields without names of their own, so the service's
    /// snake_case strategy is what goes out.
    pub async fn socials_edit(&self, socials: &crate::endpoints::Socials) -> Result<()> {
        self.preference_json(
            "profile/preference/social/edit",
            json!({
                "vk_page": socials.vk_page,
                "tg_page": socials.tg_page,
                "inst_page": socials.inst_page,
                "tt_page": socials.tt_page,
                "discord_page": socials.discord_page,
            }),
        )
        .await
    }

    /// Who may see one part of the account. `permission` is the service's
    /// number: 0 everyone, 1 friends only, 2 only the account itself. For
    /// friend requests there are two: 0 anyone may send one, 1 nobody.
    ///
    /// `POST profile/preference/privacy/{counts,stats,social,friendRequests}/edit`
    pub async fn privacy_edit(&self, what: Privacy, permission: i32) -> Result<()> {
        self.preference_json(what.path(), json!({ "permission": permission }))
            .await
    }

    /// Turns being invisible online over — the request carries no value.
    ///
    /// `GET profile/preference/privacy/incognito/edit`
    pub async fn incognito_switch(&self) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.get("profile/preference/privacy/incognito/edit")
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// Which section of the profile is pinned at its top.
    ///
    /// `POST profile/preference/section/edit`
    pub async fn pinned_section_edit(&self, id: i32) -> Result<()> {
        self.preference_json("profile/preference/section/edit", json!({ "id": id }))
            .await
    }

    /// The profile's decoration theme.
    ///
    /// `POST profile/preference/themes/edit`
    pub async fn theme_edit(&self, id: i64) -> Result<()> {
        self.preference_json("profile/preference/themes/edit", json!({ "id": id }))
            .await
    }

    /// Hides the voice-over channels' widgets on every episode list.
    ///
    /// `POST profile/preference/episode-widget/edit?hidden=`
    pub async fn episode_widgets_hidden(&self, hidden: bool) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.post("profile/preference/episode-widget/edit")
                    .query("hidden", hidden)
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// When the login may next be changed.
    ///
    /// `POST profile/preference/login/info`
    pub async fn login_change_info(&self) -> Result<LoginChangeInfo> {
        self.require_token()?;
        self.send(self.post("profile/preference/login/info").with_token())
            .await
    }

    /// Changes the login. 2 a bad login, 3 taken (with suggestions), 4 too
    /// soon since the last change.
    ///
    /// `POST profile/preference/login/change?login=`
    pub async fn login_change(&self, login: &str) -> Result<SettingStep> {
        self.require_token()?;
        self.send_any_code(
            self.post("profile/preference/login/change")
                .query("login", login)
                .with_token(),
        )
        .await
    }

    /// Changes the password. The answer carries the token to carry on with:
    /// a password change ends the old session. 2 a bad new password, 3 a
    /// wrong current one.
    ///
    /// `POST profile/preference/password/change`
    pub async fn password_change(&self, current: &str, new: &str) -> Result<SettingStep> {
        self.require_token()?;
        self.send_any_code(
            self.post("profile/preference/password/change")
                .with_token()
                .form([("current", current), ("new", new)]),
        )
        .await
    }

    /// Starts changing the email: the new one is sent a code. 2 a wrong
    /// password, 3 a wrong current email, 4 a bad new one, 5 taken.
    ///
    /// `POST profile/preference/email/change`
    pub async fn email_change(
        &self,
        current_email: &str,
        current_password: &str,
        new_email: &str,
    ) -> Result<SettingStep> {
        self.require_token()?;
        self.send_any_code(
            self.post("profile/preference/email/change")
                .with_token()
                .form([
                    ("current_email", current_email),
                    ("current_password", current_password),
                    ("new_email", new_email),
                ]),
        )
        .await
    }

    /// Sends the email-change code again.
    ///
    /// `POST profile/preference/email/resend`
    pub async fn email_change_resend(
        &self,
        current_email: &str,
        current_password: &str,
        new_email: &str,
        hash: &str,
    ) -> Result<SettingStep> {
        self.require_token()?;
        self.send_any_code(
            self.post("profile/preference/email/resend")
                .with_token()
                .form([
                    ("new_email", new_email),
                    ("current_email", current_email),
                    ("current_password", current_password),
                    ("hash", hash),
                ]),
        )
        .await
    }

    /// Finishes changing the email with the code sent to it.
    ///
    /// `GET profile/preference/email/verify?new_email=&code=&hash=`
    pub async fn email_change_verify(
        &self,
        new_email: &str,
        code: &str,
        hash: &str,
    ) -> Result<SettingStep> {
        self.require_token()?;
        self.send_any_code(
            self.get("profile/preference/email/verify")
                .query("new_email", new_email)
                .query("code", code)
                .query("hash", hash)
                .with_token(),
        )
        .await
    }

    /// Binds another service's account to this one, with that service's
    /// token — which, as for signing in with it, only the official app can
    /// get.
    ///
    /// `POST profile/preference/{google,vk,telegram,yandex}/bind`
    pub async fn bind(&self, to: Binding, token: &str) -> Result<()> {
        self.require_token()?;
        let path = format!("profile/preference/{}/bind", to.segment());
        let _: Ack = self
            .send(self.post(path).with_token().form([(to.field(), token)]))
            .await?;
        Ok(())
    }

    /// `POST profile/preference/{google,vk,telegram,yandex}/unbind`
    pub async fn unbind(&self, from: Binding) -> Result<()> {
        self.require_token()?;
        let path = format!("profile/preference/{}/unbind", from.segment());
        let _: Ack = self.send(self.post(path).with_token()).await?;
        Ok(())
    }

    async fn preference_json(&self, path: &'static str, body: serde_json::Value) -> Result<()> {
        self.require_token()?;
        let _: Ack = self.send(self.post(path).with_token().json(body)).await?;
        Ok(())
    }
}

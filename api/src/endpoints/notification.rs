// SPDX-License-Identifier: GPL-3.0-or-later
//
// Notifications, and what the account wants to be notified about.

use serde::Deserialize;
use serde_json::json;

use crate::client::{Ack, Client};
use crate::error::Result;
use crate::models::{
    Dubber, Notification, NotificationDelete, NotificationKind, NotificationPreferences,
    NotificationSwitch, Page, Release,
};

use super::PageablePayload;

#[derive(Deserialize)]
struct CountPayload {
    #[serde(default)]
    count: i64,
}

#[derive(Deserialize)]
struct ReleaseDubberPreferences {
    #[serde(default)]
    profile_release_type_notification_preferences: Vec<DubberPreference>,
}

#[derive(Deserialize)]
struct DubberPreference {
    #[serde(rename = "type", default)]
    dubber: Dubber,
}

impl Client {
    /// How many notifications have not been seen — what the bell's badge
    /// counts.
    ///
    /// `GET notification/count`
    pub async fn notification_count(&self) -> Result<i64> {
        self.require_token()?;
        let payload: CountPayload = self
            .send(self.get("notification/count").with_token())
            .await?;
        Ok(payload.count)
    }

    /// One list of notifications, newest first. 0-based.
    ///
    /// `GET notification/{kind}/{page}`
    pub async fn notifications(
        &self,
        kind: NotificationKind,
        page: i32,
    ) -> Result<Page<Notification>> {
        self.require_token()?;
        let base = kind.list_path();
        let payload: PageablePayload<Notification> = self
            .send(self.get(format!("{base}/{page}")).with_token())
            .await?;
        Ok(payload.into())
    }

    /// Marks every notification seen.
    ///
    /// `GET notification/read`
    pub async fn notifications_read(&self) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(self.get("notification/read").with_token())
            .await?;
        Ok(())
    }

    /// Deletes every notification.
    ///
    /// `GET notification/delete/all`
    pub async fn notifications_delete_all(&self) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(self.get("notification/delete/all").with_token())
            .await?;
        Ok(())
    }

    /// Deletes one notification.
    ///
    /// `GET notification/{kind}/delete/{id}`
    pub async fn notification_delete(&self, kind: NotificationDelete, id: i64) -> Result<()> {
        self.require_token()?;
        let base = kind.path();
        let _: Ack = self
            .send(self.get(format!("{base}/{id}")).with_token())
            .await?;
        Ok(())
    }

    // ---- preferences ------------------------------------------------------

    /// What the account is notified about.
    ///
    /// `GET profile/preference/notification/my`
    pub async fn notification_preferences(&self) -> Result<NotificationPreferences> {
        self.require_token()?;
        self.send(self.get("profile/preference/notification/my").with_token())
            .await
    }

    /// Turns one switch over. The request carries no value; the server flips
    /// what it has, so the caller should read the preferences back rather
    /// than assume.
    ///
    /// `GET profile/preference/notification/{switch}/edit`
    pub async fn notification_switch(&self, switch: NotificationSwitch) -> Result<()> {
        self.require_token()?;
        let _: Ack = self.send(self.get(switch.path()).with_token()).await?;
        Ok(())
    }

    /// The releases picked by hand for episode notifications. 0-based.
    ///
    /// `GET profile/preference/notification/release/all/{page}`
    pub async fn notification_releases(&self, page: i32) -> Result<Page<Release>> {
        self.require_token()?;
        let payload: PageablePayload<Release> = self
            .send(
                self.get(format!(
                    "profile/preference/notification/release/all/{page}"
                ))
                .with_token(),
            )
            .await?;
        Ok(payload.into())
    }

    /// Which voice-overs of one release the account is notified about.
    ///
    /// `GET profile/preference/notification/release/type/{release_id}`
    pub async fn notification_release_dubbers(&self, release_id: i64) -> Result<Vec<Dubber>> {
        self.require_token()?;
        let payload: ReleaseDubberPreferences = self
            .send(
                self.get(format!(
                    "profile/preference/notification/release/type/{release_id}"
                ))
                .with_token(),
            )
            .await?;
        Ok(payload
            .profile_release_type_notification_preferences
            .into_iter()
            .map(|preference| preference.dubber)
            .collect())
    }

    /// Sets which voice-overs of one release the account is notified about.
    ///
    /// `POST profile/preference/notification/release/type/edit`
    pub async fn notification_release_dubbers_edit(
        &self,
        release_id: i64,
        dubber_ids: &[i64],
    ) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.post("profile/preference/notification/release/type/edit")
                    .with_token()
                    .json(json!({
                        "release_id": release_id,
                        "profile_release_type_notification_preferences": dubber_ids,
                    })),
            )
            .await?;
        Ok(())
    }

    /// Sets which of the account's lists, by status number, bring episode
    /// notifications.
    ///
    /// `POST profile/preference/notification/status/edit`. The field name
    /// is the request class's, written by the same naming strategy as the
    /// rest — it has no annotation of its own.
    pub async fn notification_statuses_edit(&self, statuses: &[i32]) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.post("profile/preference/notification/status/edit")
                    .with_token()
                    .json(json!({ "profile_status_notification_preferences": statuses })),
            )
            .await?;
        Ok(())
    }

    /// Sets which voice-overs, anywhere, bring episode notifications.
    ///
    /// `POST profile/preference/notification/type/edit`
    pub async fn notification_dubbers_edit(&self, dubber_ids: &[i64]) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.post("profile/preference/notification/type/edit")
                    .with_token()
                    .json(json!({ "profile_type_notification_preferences": dubber_ids })),
            )
            .await?;
        Ok(())
    }
}

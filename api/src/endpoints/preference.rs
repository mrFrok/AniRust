// SPDX-License-Identifier: GPL-3.0-or-later
//
// The account's own settings: what it looks like, who can see what, and how
// it is reached.

use crate::client::{Ack, Client, Upload};
use crate::error::Result;

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
}

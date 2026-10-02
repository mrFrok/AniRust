// SPDX-License-Identifier: GPL-3.0-or-later

//! Moving the account's lists in and out: in from a Shikimori export, out
//! as a table any spreadsheet opens.
//!
//! The import sends Shikimori's own ids and names the service they came
//! from; matching them to releases is the server's work. It may be done once
//! a month, and the server is asked before anything is sent. The export
//! asks for each list on its own, so every row can say which list it is in.

use std::fmt::Write as _;

use serde::Deserialize;
use slint::ComponentHandle;

use anirust_api::{Bookmarks, Client, ProfileList, Release};

use crate::{MainWindow, tasks};

/// The name the service knows the Shikimori importer by.
const SHIKIMORI: &str = "Shikimori";

/// One entry of a Shikimori list export.
#[derive(Deserialize)]
struct Entry {
    target_id: i64,
    status: String,
}

/// Reads a Shikimori JSON export into the five lists. Rewatching goes with
/// watching: there is no list for it here, and it is being watched.
fn parse_shikimori(text: &[u8]) -> Option<Bookmarks> {
    let entries: Vec<Entry> = serde_json::from_slice(text).ok()?;
    let mut lists = Bookmarks::default();
    for entry in entries {
        let list = match entry.status.as_str() {
            "watching" | "rewatching" => &mut lists.watching,
            "planned" => &mut lists.plans,
            "completed" => &mut lists.completed,
            "on_hold" => &mut lists.hold_on,
            "dropped" => &mut lists.dropped,
            _ => continue,
        };
        if entry.target_id > 0 && !list.contains(&entry.target_id) {
            list.push(entry.target_id);
        }
    }
    Some(lists)
}

fn count(lists: &Bookmarks) -> usize {
    lists.watching.len()
        + lists.plans.len()
        + lists.completed.len()
        + lists.hold_on.len()
        + lists.dropped.len()
}

/// Asks for a Shikimori export and moves its lists in.
pub fn import(window: &MainWindow, client: &Client) {
    let weak = window.as_weak();
    let api = client.clone();
    let picked = slint::spawn_local(async move {
        let Some(file) = rfd::AsyncFileDialog::new()
            .add_filter("JSON", &["json"])
            .pick_file()
            .await
        else {
            return;
        };
        let bytes = file.read().await;
        let Some(window) = weak.upgrade() else { return };
        let lists = match parse_shikimori(&bytes) {
            Some(lists) if count(&lists) > 0 => lists,
            _ => {
                window.set_settings_message("import-unreadable".into());
                return;
            }
        };
        window.set_settings_busy(true);
        window.set_settings_message("".into());
        let weak = window.as_weak();
        tasks::spawn(
            async move {
                // Once a month: asked first, so a refusal costs nothing.
                if api.import_status().await? != 0 {
                    return Ok(false);
                }
                api.import_bookmarks(SHIKIMORI, &lists).await.map(|()| true)
            },
            move |result: anirust_api::Result<bool>| {
                let Some(window) = weak.upgrade() else { return };
                window.set_settings_busy(false);
                let message = match result {
                    Ok(true) => "imported",
                    Ok(false) => "import-too-soon",
                    Err(error) => {
                        tracing::warn!(%error, "the bookmarks were not imported");
                        "failed"
                    }
                };
                window.set_settings_message(message.into());
            },
        );
    });
    if let Err(error) = picked {
        tracing::warn!(%error, "the file chooser could not be opened");
    }
}

/// Writes the account's lists to a CSV file of the viewer's choosing.
pub fn export(window: &MainWindow, client: &Client) {
    window.set_settings_busy(true);
    window.set_settings_message("".into());
    let weak = window.as_weak();
    let api = client.clone();
    let names = list_names(window.get_lang() == "ru");
    tasks::spawn(
        async move {
            let mut lists = Vec::new();
            for list in ProfileList::ALL {
                lists.push((list, api.export_bookmarks(&[list.raw()], 0).await?));
            }
            Ok::<_, anirust_api::Error>(lists)
        },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            window.set_settings_busy(false);
            let lists = match result {
                Ok(lists) => lists,
                Err(error) => {
                    tracing::warn!(%error, "the bookmarks were not exported");
                    window.set_settings_message("failed".into());
                    return;
                }
            };
            let table = csv(&lists, &names);
            let weak = window.as_weak();
            let saved = slint::spawn_local(async move {
                let Some(file) = rfd::AsyncFileDialog::new()
                    .set_file_name("anixart-bookmarks.csv")
                    .add_filter("CSV", &["csv"])
                    .save_file()
                    .await
                else {
                    return;
                };
                let written = file.write(table.as_bytes()).await;
                let Some(window) = weak.upgrade() else { return };
                match written {
                    Ok(()) => window.set_settings_message("exported".into()),
                    Err(error) => {
                        tracing::warn!(%error, "the export file was not written");
                        window.set_settings_message("export-unwritable".into());
                    }
                }
            });
            if let Err(error) = saved {
                tracing::warn!(%error, "the file chooser could not be opened");
            }
        },
    );
}

fn list_names(russian: bool) -> [&'static str; 5] {
    if russian {
        ["Смотрю", "В планах", "Просмотрено", "Отложено", "Брошено"]
    } else {
        ["Watching", "Planned", "Watched", "On hold", "Dropped"]
    }
}

/// The lists as one table, a release to a row.
fn csv(lists: &[(ProfileList, Vec<Release>)], names: &[&str; 5]) -> String {
    let mut out = String::from("id,list,title,original_title,year,episodes\r\n");
    for (list, releases) in lists {
        let name = ProfileList::ALL
            .iter()
            .position(|l| l == list)
            .and_then(|at| names.get(at))
            .copied()
            .unwrap_or_default();
        for release in releases {
            let _ = write!(
                out,
                "{},{},{},{},{},{}\r\n",
                release.id,
                field(name),
                field(release.title()),
                field(&release.title_original),
                field(&release.year),
                release.episodes_total,
            );
        }
    }
    out
}

/// One CSV field, quoted when it has to be.
fn field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shikimori_export_lands_in_the_five_lists() {
        let text = br#"[
            {"target_title":"A","target_id":1,"status":"watching"},
            {"target_title":"B","target_id":2,"status":"rewatching"},
            {"target_title":"C","target_id":3,"status":"planned"},
            {"target_title":"D","target_id":4,"status":"completed"},
            {"target_title":"E","target_id":5,"status":"on_hold"},
            {"target_title":"F","target_id":6,"status":"dropped"},
            {"target_title":"G","target_id":7,"status":"something new"}
        ]"#;
        let lists = parse_shikimori(text).unwrap();
        assert_eq!(lists.watching, [1, 2]);
        assert_eq!(lists.plans, [3]);
        assert_eq!(lists.completed, [4]);
        assert_eq!(lists.hold_on, [5]);
        assert_eq!(lists.dropped, [6]);
    }

    #[test]
    fn something_else_is_not_an_export() {
        assert!(parse_shikimori(b"{\"not\":\"a list\"}").is_none());
        assert!(parse_shikimori(b"not json").is_none());
    }

    #[test]
    fn fields_are_quoted_only_when_they_must_be() {
        assert_eq!(field("plain"), "plain");
        assert_eq!(field("a, b"), "\"a, b\"");
        assert_eq!(field("say \"hi\""), "\"say \"\"hi\"\"\"");
    }
}

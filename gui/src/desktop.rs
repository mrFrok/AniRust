// SPDX-License-Identifier: GPL-3.0-or-later

//! The program's name and picture where the desktop shows them.
//!
//! On Linux a window manager finds a window's icon through its application id:
//! the id names a `.desktop` file, and the file names the icon. A package
//! installs both; a program run straight from an unpacked archive has
//! neither, and shows in the task bar as a blank. So when the desktop does
//! not already know the program, the two files are written into the user's
//! own data directory — the place the freedesktop specification gives for
//! exactly this — pointing at wherever the program was started from.

/// The id the window carries, and the name of the files that describe it.
pub const APP_ID: &str = "io.github.mrfrok.AniRust";

#[cfg(all(unix, not(target_os = "macos")))]
const DESKTOP_ENTRY: &str = include_str!("../../packaging/linux/io.github.mrfrok.AniRust.desktop");
#[cfg(all(unix, not(target_os = "macos")))]
const ICON: &str = include_str!("../../packaging/icons/anirust.svg");

/// Tells the windowing system who this is. Must come before the window.
///
/// The id is held by the platform, so the platform is brought up here first —
/// the same one Slint would choose on its own, `SLINT_BACKEND` included.
/// Left to the window, it comes up after the id has nowhere to go.
pub fn identify() {
    if let Err(error) = slint::BackendSelector::new().select() {
        tracing::debug!(%error, "the platform was not brought up early");
    }
    if let Err(error) = slint::set_xdg_app_id(APP_ID) {
        tracing::debug!(%error, "the application id was not set");
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Err(error) = integrate() {
        tracing::debug!(%error, "the desktop entry was not written");
    }
}

/// Writes the desktop entry and icon into the user's data directory, unless
/// a package has put them in the system's.
#[cfg(all(unix, not(target_os = "macos")))]
fn integrate() -> std::io::Result<()> {
    let installed = ["/usr/share", "/usr/local/share"].iter().any(|root| {
        std::path::Path::new(root)
            .join("applications")
            .join(format!("{APP_ID}.desktop"))
            .exists()
    });
    if installed {
        return Ok(());
    }
    let Some(data) = dirs::data_dir() else {
        return Ok(());
    };
    let exe = std::env::current_exe()?;

    // The entry runs this very file, wherever it was unpacked to.
    let entry = DESKTOP_ENTRY.replace("Exec=anirust %u", &format!("Exec=\"{}\" %u", exe.display()));
    let entry_path = data.join("applications").join(format!("{APP_ID}.desktop"));
    let icon_path = data
        .join("icons/hicolor/scalable/apps")
        .join(format!("{APP_ID}.svg"));

    write_if_changed(&entry_path, &entry)?;
    write_if_changed(&icon_path, ICON)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn write_if_changed(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    if std::fs::read_to_string(path).is_ok_and(|now| now == contents) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)
}

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
    // Flatpak installs its own entry, under the app's id, and the sandbox
    // could not write the host's anyway.
    if installed || std::env::var_os("FLATPAK_ID").is_some() {
        return Ok(());
    }
    let Some(data) = dirs::data_dir() else {
        return Ok(());
    };
    // An AppImage runs from a mount that moves every start; the entry should
    // run the AppImage file itself, which the runtime names in $APPIMAGE. The
    // Linux archive starts the program through a launcher that picks the
    // right libmpv; the entry should go through that. Otherwise the entry
    // runs this very file, wherever it was unpacked to.
    let exe = ["APPIMAGE", "ANIRUST_LAUNCHER"]
        .iter()
        .filter_map(std::env::var_os)
        .map(std::path::PathBuf::from)
        .find(|path| path.is_file())
        .map_or_else(std::env::current_exe, Ok)?;

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

/// Opens a folder of ours in the desktop's file manager.
///
/// The path goes to the platform's opener as a single argument, never
/// through a shell. Only for folders this program made — the screenshots,
/// the subtitle fonts — never a path someone else supplied.
pub fn open_folder(path: &std::path::Path) {
    #[cfg(target_os = "linux")]
    let opened = std::process::Command::new("xdg-open").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let opened = std::process::Command::new("open").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let opened = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    let opened: std::io::Result<std::process::Child> = Err(std::io::Error::other(
        "no file manager opener on this platform",
    ));

    if let Err(error) = opened {
        tracing::warn!(%error, path = %path.display(), "the folder could not be opened");
    }
}

/// The refresh rate of the monitor the window is on, in frames a second.
///
/// mpv cannot see it through the render API, so without this it assumes
/// nothing — frame generation "at the screen's rate" falls back to 60 on a
/// 180 Hz monitor, and interpolation has no display to resample to. Asked of
/// winit, which knows it on every platform; `None` before the window exists
/// or where the system will not say.
pub fn refresh_rate(window: &slint::Window) -> Option<f64> {
    use slint::winit_030::WinitWindowAccessor;

    window
        .with_winit_window(|window| {
            window
                .current_monitor()
                .and_then(|monitor| monitor.refresh_rate_millihertz())
        })
        .flatten()
        .map(|millihertz| f64::from(millihertz) / 1000.0)
        .filter(|fps| *fps > 1.0)
}

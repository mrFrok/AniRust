// SPDX-License-Identifier: GPL-3.0-or-later

//! Updates from the project's releases on GitHub.
//!
//! The program asks GitHub for the latest release when it starts and once a
//! day after, unless that is switched off in the settings. A newer one is
//! offered, never installed unasked: the viewer may be in the middle of an
//! episode. Asked to, it fetches the file for the way this copy was
//! installed, checks it against the release key's signature, puts it in
//! place and starts the new version.
//!
//! How it can be put in place depends on how it was installed, and only some
//! ways are ours to change: the Windows installer's, an AppImage the user can
//! write over, the archive and the macOS bundle as install.sh lays them out.
//! Flatpak, a distribution's package and anything unpacked by hand are left
//! to whatever installed them; for those the program only says a release is
//! out.

use std::cell::{Cell, RefCell};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt;
use slint::ComponentHandle;

use crate::{MainWindow, preferences::Preferences, tasks};

const REPO: &str = "mrFrok/AniRust";

/// The public half of the release key (packaging/minisign.pub). The release
/// workflow signs every file it publishes with the secret half; a download
/// that does not verify against this is not installed.
const RELEASE_KEY: &str = "RWTsVgZgS0HPjqK3vDoEBG+K19n1nsoOVdObXpaP1T3KMXa0yG/lm9d5";

/// A release's version, `major.minor.patch`. Pre-releases are not offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.trim().trim_start_matches('v').split('.');
        let version = Self(
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
        );
        parts.next().is_none().then_some(version)
    }

    /// This program's own.
    #[must_use]
    pub fn current() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Self(0, 0, 0))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// A published release: its version, its page, and its files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub page: String,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

impl Release {
    fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|asset| asset.name == name)
    }
}

#[derive(serde::Deserialize)]
struct ApiRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(serde::Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
}

/// The latest release, as GitHub's API answers for it.
fn parse_release(json: &str) -> Result<Release, String> {
    let release: ApiRelease = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if release.draft || release.prerelease {
        return Err("the latest release is not a final one".to_owned());
    }
    let version = Version::parse(&release.tag_name)
        .ok_or_else(|| format!("{} is not a version", release.tag_name))?;
    Ok(Release {
        version,
        page: release.html_url,
        assets: release
            .assets
            .into_iter()
            .map(|asset| Asset {
                name: asset.name,
                url: asset.browser_download_url,
            })
            .collect(),
    })
}

/// Asks GitHub for the latest release.
pub async fn latest(http: reqwest::Client) -> Result<Release, String> {
    let body = http
        .get(format!(
            "https://api.github.com/repos/{REPO}/releases/latest"
        ))
        // GitHub's API turns away requests that do not say who they are.
        .header(
            reqwest::header::USER_AGENT,
            concat!("AniRust/", env!("CARGO_PKG_VERSION")),
        )
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    parse_release(&body)
}

/// How this copy of the program was installed, which decides whether and
/// how it updates itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Install {
    /// By the Windows installer: a newer installer runs over it.
    WindowsSetup,
    /// An AppImage in a folder the user can write to: replaced whole.
    AppImage(PathBuf),
    /// The Linux archive as install.sh unpacks it, in this folder.
    Archive(PathBuf),
    /// The macOS bundle, in a folder the user can write to.
    MacApp(PathBuf),
    /// Updated by whatever installed it.
    Elsewhere(Elsewhere),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Elsewhere {
    Flatpak,
    /// A distribution's package, or the PKGBUILD.
    PackageManager,
    /// Unpacked by hand — the zip, the archive, the disk image.
    ByHand,
}

impl Install {
    /// The release file this install updates from.
    #[must_use]
    pub fn asset_name(&self, version: Version) -> Option<String> {
        match self {
            Self::WindowsSetup => Some(format!("AniRust-{version}-setup.exe")),
            Self::AppImage(_) => Some(format!("AniRust-{version}-x86_64.AppImage")),
            Self::Archive(_) => Some(format!("anirust-{version}-linux-x86_64.tar.gz")),
            Self::MacApp(_) => Some(format!("anirust-{version}-macos-arm64.dmg")),
            Self::Elsewhere(_) => None,
        }
    }
}

/// How the running program was installed.
#[must_use]
pub fn detect() -> Install {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.canonicalize().ok());
    detect_from(
        exe.as_deref(),
        std::env::var_os("APPIMAGE").map(PathBuf::from),
        std::env::var_os("FLATPAK_ID").is_some(),
    )
}

fn detect_from(exe: Option<&Path>, appimage: Option<PathBuf>, flatpak: bool) -> Install {
    if flatpak {
        return Install::Elsewhere(Elsewhere::Flatpak);
    }
    if let Some(path) = appimage.filter(|path| path.parent().is_some_and(writable)) {
        return Install::AppImage(path);
    }
    let Some(dir) = exe.and_then(Path::parent) else {
        return Install::Elsewhere(Elsewhere::ByHand);
    };
    if dir.join("unins000.exe").is_file() {
        return Install::WindowsSetup;
    }
    if let Some(app) = dir
        .ancestors()
        .find(|path| path.extension().is_some_and(|ext| ext == "app"))
        && app.parent().is_some_and(writable)
    {
        return Install::MacApp(app.to_path_buf());
    }
    // install.sh: <prefix>/app/bin/anirust, and <prefix>/.install saying so.
    if dir.ends_with("bin")
        && let Some(app) = dir.parent()
        && let Some(prefix) = app.parent()
        && std::fs::read_to_string(prefix.join(".install"))
            .is_ok_and(|marker| marker.lines().any(|line| line == "package=tarball"))
    {
        return Install::Archive(app.to_path_buf());
    }
    if dir.starts_with("/usr") || dir.starts_with("/opt") {
        return Install::Elsewhere(Elsewhere::PackageManager);
    }
    Install::Elsewhere(Elsewhere::ByHand)
}

/// Whether files can be made in `dir`, tried rather than guessed from
/// permission bits.
fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".anirust-write-test-{}", std::process::id()));
    let made = std::fs::File::create(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    made
}

/// How far an update has got, for the line on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Fetching { done: u64, total: Option<u64> },
    Installing,
}

/// What to start once an update is in place: the new version, or for
/// Windows the installer, which starts it when done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<String>,
}

/// Fetches the release's file for `install`, checks its signature, puts it
/// in place, and answers with what to start.
pub async fn install(
    http: reqwest::Client,
    release: Release,
    install: Install,
    report: impl Fn(Progress) + Send + 'static,
) -> Result<Launch, String> {
    let name = install
        .asset_name(release.version)
        .ok_or("this copy is updated by whatever installed it")?;
    let asset = release
        .asset(&name)
        .ok_or_else(|| format!("the release has no {name}"))?;
    let signature = release
        .asset(&format!("{name}.minisig"))
        .ok_or_else(|| format!("the release has no signature for {name}"))?;

    // Beside an AppImage, so the last step is a rename on one filesystem;
    // in the cache otherwise.
    let dir = match &install {
        Install::AppImage(path) => path
            .parent()
            .map(Path::to_path_buf)
            .ok_or("the AppImage has no folder")?,
        _ => anirust_player::frames::work_dir().join("update"),
    };
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| e.to_string())?;
    let file = dir.join(format!("{name}.partial"));

    let response = http
        .get(&asset.url)
        .timeout(std::time::Duration::from_secs(3600))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| e.to_string())?;
    let total = response.content_length();
    let mut out = tokio::fs::File::create(&file)
        .await
        .map_err(|e| e.to_string())?;
    let mut done = 0u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        tokio::io::AsyncWriteExt::write_all(&mut out, &chunk)
            .await
            .map_err(|e| e.to_string())?;
        done += chunk.len() as u64;
        report(Progress::Fetching { done, total });
    }
    tokio::io::AsyncWriteExt::flush(&mut out)
        .await
        .map_err(|e| e.to_string())?;
    drop(out);

    let signature = http
        .get(&signature.url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;

    report(Progress::Installing);
    tokio::task::spawn_blocking(move || {
        let placed = verify(&file, &signature, RELEASE_KEY, &name)
            .and_then(|()| put_in_place(&install, &file, release.version));
        let _ = std::fs::remove_file(&file);
        placed
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Checks `file` against a minisign signature by `key`. The signature's
/// trusted comment, which is signed too, must name the file: the release
/// writes `AniRust <version>: <file>` there, so a file signed for one
/// release cannot be passed off as another's — an old installer as the
/// newest, say.
pub fn verify(file: &Path, signature: &str, key: &str, name: &str) -> Result<(), String> {
    let key = minisign_verify::PublicKey::from_base64(key).map_err(|e| e.to_string())?;
    let signature = minisign_verify::Signature::decode(signature).map_err(|e| e.to_string())?;
    if !signature.trusted_comment().ends_with(&format!(": {name}")) {
        return Err(format!("the signature is for another file than {name}"));
    }
    let mut verifier = key
        .verify_stream(&signature)
        .map_err(|e| format!("the signature does not match the release key: {e}"))?;
    let mut input = std::fs::File::open(file).map_err(|e| e.to_string())?;
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        verifier.update(&buffer[..read]);
    }
    verifier
        .finalize()
        .map_err(|_| format!("{name} is not signed by the AniRust release key"))
}

/// Puts a verified download where the running copy is.
fn put_in_place(install: &Install, file: &Path, version: Version) -> Result<Launch, String> {
    match install {
        Install::WindowsSetup => {
            // The installer cannot run from a name ending in .partial.
            let setup = file.with_extension("");
            std::fs::rename(file, &setup).map_err(|e| e.to_string())?;
            Ok(Launch {
                program: setup,
                args: [
                    "/VERYSILENT",
                    "/SUPPRESSMSGBOXES",
                    "/NORESTART",
                    "/update=yes",
                ]
                .map(str::to_owned)
                .to_vec(),
            })
        }
        Install::AppImage(path) => {
            make_executable(file)?;
            std::fs::rename(file, path).map_err(|e| e.to_string())?;
            Ok(Launch {
                program: path.clone(),
                args: Vec::new(),
            })
        }
        Install::Archive(app) => {
            let fresh = app.with_extension("new");
            let old = app.with_extension("old");
            let _ = std::fs::remove_dir_all(&fresh);
            let _ = std::fs::remove_dir_all(&old);
            unpack_archive(file, &fresh)?;
            if !fresh.join("anirust").is_file() {
                let _ = std::fs::remove_dir_all(&fresh);
                return Err("the archive did not hold the program".to_owned());
            }
            // Swapped whole, so an interrupted update never leaves half of
            // each version.
            std::fs::rename(app, &old).map_err(|e| e.to_string())?;
            if let Err(error) = std::fs::rename(&fresh, app) {
                let _ = std::fs::rename(&old, app);
                return Err(error.to_string());
            }
            let _ = std::fs::remove_dir_all(&old);
            if let Some(prefix) = app.parent() {
                let _ = std::fs::write(
                    prefix.join(".install"),
                    format!("package=tarball\nversion={version}\n"),
                );
            }
            Ok(Launch {
                program: app.join("anirust"),
                args: Vec::new(),
            })
        }
        Install::MacApp(app) => {
            replace_app_from_dmg(file, app)?;
            Ok(Launch {
                program: PathBuf::from("open"),
                args: vec!["-n".to_owned(), app.to_string_lossy().into_owned()],
            })
        }
        Install::Elsewhere(_) => Err("this copy is updated by whatever installed it".to_owned()),
    }
}

#[cfg(unix)]
fn make_executable(file: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn make_executable(_file: &Path) -> Result<(), String> {
    Ok(())
}

/// Unpacks the Linux archive into `into`, without its top folder, keeping
/// each file's mode: the launcher and the programs must stay executable.
fn unpack_archive(archive: &Path, into: &Path) -> Result<(), String> {
    std::fs::create_dir_all(into).map_err(|e| e.to_string())?;
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    tar.set_preserve_permissions(true);
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        // Refuses anything that would climb out of `into`.
        let Some(relative) = crate::runtimes::without_top(&path) else {
            continue;
        };
        let out = into.join(relative);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        entry.unpack(&out).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Mounts the disk image, copies its bundle beside the running one, and
/// swaps the two.
fn replace_app_from_dmg(dmg: &Path, app: &Path) -> Result<(), String> {
    let mount = dmg.with_extension("mount");
    let _ = std::fs::create_dir_all(&mount);
    let run = |program: &str, args: &[&std::ffi::OsStr]| -> Result<(), String> {
        let status = std::process::Command::new(program)
            .args(args)
            .status()
            .map_err(|e| format!("{program}: {e}"))?;
        status
            .success()
            .then_some(())
            .ok_or_else(|| format!("{program} failed ({status})"))
    };
    run(
        "hdiutil",
        &[
            "attach".as_ref(),
            "-nobrowse".as_ref(),
            "-readonly".as_ref(),
            "-mountpoint".as_ref(),
            mount.as_os_str(),
            dmg.as_os_str(),
        ],
    )?;
    let fresh = app.with_extension("app-new");
    let _ = std::fs::remove_dir_all(&fresh);
    let copied = run(
        "ditto",
        &[mount.join("AniRust.app").as_os_str(), fresh.as_os_str()],
    );
    let _ = run(
        "hdiutil",
        &["detach".as_ref(), mount.as_os_str(), "-quiet".as_ref()],
    );
    let _ = std::fs::remove_dir(&mount);
    copied?;
    let old = app.with_extension("app-old");
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(app, &old).map_err(|e| e.to_string())?;
    if let Err(error) = std::fs::rename(&fresh, app) {
        let _ = std::fs::rename(&old, app);
        return Err(error.to_string());
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

/// Starts what an update left to start.
pub fn launch(launch: &Launch) -> std::io::Result<()> {
    std::process::Command::new(&launch.program)
        .args(&launch.args)
        .spawn()
        .map(drop)
}

// ---------------------------------------------------------------------------
// The window's side
// ---------------------------------------------------------------------------

/// What the updater has found, shared by its callbacks.
#[derive(Default)]
struct Found {
    release: Option<Release>,
    /// A check or an install is under way.
    busy: bool,
}

/// Who updates this copy, as the window names it: empty when the program
/// does.
fn how(install: &Install) -> &'static str {
    match install {
        Install::Elsewhere(Elsewhere::Flatpak) => "flatpak",
        Install::Elsewhere(Elsewhere::PackageManager) => "package",
        Install::Elsewhere(Elsewhere::ByHand) => "hand",
        _ => "",
    }
}

/// Wires the settings' switch and buttons and the update card, and checks
/// a few seconds after start and once a day after, when checking is on.
/// Answers with the timers, which must be held for as long as the window.
pub fn wire(
    window: &MainWindow,
    http: reqwest::Client,
    prefs: &Rc<Cell<Preferences>>,
) -> [slint::Timer; 2] {
    let found = Rc::new(RefCell::new(Found::default()));
    let install = detect();
    tracing::info!(?install, "installed as");
    window.set_check_updates(prefs.get().check_updates);
    window.set_update_how(how(&install).into());

    let check: Rc<dyn Fn(bool)> = {
        let weak = window.as_weak();
        let found = Rc::clone(&found);
        let http = http.clone();
        Rc::new(move |asked: bool| check(&weak, &found, http.clone(), asked))
    };

    window.on_check_for_updates({
        let check = Rc::clone(&check);
        move || check(true)
    });
    window.on_set_check_updates({
        let weak = window.as_weak();
        let prefs = Rc::clone(prefs);
        let check = Rc::clone(&check);
        move |on| {
            let Some(window) = weak.upgrade() else { return };
            let mut kept = prefs.get();
            kept.check_updates = on;
            prefs.set(kept);
            kept.save();
            window.set_check_updates(on);
            if on {
                check(false);
            }
        }
    });
    window.on_open_update_notes({
        let found = Rc::clone(&found);
        move || {
            if let Some(release) = &found.borrow().release {
                crate::release::open_in_browser(&release.page);
            }
        }
    });
    window.on_install_update({
        let weak = window.as_weak();
        let found = Rc::clone(&found);
        move || start_install(&weak, &found, http.clone(), install.clone())
    });

    let first = slint::Timer::default();
    let daily = slint::Timer::default();
    let wanted = {
        let prefs = Rc::clone(prefs);
        move || prefs.get().check_updates
    };
    {
        let check = Rc::clone(&check);
        let wanted = wanted.clone();
        // Not at once: the first seconds belong to the catalogue loading.
        first.start(
            slint::TimerMode::SingleShot,
            Duration::from_secs(8),
            move || {
                if wanted() {
                    check(false);
                }
            },
        );
    }
    daily.start(
        slint::TimerMode::Repeated,
        Duration::from_secs(24 * 60 * 60),
        move || {
            if wanted() {
                check(false);
            }
        },
    );
    [first, daily]
}

/// Asks for the latest release. A check the viewer asked for says how it
/// went, failure included; one of the program's own is quiet unless there
/// is something to offer.
fn check(
    weak: &slint::Weak<MainWindow>,
    found: &Rc<RefCell<Found>>,
    http: reqwest::Client,
    asked: bool,
) {
    let Some(window) = weak.upgrade() else { return };
    if std::mem::replace(&mut found.borrow_mut().busy, true) {
        return;
    }
    if asked {
        window.set_update_state("checking".into());
    }
    let weak = weak.clone();
    let found = Rc::clone(found);
    tasks::spawn(latest(http), move |result| {
        found.borrow_mut().busy = false;
        let Some(window) = weak.upgrade() else { return };
        match result {
            Ok(release) if release.version > Version::current() => {
                tracing::info!(version = %release.version, "a newer release is out");
                let version = release.version.to_string();
                // A release newer than the one put away brings the card back.
                if window.get_update_version() != version.as_str() {
                    window.set_update_dismissed(false);
                }
                window.set_update_version(version.into());
                window.set_update_state("available".into());
                found.borrow_mut().release = Some(release);
            }
            Ok(_) => {
                if asked || window.get_update_state() == "checking" {
                    window.set_update_state("latest".into());
                }
            }
            Err(error) => {
                tracing::warn!(%error, "the update check failed");
                if asked {
                    window.set_update_error(error.into());
                    window.set_update_state("check-failed".into());
                }
            }
        }
    });
}

/// Fetches, checks and installs the release found, then starts the new
/// version and closes this one.
fn start_install(
    weak: &slint::Weak<MainWindow>,
    found: &Rc<RefCell<Found>>,
    http: reqwest::Client,
    install: Install,
) {
    let Some(window) = weak.upgrade() else { return };
    let release = {
        let mut found = found.borrow_mut();
        if found.busy {
            return;
        }
        let Some(release) = found.release.clone() else {
            return;
        };
        found.busy = true;
        release
    };
    window.set_update_progress(0.0);
    window.set_update_state("downloading".into());

    let (send, mut receive) = tokio::sync::mpsc::unbounded_channel::<Progress>();
    {
        let weak = weak.clone();
        let _ = slint::spawn_local(async move {
            while let Some(progress) = receive.recv().await {
                let Some(window) = weak.upgrade() else { return };
                match progress {
                    Progress::Fetching { done, total } => {
                        if let Some(total) = total.filter(|total| *total > 0) {
                            #[allow(clippy::cast_precision_loss)]
                            window.set_update_progress((done as f64 / total as f64) as f32);
                        }
                    }
                    Progress::Installing => window.set_update_state("installing".into()),
                }
            }
        });
    }

    let weak = weak.clone();
    let found = Rc::clone(found);
    tasks::spawn(
        self::install(http, release, install, move |progress| {
            let _ = send.send(progress);
        }),
        move |result| {
            found.borrow_mut().busy = false;
            let Some(window) = weak.upgrade() else { return };
            match result.and_then(|launch| launch_new(&launch).map(|()| launch)) {
                Ok(launch) => {
                    tracing::info!(program = %launch.program.display(), "updated; restarting");
                    // Watch positions are written every couple of seconds,
                    // so closing now loses nothing.
                    let _ = slint::quit_event_loop();
                }
                Err(error) => {
                    tracing::warn!(%error, "the update was not installed");
                    window.set_update_error(error.into());
                    window.set_update_state("failed".into());
                    window.set_update_dismissed(false);
                }
            }
        },
    );
}

fn launch_new(launch: &Launch) -> Result<(), String> {
    self::launch(launch).map_err(|e| format!("{}: {e}", launch.program.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY: &str = include_str!("../tests/fixtures/update-test.pub");
    const TEST_SIGNATURE: &str = include_str!("../tests/fixtures/update-test.bin.minisig");

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    #[test]
    fn versions_compare_by_number_not_by_text() {
        assert!(Version::parse("1.10.0") > Version::parse("1.9.3"));
        assert_eq!(Version::parse("v1.0.2"), Some(Version(1, 0, 2)));
        assert_eq!(Version::parse("1.0"), None);
        assert_eq!(Version::parse("1.0.2-rc1"), None);
        assert_eq!(Version::parse("1.0.2.4"), None);
        assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn a_release_is_read_from_the_api_answer() {
        let json = r#"{"tag_name":"v1.1.0","html_url":"https://github.com/mrFrok/AniRust/releases/tag/v1.1.0",
            "draft":false,"prerelease":false,"assets":[
            {"name":"AniRust-1.1.0-setup.exe","browser_download_url":"https://example.com/a"},
            {"name":"AniRust-1.1.0-setup.exe.minisig","browser_download_url":"https://example.com/b"}]}"#;
        let release = parse_release(json).unwrap();
        assert_eq!(release.version, Version(1, 1, 0));
        assert_eq!(
            release.asset("AniRust-1.1.0-setup.exe").unwrap().url,
            "https://example.com/a"
        );
        let pre = json.replace(r#""prerelease":false"#, r#""prerelease":true"#);
        assert!(parse_release(&pre).is_err());
    }

    #[test]
    fn each_install_updates_from_its_own_file() {
        let v = Version(1, 1, 0);
        assert_eq!(
            Install::WindowsSetup.asset_name(v).unwrap(),
            "AniRust-1.1.0-setup.exe"
        );
        assert_eq!(
            Install::AppImage(PathBuf::new()).asset_name(v).unwrap(),
            "AniRust-1.1.0-x86_64.AppImage"
        );
        assert_eq!(
            Install::Archive(PathBuf::new()).asset_name(v).unwrap(),
            "anirust-1.1.0-linux-x86_64.tar.gz"
        );
        assert_eq!(Install::Elsewhere(Elsewhere::Flatpak).asset_name(v), None);
    }

    #[test]
    fn the_install_is_told_from_where_the_program_runs() {
        let root = std::env::temp_dir().join(format!("anirust-update-test-{}", std::process::id()));
        let bin = root.join("opt/anirust/app/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let exe = bin.join("anirust");
        // Without install.sh's marker it is an archive unpacked by hand.
        assert_eq!(
            detect_from(Some(&exe), None, false),
            Install::Elsewhere(Elsewhere::ByHand)
        );
        std::fs::write(
            root.join("opt/anirust/.install"),
            "package=tarball\nversion=1.0.2\n",
        )
        .unwrap();
        assert_eq!(
            detect_from(Some(&exe), None, false),
            Install::Archive(root.join("opt/anirust/app"))
        );
        // An AppImage in a writable folder replaces itself; Flatpak never.
        let appimage = root.join("AniRust.AppImage");
        assert_eq!(
            detect_from(Some(&exe), Some(appimage.clone()), false),
            Install::AppImage(appimage.clone())
        );
        assert_eq!(
            detect_from(Some(&exe), Some(appimage), true),
            Install::Elsewhere(Elsewhere::Flatpak)
        );
        // Beside the Windows installer's uninstaller.
        std::fs::write(bin.join("unins000.exe"), b"").unwrap();
        assert_eq!(detect_from(Some(&exe), None, false), Install::WindowsSetup);
        assert_eq!(
            detect_from(Some(Path::new("/usr/bin/anirust")), None, false),
            Install::Elsewhere(Elsewhere::PackageManager)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The Linux archive over the one install.sh put there: swapped whole,
    /// the launcher still executable, the marker moved on.
    #[cfg(unix)]
    #[test]
    fn an_archive_update_replaces_the_folder_whole() {
        use std::os::unix::fs::PermissionsExt;

        let root =
            std::env::temp_dir().join(format!("anirust-archive-test-{}", std::process::id()));
        let app = root.join("app");
        std::fs::create_dir_all(app.join("bin")).unwrap();
        std::fs::write(app.join("anirust"), b"old launcher").unwrap();
        std::fs::write(app.join("left-over.so"), b"only the old version had this").unwrap();
        std::fs::write(root.join(".install"), "package=tarball\nversion=1.0.2\n").unwrap();

        let archive = root.join("update.tar.gz.partial");
        {
            let file = std::fs::File::create(&archive).unwrap();
            let gz = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
            let mut tar = tar::Builder::new(gz);
            for (path, body, mode) in [
                (
                    "anirust-9.9.9-linux-x86_64/anirust",
                    &b"new launcher"[..],
                    0o755,
                ),
                (
                    "anirust-9.9.9-linux-x86_64/bin/anirust",
                    &b"new program"[..],
                    0o755,
                ),
            ] {
                let mut header = tar::Header::new_gnu();
                header.set_size(body.len() as u64);
                header.set_mode(mode);
                header.set_cksum();
                tar.append_data(&mut header, path, body).unwrap();
            }
            tar.into_inner().unwrap().finish().unwrap();
        }

        let launch =
            put_in_place(&Install::Archive(app.clone()), &archive, Version(9, 9, 9)).unwrap();
        assert_eq!(launch.program, app.join("anirust"));
        assert_eq!(std::fs::read(app.join("anirust")).unwrap(), b"new launcher");
        assert!(!app.join("left-over.so").exists());
        let mode = std::fs::metadata(app.join("bin/anirust"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "{mode:o}");
        assert!(
            std::fs::read_to_string(root.join(".install"))
                .unwrap()
                .contains("version=9.9.9")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_signed_file_verifies_and_anything_else_does_not() {
        let file = fixture("update-test.bin");
        let key = TEST_KEY.trim();
        verify(&file, TEST_SIGNATURE, key, "update-test.bin").unwrap();

        // Signed, but for another file name.
        assert!(verify(&file, TEST_SIGNATURE, key, "AniRust-9.9.9-setup.exe").is_err());
        // Another key: the release key did not sign the fixture.
        assert!(verify(&file, TEST_SIGNATURE, RELEASE_KEY, "update-test.bin").is_err());
        // The same name, other bytes.
        let tampered =
            std::env::temp_dir().join(format!("anirust-tampered-{}", std::process::id()));
        std::fs::write(&tampered, b"AniRust update test file!\n").unwrap();
        assert!(verify(&tampered, TEST_SIGNATURE, key, "update-test.bin").is_err());
        let _ = std::fs::remove_file(tampered);
        // No signature at all.
        assert!(verify(&file, "", key, "update-test.bin").is_err());
    }
}

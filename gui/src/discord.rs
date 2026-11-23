// SPDX-License-Identifier: GPL-3.0-or-later

//! Discord Rich Presence.
//!
//! Discord is told what is playing over its own local IPC: a socket on Unix, a
//! named pipe on Windows, framed JSON either way. Nothing leaves the machine
//! except the activity Discord itself sends to its servers, which is the whole
//! point of the feature — the title, the episode and the time left are what
//! somebody's friends see in their member list.
//!
//! The talking happens on a thread of its own. Discord may not be running, may
//! start later, or may be restarted mid-episode, and none of that is allowed
//! to hold up the window or lose the current activity: the thread keeps the
//! last thing it was asked to show and reconnects around it.
//!
//! Nothing is sent unless it changed, and no more often than Discord's rate
//! limit allows. A quarter-second status tick would otherwise be four
//! SET_ACTIVITY commands a second.

use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The Discord application presence is posted under.
///
/// Register an application at <https://discord.com/developers/applications>,
/// name it AniRust, and paste its Application ID here. The same ID can be
/// given with `ANIRUST_DISCORD_CLIENT_ID`, at run time or when building, which
/// is what a build from source uses without touching this file.
///
/// Upload an asset called `anirust` on that application's Rich Presence page
/// for when no episode poster is shown. The icon in `packaging/icons/anirust.svg`
/// is the one to use.
const DEFAULT_CLIENT_ID: &str = "1574480257237393428";

/// The name Discord shows beside the activity.
const APPLICATION_NAME: &str = "AniRust";

/// The asset key on the Discord application. See [`DEFAULT_CLIENT_ID`].
const ARTWORK: &str = "anirust";

/// Where the buttons lead.
const PROJECT_URL: &str = "https://github.com/mrfrok/AniRust";

/// How a title is opened on the service's own site.
const RELEASE_URL: &str = "https://anixart.tv/release/";

/// Discord accepts five activity updates in twenty seconds. Five seconds
/// apart is four, which leaves a margin without making a pause feel
/// unresponsive.
const UPDATE_INTERVAL: Duration = Duration::from_secs(5);

/// How often to look for a Discord that was not running last time.
const RECONNECT_INTERVAL: Duration = Duration::from_secs(15);

/// How long the worker sleeps when there is nothing to do.
const IDLE_TICK: Duration = Duration::from_millis(500);

/// Longest `details` and `state` Discord will take. Longer is cut rather than
/// refused, and a character count rather than a byte count keeps a Cyrillic
/// title from being cut mid-letter.
const TEXT_LIMIT: usize = 128;

/// Longest button label Discord will take.
const BUTTON_LIMIT: usize = 32;

/// What the viewer is doing, as Discord can draw it.
///
/// Owned rather than borrowed: it crosses a channel to the presence thread,
/// which may still be holding the previous one when the next tick arrives.
#[derive(Clone, PartialEq, Eq)]
pub struct Activity {
    details: String,
    state: String,
    /// Milliseconds since the Unix epoch, or none when there is no clock to
    /// draw — a pause, a stream still being found.
    started: Option<i64>,
    ends: Option<i64>,
    /// Discord's activity type: 3 is "Watching", 0 is "Playing".
    kind: u8,
    /// The release's cover, when it has one Discord can fetch.
    poster: Option<String>,
    buttons: Vec<(String, String)>,
}

impl Activity {
    /// An episode being watched.
    ///
    /// `position` is where the picture is now and `duration` its length; the
    /// two become the elapsed/remaining clock Discord draws under the title.
    /// A pause drops the clock instead of leaving it counting: a pause that
    /// goes on counting is a lie about what is happening.
    #[must_use]
    pub fn watching(
        title: &str,
        line: &str,
        url: Option<&str>,
        position: Duration,
        duration: Option<Duration>,
        paused: bool,
        russian: bool,
    ) -> Self {
        let state = if paused {
            format!(
                "{} · {}",
                clipped(line, TEXT_LIMIT / 2),
                if russian { "на паузе" } else { "paused" }
            )
        } else {
            clipped(line, TEXT_LIMIT)
        };

        let (started, ends) = if paused {
            (None, None)
        } else {
            let position_ms = i64::try_from(position.as_millis()).unwrap_or(i64::MAX);
            let watched = now_ms().saturating_sub(position_ms);
            (
                Some(watched),
                duration.map(|total| {
                    watched.saturating_add(i64::try_from(total.as_millis()).unwrap_or(i64::MAX))
                }),
            )
        };

        let mut buttons = Vec::new();
        if let Some(url) = url {
            buttons.push((
                clipped(
                    if russian {
                        "Открыть на Anixart"
                    } else {
                        "Open on Anixart"
                    },
                    BUTTON_LIMIT,
                ),
                url.to_owned(),
            ));
        }
        buttons.push((
            clipped(
                if russian {
                    "AniRust на GitHub"
                } else {
                    "AniRust on GitHub"
                },
                BUTTON_LIMIT,
            ),
            PROJECT_URL.to_owned(),
        ));

        Self {
            details: clipped(title, TEXT_LIMIT),
            state,
            started,
            ends,
            kind: 3,
            poster: None,
            buttons,
        }
    }

    /// Uses a public HTTPS cover in place of the application's asset.
    #[must_use]
    pub fn with_poster(mut self, poster: Option<&str>) -> Self {
        self.poster = poster
            .and_then(|poster| reqwest::Url::parse(poster).ok())
            .filter(|url| {
                url.scheme() == "https"
                    && url.has_host()
                    && url.username().is_empty()
                    && url.password().is_none()
            })
            .map(Into::into);
        self
    }

    /// The application with nothing playing.
    #[must_use]
    pub fn browsing(russian: bool) -> Self {
        Self {
            details: APPLICATION_NAME.to_owned(),
            state: if russian {
                "Выбирает, что посмотреть"
            } else {
                "Choosing what to watch"
            }
            .to_owned(),
            started: None,
            ends: None,
            kind: 0,
            poster: None,
            buttons: vec![(
                clipped(
                    if russian {
                        "Исходный код"
                    } else {
                        "Source code"
                    },
                    BUTTON_LIMIT,
                ),
                PROJECT_URL.to_owned(),
            )],
        }
    }

    /// Whether two activities say the same thing.
    ///
    /// The timestamps are compared with a tolerance: the clock is recomputed
    /// from the player's position four times a second, and a few hundred
    /// milliseconds of drift between two reads is not news worth a command.
    fn same(&self, other: &Self) -> bool {
        self.details == other.details
            && self.state == other.state
            && self.kind == other.kind
            && self.poster == other.poster
            && self.buttons == other.buttons
            && near(self.started, other.started)
            && near(self.ends, other.ends)
    }

    fn to_json(&self) -> serde_json::Value {
        let mut activity = serde_json::Map::new();
        activity.insert("details".into(), self.details.clone().into());
        activity.insert("state".into(), self.state.clone().into());
        activity.insert("type".into(), self.kind.into());
        // Show the title in the member list rather than "Watching AniRust".
        activity.insert("status_display_type".into(), 2.into());

        if self.started.is_some() || self.ends.is_some() {
            let mut timestamps = serde_json::Map::new();
            if let Some(start) = self.started {
                timestamps.insert("start".into(), start.into());
            }
            if let Some(end) = self.ends {
                timestamps.insert("end".into(), end.into());
            }
            activity.insert("timestamps".into(), timestamps.into());
        }

        // Discord fetches a poster through its media proxy. With no URL,
        // the application's uploaded logo is used instead.
        let (large_image, large_text) = match &self.poster {
            Some(url) => (url.as_str(), self.details.as_str()),
            None => (ARTWORK, APPLICATION_NAME),
        };
        activity.insert(
            "assets".into(),
            serde_json::json!({
                "large_image": large_image,
                "large_text": large_text,
            }),
        );

        if !self.buttons.is_empty() {
            let buttons: Vec<serde_json::Value> = self
                .buttons
                .iter()
                .take(2)
                .map(|(label, url)| serde_json::json!({ "label": label, "url": url }))
                .collect();
            activity.insert("buttons".into(), buttons.into());
        }

        serde_json::Value::Object(activity)
    }
}

/// Two timestamps within the same breath of the clock.
fn near(one: Option<i64>, other: Option<i64>) -> bool {
    match (one, other) {
        (Some(one), Some(other)) => one.abs_diff(other) < 3_000,
        (None, None) => true,
        _ => false,
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or_default()
}

fn clipped(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(limit.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// The application ID presence is posted under, if one is configured.
fn client_id() -> Option<String> {
    let from_env = std::env::var("ANIRUST_DISCORD_CLIENT_ID")
        .ok()
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty());
    // Also baked in at build time, so a package started from the applications
    // menu does not depend on the environment it was started from.
    let built_in = option_env!("ANIRUST_DISCORD_CLIENT_ID")
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned);
    let from_source =
        (!DEFAULT_CLIENT_ID.trim().is_empty()).then(|| DEFAULT_CLIENT_ID.trim().to_owned());

    from_env.or(built_in).or(from_source)
}

/// The release's page on the service's own site, for the button.
#[must_use]
pub fn release_url(release_id: i64) -> Option<String> {
    (release_id > 0).then(|| format!("{RELEASE_URL}{release_id}"))
}

/// A handle to the presence thread.
///
/// Cheap to hold and to call; every method only posts a command. Dropping it
/// asks the thread to clear the activity and stop.
pub struct Presence {
    enabled: std::cell::Cell<bool>,
    tx: Option<Sender<Command>>,
    configured: bool,
}

impl Presence {
    /// Starts the thread, with presence on or off as `enabled` says.
    ///
    /// Without an application ID there is no thread at all, and every method
    /// is a no-op — the same shape as a build where the feature is unused.
    #[must_use]
    pub fn start(enabled: bool) -> Self {
        let Some(id) = client_id() else {
            return Self {
                enabled: std::cell::Cell::new(false),
                tx: None,
                configured: false,
            };
        };

        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("discord presence".into())
            .spawn(move || run(&id, &rx));

        match spawned {
            Ok(_) => Self {
                enabled: std::cell::Cell::new(enabled),
                tx: Some(tx),
                configured: true,
            },
            Err(error) => {
                tracing::warn!(%error, "the Discord presence thread was not started");
                Self {
                    enabled: std::cell::Cell::new(false),
                    tx: None,
                    configured: false,
                }
            }
        }
    }

    /// Whether a Discord application is configured; the settings row reads it.
    #[must_use]
    pub fn configured(&self) -> bool {
        self.configured
    }

    /// Shows `activity`, if presence is on.
    pub fn show(&self, activity: Activity) {
        if !self.enabled.get() {
            return;
        }
        if let Some(tx) = &self.tx {
            let _ = tx.send(Command::Show(activity));
        }
    }

    /// Turns presence on or off from the settings.
    ///
    /// Off clears the activity at once rather than hiding it from the next
    /// tick: a switch that leaves the title up until something else happens
    /// looks broken. On waits for the next [`Self::show`] rather than keeping
    /// a stale activity of its own.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.set(enabled);
        if let Some(tx) = &self.tx {
            let _ = tx.send(if enabled {
                Command::Wake
            } else {
                Command::Hide
            });
        }
    }
}

impl Drop for Presence {
    fn drop(&mut self) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Command::Stop);
        }
    }
}

enum Command {
    Show(Activity),
    Hide,
    /// Discord may have appeared since the last attempt.
    Wake,
    Stop,
}

fn run(client_id: &str, rx: &Receiver<Command>) {
    let mut link: Option<Link> = None;
    let mut desired: Option<Activity> = None;
    let mut sent: Option<Activity> = None;
    let mut last_sent: Option<Instant> = None;
    let mut next_attempt = Instant::now();

    loop {
        match rx.recv_timeout(IDLE_TICK) {
            Ok(Command::Show(activity)) => desired = Some(activity),
            Ok(Command::Hide) => {
                desired = None;
                sent = None;
                last_sent = Some(Instant::now());
                if let Some(link) = &mut link {
                    let _ = link.clear();
                }
            }
            Ok(Command::Wake) => next_attempt = Instant::now(),
            Ok(Command::Stop) => {
                if let Some(link) = &mut link {
                    let _ = link.clear();
                }
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        let Some(want) = &desired else {
            continue;
        };

        // A Discord that was closed and opened again is a new connection,
        // and the activity on it has to be sent afresh even though nothing
        // about it changed: a reconnect clears `sent` for exactly that.
        if link.is_none() && Instant::now() >= next_attempt {
            next_attempt = Instant::now() + RECONNECT_INTERVAL;
            match Link::connect(client_id) {
                Ok(connected) => {
                    tracing::info!("Discord presence connected");
                    link = Some(connected);
                    sent = None;
                }
                Err(error) => {
                    // Discord not running is the ordinary case, not a fault
                    // worth a warning line on every start.
                    tracing::debug!(%error, "Discord is not listening");
                }
            }
        }

        let Some(connected) = &mut link else {
            continue;
        };
        let changed = sent.as_ref().is_none_or(|sent| !sent.same(want));
        let due = last_sent.is_none_or(|at| at.elapsed() >= UPDATE_INTERVAL);
        if !changed || !due {
            continue;
        }

        match connected.show(want) {
            Ok(()) => {
                sent = Some(want.clone());
                last_sent = Some(Instant::now());
            }
            Err(error) => {
                tracing::debug!(%error, "Discord stopped listening");
                link = None;
                sent = None;
                next_attempt = Instant::now() + RECONNECT_INTERVAL;
            }
        }
    }
}

/// One connection to Discord's local socket.
struct Link {
    stream: Stream,
}

enum Stream {
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream),
    #[cfg(windows)]
    Pipe(std::fs::File),
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.write(buf),
            #[cfg(windows)]
            Self::Pipe(file) => file.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.flush(),
            #[cfg(windows)]
            Self::Pipe(file) => file.flush(),
        }
    }
}

impl Link {
    fn connect(client_id: &str) -> io::Result<Self> {
        let mut link = Self {
            stream: open_socket()?,
        };
        // Opcode 0 is the handshake. Discord answers with READY, which there
        // is no need to read: the frames after it are processed in order, and
        // an unread answer costs nothing on a socket nobody else reads.
        link.frame(0, &serde_json::json!({ "v": 1, "client_id": client_id }))?;
        Ok(link)
    }

    fn show(&mut self, activity: &Activity) -> io::Result<()> {
        let nonce = format!("{}:{}", std::process::id(), now_ms());
        let command = serde_json::json!({
            "cmd": "SET_ACTIVITY",
            "nonce": nonce,
            "args": {
                "pid": std::process::id(),
                "activity": activity.to_json(),
            },
        });
        self.frame(1, &command)
    }

    fn clear(&mut self) -> io::Result<()> {
        let nonce = format!("{}:{}", std::process::id(), now_ms());
        let command = serde_json::json!({
            "cmd": "SET_ACTIVITY",
            "nonce": nonce,
            "args": {
                "pid": std::process::id(),
                "activity": serde_json::Value::Null,
            },
        });
        self.frame(1, &command)
    }

    /// Writes one opcode-prefixed frame: little-endian opcode, little-endian
    /// payload length, then the UTF-8 JSON.
    fn frame(&mut self, opcode: u32, payload: &serde_json::Value) -> io::Result<()> {
        let text = payload.to_string();
        let length = u32::try_from(text.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a frame too large"))?;
        let mut header = [0u8; 8];
        header[..4].copy_from_slice(&opcode.to_le_bytes());
        header[4..].copy_from_slice(&length.to_le_bytes());
        self.stream.write_all(&header)?;
        self.stream.write_all(text.as_bytes())?;
        self.stream.flush()
    }
}

#[cfg(unix)]
fn open_socket() -> io::Result<Stream> {
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;

    let mut directories: Vec<PathBuf> = Vec::new();
    let mut add = |directory: PathBuf| {
        if !directories.contains(&directory) {
            directories.push(directory);
        }
    };

    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        let runtime = PathBuf::from(runtime);
        add(runtime.clone());
        // Discord from Flathub and from the Snap store runs in its own
        // sandbox, where the socket sits under a directory of its own.
        add(runtime.join("app/com.discordapp.Discord"));
        add(runtime.join("snap.discord"));
        add(runtime.join(".flatpak/com.discordapp.Discord"));
    }
    if let Some(temporary) = std::env::var_os("TMPDIR") {
        add(PathBuf::from(temporary));
    }
    add(PathBuf::from("/tmp"));

    for directory in directories {
        for index in 0..10 {
            let path = directory.join(format!("discord-ipc-{index}"));
            match UnixStream::connect(&path) {
                Ok(stream) => return Ok(Stream::Unix(stream)),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                    ) => {}
                Err(error) => {
                    tracing::debug!(%error, path = %path.display(), "a Discord socket would not open");
                }
            }
        }
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "no Discord IPC socket",
    ))
}

#[cfg(windows)]
fn open_socket() -> io::Result<Stream> {
    use std::fs::OpenOptions;

    for index in 0..10 {
        let path = format!(r"\\.\pipe\discord-ipc-{index}");
        match OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => return Ok(Stream::Pipe(file)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::debug!(%error, pipe = %path, "a Discord pipe would not open");
            }
        }
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "no Discord IPC pipe",
    ))
}

#[cfg(not(any(unix, windows)))]
fn open_socket() -> io::Result<Stream> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Discord IPC is not available on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_is_cut_to_what_discord_takes() {
        let long = "а".repeat(TEXT_LIMIT + 20);
        let activity =
            Activity::watching(&long, "Серия 1", None, Duration::ZERO, None, false, true);
        assert_eq!(activity.details.chars().count(), TEXT_LIMIT);
        assert!(activity.details.ends_with('…'));
    }

    #[test]
    fn a_playing_episode_has_a_clock() {
        let activity = Activity::watching(
            "Тест",
            "Серия 2",
            Some("https://anixart.tv/release/1"),
            Duration::from_secs(60),
            Some(Duration::from_secs(600)),
            false,
            true,
        );
        let started = activity.started.expect("a start");
        let ends = activity.ends.expect("an end");
        assert!((ends - started - 600_000).abs() < 1_000);
        assert_eq!(activity.buttons.len(), 2);
        assert_eq!(activity.kind, 3);
    }

    #[test]
    fn a_pause_drops_the_clock() {
        let activity = Activity::watching(
            "Тест",
            "Серия 2",
            None,
            Duration::from_secs(60),
            Some(Duration::from_secs(600)),
            true,
            false,
        );
        assert!(activity.started.is_none());
        assert!(activity.ends.is_none());
        assert!(activity.state.contains("paused"));
    }

    #[test]
    fn the_same_activity_with_a_little_clock_drift_is_not_news() {
        let one = Activity::watching(
            "Тест",
            "Серия 2",
            None,
            Duration::from_secs(60),
            None,
            false,
            true,
        );
        let other = Activity {
            started: one.started.map(|at| at + 2_000),
            ..one.clone()
        };
        assert!(one.same(&other));
    }

    #[test]
    fn a_different_episode_is_news() {
        let one = Activity::watching("Тест", "Серия 2", None, Duration::ZERO, None, false, true);
        let other = Activity::watching("Тест", "Серия 3", None, Duration::ZERO, None, false, true);
        assert!(!one.same(&other));
    }

    #[test]
    fn the_json_says_it_is_watching() {
        let activity =
            Activity::watching("Тест", "Серия 2", None, Duration::ZERO, None, false, true);
        let json = activity.to_json();
        assert_eq!(json["type"], 3);
        assert_eq!(json["details"], "Тест");
        assert_eq!(json["assets"]["large_image"], "anirust");
    }

    #[test]
    fn a_watching_activity_can_show_the_release_poster() {
        let url = "https://s.anixmirai.com/posters/cover.jpg";
        let plain = Activity::watching("Тест", "Серия 2", None, Duration::ZERO, None, true, true);
        let covered = plain.clone().with_poster(Some(url));
        assert_eq!(covered.to_json()["assets"]["large_image"], url);
        assert_eq!(covered.to_json()["assets"]["large_text"], "Тест");
        assert!(!plain.same(&covered));
        assert_eq!(
            Activity::browsing(true).to_json()["assets"]["large_image"],
            ARTWORK
        );
    }

    #[test]
    fn a_missing_or_invalid_poster_uses_the_app_asset() {
        let watch = Activity::watching("Тест", "Серия 2", None, Duration::ZERO, None, true, true);
        for poster in [
            None,
            Some("https://"),
            Some("file:///tmp/poster.jpg"),
            Some("http://example.com/poster.jpg"),
            Some("https://user:password@example.com/poster.jpg"),
        ] {
            let json = watch.clone().with_poster(poster).to_json();
            assert_eq!(json["assets"]["large_image"], ARTWORK);
        }
    }

    /// The frame Discord reads: opcode 1, its length, then the command.
    #[cfg(unix)]
    #[test]
    fn a_show_is_one_framed_set_activity_command() {
        use std::io::Read;

        let (ours, mut discord) = std::os::unix::net::UnixStream::pair().expect("a socket pair");
        let mut link = Link {
            stream: Stream::Unix(ours),
        };
        let activity =
            Activity::watching("Тест", "Серия 2", None, Duration::ZERO, None, false, true);
        link.show(&activity).expect("a frame is written");

        let mut header = [0u8; 8];
        discord.read_exact(&mut header).expect("an opcode");
        let opcode = u32::from_le_bytes(header[..4].try_into().expect("four bytes"));
        let length = u32::from_le_bytes(header[4..].try_into().expect("four bytes"));
        let mut body = vec![0u8; length as usize];
        discord.read_exact(&mut body).expect("a body");

        assert_eq!(opcode, 1);
        let command: serde_json::Value = serde_json::from_slice(&body).expect("the body is JSON");
        assert_eq!(command["cmd"], "SET_ACTIVITY");
        assert_eq!(command["args"]["activity"]["details"], "Тест");
    }
}

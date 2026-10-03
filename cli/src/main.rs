// SPDX-License-Identifier: GPL-3.0-or-later

//! Debugging probe for the Anixart API.
//!
//! This exists to exercise the API layer against the live service without
//! rebuilding the GUI (Slint + Skia make that cycle slow). Diagnostics go to
//! stderr and data to stdout, so output can be piped into `jq`.

mod i18n;

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};

use anirust_api::{Client, EpisodeSort, ProfileList, SearchBy};
use anirust_extract::{Registry, ResolvedStream, StreamKind, StreamVariant};
use anirust_player::{
    MediaSource, Player, PlayerConfig, UpscaleMode, UpscalePreset, UpscaleQuality,
};

use crate::i18n::Lang;

#[derive(Parser)]
#[command(
    name = "anirust-cli",
    about = "Probe the Anixart API",
    version,
    disable_help_subcommand = true
)]
struct Cli {
    /// Auth token. Also read from ANIRUST_TOKEN.
    #[arg(long, global = true, env = "ANIRUST_TOKEN")]
    token: Option<String>,

    /// Override the API base URL.
    #[arg(long, global = true)]
    base_url: Option<String>,

    /// Verbose request logging.
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Emit raw JSON instead of a human-readable summary.
    #[arg(long, global = true)]
    json: bool,

    /// Output language. Detected from the locale when not given.
    #[arg(long, global = true, value_enum)]
    lang: Option<Lang>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Search releases by title, studio, director, author or genre.
    Search {
        query: String,
        #[arg(long, value_enum, default_value_t = SearchField::Title)]
        by: SearchField,
        #[arg(long, default_value_t = 0)]
        page: i32,
    },
    /// Fetch one release.
    Release {
        id: i64,
        /// Ask the server to inline related and recommended releases.
        #[arg(long)]
        extended: bool,
    },
    /// List voice-over tracks for a release.
    Dubbers { release_id: i64 },
    /// List hosts serving a dubber.
    Sources { release_id: i64, dubber_id: i64 },
    /// List episodes for a dubber/source pair.
    Episodes {
        release_id: i64,
        dubber_id: i64,
        source_id: i64,
        #[arg(long)]
        desc: bool,
    },
    /// Walk the whole playback chain and report how many episodes need an
    /// extractor, broken down by host. This is the measurement the extractor
    /// work is prioritised from.
    Chain {
        release_id: i64,
        /// Inspect every dubber rather than just the first.
        #[arg(long)]
        all_dubbers: bool,
    },
    /// Sign in and print a token.
    Login {
        login: String,
        /// Read the password from this env var instead of prompting.
        #[arg(long, env = "ANIRUST_PASSWORD", hide_env_values = true)]
        password: Option<String>,
    },
    /// A profile. Defaults to whoever `ANIRUST_PROFILE` names.
    Me {
        #[arg(env = "ANIRUST_PROFILE")]
        id: i64,
    },
    /// One of the signed-in user's lists.
    List {
        #[arg(value_enum)]
        which: ListKind,
        #[arg(long, default_value_t = 0)]
        page: i32,
    },
    /// Watch history.
    History {
        #[arg(long, default_value_t = 0)]
        page: i32,
    },
    /// What people are watching right now.
    Watching {
        #[arg(long, default_value_t = 0)]
        page: i32,
    },
    /// Send any request by path and print the body as it came.
    ///
    /// For capturing responses as fixtures, redirected into
    /// `api/tests/fixtures/`. The token goes along when one is set; nothing is
    /// decoded.
    Raw {
        /// GET or POST.
        method: String,
        /// Relative to the API root, e.g. `profile/5`.
        path: String,
        /// Query parameters, `key=value`, repeatable.
        #[arg(long = "query", short = 'q', value_parser = key_value)]
        query: Vec<(String, String)>,
    },
    /// List the hosts an extractor is implemented for.
    Hosts,
    /// Resolve an embed URL to a directly playable stream.
    Resolve {
        url: String,
        #[command(flatten)]
        output: StreamOutput,
    },
    /// Open a stream with the real player, headless, and report what mpv
    /// makes of it. Exercises the player crate without needing a window.
    Play {
        release_id: i64,
        /// Episode number, 1-based.
        #[arg(default_value_t = 1)]
        position: i32,
        #[arg(long)]
        dubber: Option<i64>,
        #[arg(long)]
        source: Option<i64>,
        /// Seconds to keep decoding once playback starts.
        #[arg(long, default_value_t = 5)]
        seconds: u64,
        /// Playback speed to exercise.
        #[arg(long, default_value_t = 1.0)]
        speed: f64,
        /// Directory holding the shaders, instead of the ones built in.
        #[arg(long, env = "ANIRUST_SHADER_DIR")]
        shader_dir: Option<std::path::PathBuf>,
        /// Upscaling recipe: one of Anime4K's modes.
        #[arg(long, value_enum, default_value_t = Upscale::Off)]
        upscale: Upscale,
        /// How big a network runs the recipe.
        #[arg(long, value_enum, default_value_t = Quality::High)]
        quality: Quality,
        /// Enable temporal interpolation.
        #[arg(long)]
        interpolation: bool,
    },
    /// Walk the API chain to one episode and resolve it in one step.
    ///
    /// Without --dubber/--source the first of each is used, which is the
    /// most-viewed voice-over.
    Stream {
        release_id: i64,
        /// Episode number, 1-based.
        #[arg(default_value_t = 1)]
        position: i32,
        #[arg(long)]
        dubber: Option<i64>,
        #[arg(long)]
        source: Option<i64>,
        #[command(flatten)]
        output: StreamOutput,
    },
    /// Save an episode to a file.
    ///
    /// HLS streams are fetched several segments at a time and remuxed with
    /// ffmpeg; direct links are written straight to disk.
    Download {
        release_id: i64,
        #[arg(default_value_t = 1)]
        position: i32,
        #[arg(long)]
        dubber: Option<i64>,
        #[arg(long)]
        source: Option<i64>,
        /// Where to write it. Defaults to a name built from the release.
        #[arg(long, short)]
        out: Option<std::path::PathBuf>,
        /// Highest rendition to take, as a height. The best available is used
        /// when this is left out or nothing is that small.
        #[arg(long)]
        quality: Option<u32>,
    },
}

/// How to print a resolved stream. Shared by `resolve` and `stream`.
#[derive(clap::Args)]
#[group(multiple = false)]
struct StreamOutput {
    /// Print only the best rendition's URL, for piping.
    #[arg(long)]
    best: bool,
    /// Print a ready-to-run mpv command, headers included.
    #[arg(long)]
    mpv: bool,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum Upscale {
    Off,
    A,
    B,
    C,
    #[value(name = "a+a")]
    AA,
    #[value(name = "b+b")]
    BB,
    #[value(name = "c+a")]
    CA,
}

impl From<Upscale> for UpscaleMode {
    fn from(u: Upscale) -> Self {
        match u {
            Upscale::Off => Self::Off,
            Upscale::A => Self::A,
            Upscale::B => Self::B,
            Upscale::C => Self::C,
            Upscale::AA => Self::AA,
            Upscale::BB => Self::BB,
            Upscale::CA => Self::CA,
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum Quality {
    Fast,
    High,
    Max,
    Ultra,
}

impl From<Quality> for UpscaleQuality {
    fn from(q: Quality) -> Self {
        match q {
            Quality::Fast => Self::Fast,
            Quality::High => Self::High,
            Quality::Max => Self::Max,
            Quality::Ultra => Self::Ultra,
        }
    }
}

#[derive(Copy, Clone, ValueEnum)]
enum SearchField {
    Title,
    Studio,
    Director,
    Author,
    Genre,
}

impl From<SearchField> for SearchBy {
    fn from(f: SearchField) -> Self {
        match f {
            SearchField::Title => SearchBy::Title,
            SearchField::Studio => SearchBy::Studio,
            SearchField::Director => SearchBy::Director,
            SearchField::Author => SearchBy::Author,
            SearchField::Genre => SearchBy::Genre,
        }
    }
}

#[derive(Copy, Clone, ValueEnum)]
enum ListKind {
    Watching,
    Planned,
    Watched,
    HoldOn,
    Dropped,
}

impl From<ListKind> for ProfileList {
    fn from(k: ListKind) -> Self {
        match k {
            ListKind::Watching => ProfileList::Watching,
            ListKind::Planned => ProfileList::Planned,
            ListKind::Watched => ProfileList::Watched,
            ListKind::HoldOn => ProfileList::HoldOn,
            ListKind::Dropped => ProfileList::Dropped,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(if cli.verbose { "debug" } else { "warn" }.parse()?)
                .from_env_lossy(),
        )
        // Diagnostics on stderr keeps stdout pipeable into jq.
        .with_writer(std::io::stderr)
        .without_time()
        .init();

    let mut builder = Client::builder();
    if let Some(url) = &cli.base_url {
        builder = builder.base_urls([url.clone()]);
    }
    if let Some(t) = &cli.token {
        builder = builder.token(t.clone());
    }
    let client = builder.build().context("building the API client")?;

    let lang = cli.lang.unwrap_or_else(Lang::from_env);
    run(&client, &cli, lang).await
}

/// `key=value`, for `raw --query`.
fn key_value(text: &str) -> std::result::Result<(String, String), String> {
    text.split_once('=')
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .ok_or_else(|| format!("expected key=value, got `{text}`"))
}

async fn run(client: &Client, cli: &Cli, lang: Lang) -> Result<()> {
    match &cli.command {
        Command::Raw {
            method,
            path,
            query,
        } => {
            let method: reqwest::Method = method
                .to_ascii_uppercase()
                .parse()
                .context("the method must be GET or POST")?;
            println!("{}", client.raw(method, path, query).await?);
        }
        Command::Search { query, by, page } => {
            let hits = client.search_releases(query, (*by).into(), *page).await?;
            if cli.json {
                print_json(&hits)?;
            } else if hits.is_empty() {
                eprintln!("{}", lang.nothing_found());
            } else {
                for r in &hits {
                    println!(
                        "{:>8}  {:<52} {:>4}  {:>4.1}  {}/{}",
                        r.id,
                        truncate(r.title(), 52),
                        r.year,
                        r.score(),
                        r.episodes_released,
                        r.episodes_total,
                    );
                }
            }
        }

        Command::Release { id, extended } => {
            let r = client.release(*id, *extended).await?;
            if cli.json {
                print_json(&r)?;
            } else {
                println!("{}", lang.release_header(r.id, r.title()));
                println!("{}", lang.release_original(&r.title_original));
                println!("{}", lang.release_year(&r.year));
                println!("{}", lang.release_status(r.status_name()));
                println!("{}", lang.release_genres(&r.genres));
                println!("{}", lang.release_studio(&r.studio));
                println!(
                    "{}",
                    lang.release_episodes(r.episodes_released, r.episodes_total)
                );
                println!("{}", lang.release_score(r.score(), r.vote_count));
                println!("{}", lang.release_playable(&lang.yes_no(r.is_playable())));
                if !r.is_playable() {
                    println!(
                        "{}",
                        lang.release_block_reasons(
                            r.is_play_disabled,
                            r.is_view_blocked,
                            r.is_deleted
                        )
                    );
                }
            }
        }

        Command::Dubbers { release_id } => {
            let ds = client.dubbers(*release_id).await?;
            if cli.json {
                print_json(&ds)?;
            } else {
                for d in &ds {
                    let kind = if d.is_sub {
                        lang.subtitles()
                    } else {
                        lang.dubbed()
                    };
                    println!(
                        "{}",
                        lang.dubber_row(
                            d.id,
                            truncate(&d.name, 38),
                            d.episodes_count,
                            &kind,
                            d.view_count
                        )
                    );
                }
            }
        }

        Command::Sources {
            release_id,
            dubber_id,
        } => {
            let ss = client.sources(*release_id, *dubber_id).await?;
            if cli.json {
                print_json(&ss)?;
            } else {
                for s in &ss {
                    println!(
                        "{}",
                        lang.source_row(s.id, truncate(&s.name, 28), s.episodes_count, s.quality)
                    );
                }
            }
        }

        Command::Episodes {
            release_id,
            dubber_id,
            source_id,
            desc,
        } => {
            let sort = if *desc {
                EpisodeSort::Descending
            } else {
                EpisodeSort::Ascending
            };
            let eps = client
                .episodes(*release_id, *dubber_id, *source_id, sort)
                .await?;
            if cli.json {
                print_json(&eps)?;
            } else {
                for e in &eps {
                    println!(
                        "{:>4}  {:<7} {:<22} {}",
                        e.position,
                        if e.iframe {
                            lang.embed()
                        } else {
                            lang.direct()
                        },
                        truncate(host_of(&e.url).unwrap_or("-"), 22),
                        e.url
                    );
                }
            }
        }

        Command::Chain {
            release_id,
            all_dubbers,
        } => chain(client, *release_id, *all_dubbers, cli.json, lang).await?,

        Command::Login { login, password } => {
            let password = match password {
                Some(p) => p.clone(),
                None => rpassword::prompt_password(lang.password_prompt(login))
                    .context("reading the password")?,
            };
            match client.sign_in(login, &password).await {
                Ok((profile, token)) => {
                    eprintln!("{}", lang.signed_in(&profile.login, profile.id));
                    // The token alone on stdout, so it can be captured:
                    //   export ANIRUST_TOKEN=$(anirust-cli login me)
                    println!("{}", token.token);
                }
                Err(e) => bail!("{}", lang.sign_in_failed(&e)),
            }
        }

        Command::Me { id } => {
            let p = client.profile(*id).await?;
            if cli.json {
                print_json(&p)?;
            } else {
                println!("{}", lang.release_header(p.id, &p.login));
                println!("{}", lang.profile_watching(p.watching_count));
                println!("{}", lang.profile_planned(p.plan_count));
                println!("{}", lang.profile_watched(p.completed_count));
                println!("{}", lang.profile_on_hold(p.hold_on_count));
                println!("{}", lang.profile_dropped(p.dropped_count));
            }
        }

        Command::List { which, page } => {
            let p = client.profile_list((*which).into(), *page, None).await?;
            print_release_page(&p, cli.json, lang)?;
        }

        Command::History { page } => {
            let p = client.history(*page).await?;
            print_release_page(&p, cli.json, lang)?;
        }

        Command::Watching { page } => {
            let p = client.discover_watching(*page).await?;
            print_release_page(&p, cli.json, lang)?;
        }

        Command::Hosts => {
            for host in Registry::new(reqwest::Client::new()).supported_hosts() {
                println!("{host}");
            }
        }

        Command::Play {
            release_id,
            position,
            dubber,
            source,
            seconds,
            speed,
            shader_dir,
            upscale,
            quality,
            interpolation,
        } => {
            let episode =
                pick_episode(client, *release_id, *dubber, *source, *position, lang).await?;
            let stream = resolve_episode(&episode, lang).await?;
            play(
                &stream,
                *seconds,
                *speed,
                shader_dir.as_deref(),
                UpscalePreset::new((*upscale).into(), (*quality).into()),
                *interpolation,
                lang,
            )?;
        }

        Command::Resolve { url, output } => {
            let registry = Registry::new(reqwest::Client::new());
            let stream = registry.resolve(url).await?;
            print_stream(&stream, output, cli.json, lang)?;
        }

        Command::Stream {
            release_id,
            position,
            dubber,
            source,
            output,
        } => {
            let episode =
                pick_episode(client, *release_id, *dubber, *source, *position, lang).await?;
            let stream = resolve_episode(&episode, lang).await?;
            print_stream(&stream, output, cli.json, lang)?;
        }

        Command::Download {
            release_id,
            position,
            dubber,
            source,
            out,
            quality,
        } => {
            let wanted = Wanted {
                release_id: *release_id,
                position: *position,
                dubber: *dubber,
                source: *source,
                out: out.clone(),
                quality: *quality,
            };
            download(client, &wanted, lang).await?;
        }
    }

    Ok(())
}

/// Which episode to save, and how.
struct Wanted {
    release_id: i64,
    position: i32,
    dubber: Option<i64>,
    source: Option<i64>,
    out: Option<std::path::PathBuf>,
    quality: Option<u32>,
}

/// Resolves an episode and writes it to a file.
async fn download(client: &Client, wanted: &Wanted, lang: Lang) -> Result<()> {
    let Wanted {
        release_id,
        position,
        dubber,
        source,
        out,
        quality,
    } = wanted;
    let (release_id, position) = (*release_id, *position);
    use std::io::Write;

    // Looked up before anything is fetched: a missing ffmpeg is worth knowing
    // about now rather than after several hundred megabytes.
    let ffmpeg = anirust_download::Ffmpeg::find()?;

    let episode = pick_episode(client, release_id, *dubber, *source, position, lang).await?;
    let mut stream = resolve_episode(&episode, lang).await?;

    // The downloader takes the first rendition, so asking for a lower one is a
    // matter of putting it first.
    if let Some(height) = *quality
        && let Some(chosen) = stream.at_most(height).cloned()
    {
        stream.variants.retain(|v| v.url != chosen.url);
        stream.variants.insert(0, chosen);
    }

    let destination = match out {
        Some(path) => path.clone(),
        None => {
            let release = client.release(release_id, false).await?;
            std::path::PathBuf::from(anirust_download::file_name(release.title(), position, ""))
        }
    };

    eprintln!("{}", lang.downloading(&destination.display().to_string()));

    let downloader = anirust_download::Download::new(reqwest::Client::new(), ffmpeg);
    let mut last = 0;
    downloader
        .save(&stream, &destination, |progress| {
            // Redrawn in place rather than a line per segment: several hundred
            // lines of progress is not progress.
            if let Some(fraction) = progress.fraction() {
                let percent = (fraction * 100.0) as u32;
                if percent != last {
                    last = percent;
                    eprint!("\r  {percent:3}%");
                    let _ = std::io::stderr().flush();
                }
            }
        })
        .await?;
    eprintln!();

    println!("{}", destination.display());
    Ok(())
}

/// Walks dubbers → sources → episodes and returns the requested episode.
async fn pick_episode(
    client: &Client,
    release_id: i64,
    dubber: Option<i64>,
    source: Option<i64>,
    position: i32,
    lang: Lang,
) -> Result<anirust_api::Episode> {
    let dubber_id = match dubber {
        Some(id) => id,
        None => {
            let dubbers = client.dubbers(release_id).await?;
            let first = dubbers.first().with_context(|| lang.err_no_dubbers())?;
            eprintln!("{}", lang.dubber_chosen(&first.name, first.id));
            first.id
        }
    };

    let source_id = match source {
        Some(id) => id,
        None => {
            let sources = client.sources(release_id, dubber_id).await?;
            let first = sources.first().with_context(|| lang.err_no_sources())?;
            eprintln!("{}", lang.source_chosen(&first.name, first.id));
            first.id
        }
    };

    let episodes = client
        .episodes(release_id, dubber_id, source_id, EpisodeSort::Ascending)
        .await?;

    episodes
        .into_iter()
        .find(|e| e.position == position)
        .with_context(|| lang.err_no_episode(position))
}

/// Turns an episode into a playable stream.
///
/// The API's `iframe` flag is not trustworthy: Sibnet and AniLibria episodes
/// arrive with `iframe = false` even though their URLs are player pages
/// answering text/html. Routing by host instead lets the flag be wrong without
/// breaking playback, and the disagreement is reported rather than hidden.
async fn resolve_episode(episode: &anirust_api::Episode, lang: Lang) -> Result<ResolvedStream> {
    let kind = if episode.iframe {
        lang.embed()
    } else {
        lang.direct()
    };
    eprintln!(
        "{}",
        lang.episode_line(episode.position, &episode.url, &kind)
    );

    let registry = Registry::new(reqwest::Client::new());
    if registry.supports(&episode.url) {
        if !episode.iframe {
            eprintln!("{}", lang.note_flag_says_direct());
        }
        return Ok(registry.resolve(&episode.url).await?);
    }

    if episode.iframe {
        eprintln!("{}", lang.warn_no_extractor());
    }
    Ok(ResolvedStream {
        variants: vec![StreamVariant {
            height: episode.quality.max(0) as u32,
            url: episode.url.clone(),
            kind: StreamKind::classify(None, &episode.url),
        }],
        ..Default::default()
    })
}

/// Opens a stream with the real player, headless, and reports what mpv makes
/// of it.
///
/// This is how the player crate gets exercised before a window exists: it
/// drives the same code path the GUI will, minus the rendering.
fn play(
    stream: &ResolvedStream,
    seconds: u64,
    speed: f64,
    shader_dir: Option<&std::path::Path>,
    upscale: UpscalePreset,
    interpolation: bool,
    lang: Lang,
) -> Result<()> {
    let best = stream.best().with_context(|| lang.err_nothing_resolved())?;

    // The player carries the shaders and writes them out on startup, so a
    // preset only has to be checked when the caller overrode the directory.
    let mut preset = upscale;
    if preset.mode != UpscaleMode::Off
        && let Some(dir) = shader_dir
    {
        let missing = UpscalePreset::missing_from(dir);
        if !missing.is_empty() {
            eprintln!(
                "{}",
                lang.play_shaders_missing(missing.len(), &dir.display().to_string())
            );
            preset = UpscalePreset::OFF;
        }
    }

    let player = Player::new(&PlayerConfig {
        upscale: preset,
        interpolation,
        shader_dir: shader_dir.map(std::path::Path::to_path_buf),
        ..PlayerConfig::headless()
    })?;

    eprintln!("{}", lang.play_opening(&best.url));
    player.open(
        &MediaSource::new(&best.url)
            .headers(stream.headers.iter().map(|(k, v)| (k.as_str(), v.as_str()))),
    )?;
    player.set_speed(speed)?;

    // Poll rather than subscribe to mpv events: the probe only needs to know
    // that decoding started, and polling keeps this free of an event loop.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let size = loop {
        if let Some(size) = player.video_size() {
            break Some(size);
        }
        if std::time::Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };

    let Some((width, height)) = size else {
        bail!("{}", lang.play_timed_out(30));
    };
    println!("{}", lang.play_ready(width, height));
    if let Some(duration) = player.duration() {
        println!("{}", lang.play_duration(duration.as_secs_f64()));
    }
    println!("{}", lang.play_speed(player.speed()?));

    std::thread::sleep(std::time::Duration::from_secs(seconds));
    if let Some(position) = player.position() {
        println!("{}", lang.play_position(position.as_secs_f64()));
    }

    println!("state: {:?}", player.state());
    if let Some(ahead) = player.buffered_ahead() {
        println!("buffered ahead: {:.1}s", ahead.as_secs_f64());
    }

    let tracks = player.tracks();
    println!("tracks: {}", tracks.len());
    for track in &tracks {
        println!(
            "  {:?}{} id={} {}",
            track.kind,
            if track.selected { " *" } else { "  " },
            track.id,
            track.label()
        );
    }

    // No published boundaries for this source, so this exercises the fallback.
    let before = player.position().unwrap_or_default();
    player.skip_opening(None)?;
    std::thread::sleep(std::time::Duration::from_millis(500));
    if let Some(after) = player.position() {
        println!(
            "skip opening: {:.1}s -> {:.1}s",
            before.as_secs_f64(),
            after.as_secs_f64()
        );
    }

    player.stop()?;
    Ok(())
}

fn print_stream(
    stream: &ResolvedStream,
    output: &StreamOutput,
    json: bool,
    lang: Lang,
) -> Result<()> {
    let best = stream.best().with_context(|| lang.err_nothing_resolved())?;

    if output.best {
        println!("{}", best.url);
        return Ok(());
    }

    if output.mpv {
        let mut parts = vec!["mpv".to_owned()];
        parts.extend(stream.mpv_args().iter().map(|a| shell_quote(a)));
        parts.push(shell_quote(&best.url));
        println!("{}", parts.join(" "));
        return Ok(());
    }

    if json {
        return print_json(&serde_json::json!({
            "variants": stream.variants.iter().map(|v| serde_json::json!({
                "height": v.height,
                "url": v.url,
                "kind": format!("{:?}", v.kind),
            })).collect::<Vec<_>>(),
            "headers": stream.headers,
        }));
    }

    for v in &stream.variants {
        // A height of zero means the host did not advertise a resolution.
        let quality = if v.height == anirust_extract::UNKNOWN_HEIGHT {
            "?".to_owned()
        } else {
            format!("{}p", v.height)
        };
        println!("{:>6}  {:<12} {}", quality, format!("{:?}", v.kind), v.url);
    }
    if let Some(op) = stream.opening {
        eprintln!(
            "{}",
            lang.opening_range(op.start, op.end, op.duration_secs())
        );
    }
    if !stream.headers.is_empty() {
        eprintln!("{}", lang.headers_required());
        for (name, value) in &stream.headers {
            eprintln!("  {name}: {value}");
        }
    }
    Ok(())
}

/// Single-quotes a value for a POSIX shell, so the printed mpv command can be
/// pasted as-is.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Walks dubbers → sources → episodes and tallies how many episodes are direct
/// versus embeds, grouped by host.
async fn chain(
    client: &Client,
    release_id: i64,
    all_dubbers: bool,
    json: bool,
    lang: Lang,
) -> Result<()> {
    let release = client.release(release_id, false).await?;
    eprintln!("{}", lang.chain_release(release.id, release.title()));
    if !release.is_playable() {
        eprintln!("{}", lang.chain_not_playable());
    }

    let dubbers = client.dubbers(release_id).await?;
    eprintln!("{}", lang.chain_dubber_count(dubbers.len()));

    let chosen = if all_dubbers {
        &dubbers[..]
    } else {
        &dubbers[..dubbers.len().min(1)]
    };

    let mut direct = 0usize;
    let mut embed = 0usize;
    let mut by_host: BTreeMap<String, usize> = BTreeMap::new();
    let mut rows = Vec::new();

    for d in chosen {
        let sources = client.sources(release_id, d.id).await?;
        eprintln!("{}", lang.chain_dubber(d.id, &d.name, sources.len()));

        for s in &sources {
            let eps = client
                .episodes(release_id, d.id, s.id, EpisodeSort::Ascending)
                .await?;
            eprintln!("{}", lang.chain_source(s.id, &s.name, eps.len()));

            for e in &eps {
                let host = host_of(&e.url).unwrap_or("—").to_owned();
                if e.iframe {
                    embed += 1;
                } else {
                    direct += 1;
                }
                *by_host.entry(host.clone()).or_default() += 1;
                rows.push(serde_json::json!({
                    "dubber": d.name,
                    "source": s.name,
                    "position": e.position,
                    "iframe": e.iframe,
                    "host": host,
                    "url": e.url,
                }));
            }
        }
    }

    if json {
        print_json(&serde_json::json!({
            "release_id": release_id,
            "direct": direct,
            "embed": embed,
            "by_host": by_host,
            "episodes": rows,
        }))?;
        return Ok(());
    }

    let total = direct + embed;
    println!();
    println!("{}", lang.chain_total(total));
    if total > 0 {
        println!(
            "{}",
            lang.chain_direct(direct, direct as f64 * 100.0 / total as f64)
        );
        println!(
            "{}",
            lang.chain_embeds(embed, embed as f64 * 100.0 / total as f64)
        );
    }
    println!();
    println!("{}", lang.chain_by_host());
    for (host, n) in &by_host {
        println!("  {:<28} {n}", host);
    }

    Ok(())
}

fn print_release_page(
    page: &anirust_api::Page<anirust_api::Release>,
    json: bool,
    lang: Lang,
) -> Result<()> {
    if json {
        return print_json(&page.content);
    }
    for r in &page.content {
        println!(
            "{:>8}  {:<52} {:>4}  {}/{}",
            r.id,
            truncate(r.title(), 52),
            r.year,
            r.episodes_released,
            r.episodes_total
        );
    }
    eprintln!(
        "{}",
        lang.page_info(
            page.current_page + 1,
            page.total_page_count,
            page.total_count
        )
    );
    Ok(())
}

fn print_json<T: serde::Serialize>(v: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

/// Host portion of a URL, for the embed histogram.
fn host_of(url: &str) -> Option<&str> {
    let rest = url.split_once("//").map(|(_, r)| r).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit_once('@').map(|(_, h)| h).unwrap_or(host);
    let host = host.split_once(':').map(|(h, _)| h).unwrap_or(host);
    (!host.is_empty()).then_some(host)
}

fn truncate(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

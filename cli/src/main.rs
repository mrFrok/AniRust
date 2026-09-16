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
    /// The signed-in user's profile.
    Me,
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
    /// List the hosts an extractor is implemented for.
    Hosts,
    /// Resolve an embed URL to a directly playable stream.
    Resolve {
        url: String,
        #[command(flatten)]
        output: StreamOutput,
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

async fn run(client: &Client, cli: &Cli, lang: Lang) -> Result<()> {
    match &cli.command {
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

        Command::Me => {
            let p = client.my_profile().await?;
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
            let kind = if episode.iframe {
                lang.embed()
            } else {
                lang.direct()
            };
            eprintln!(
                "{}",
                lang.episode_line(episode.position, &episode.url, &kind)
            );

            // The API's `iframe` flag is not trustworthy: Sibnet and
            // AniLibria episodes arrive with `iframe = false` even though
            // their URLs are player pages answering text/html. Routing by
            // host instead means the flag can be wrong without breaking
            // playback.
            let registry = Registry::new(reqwest::Client::new());
            let stream = if registry.supports(&episode.url) {
                if !episode.iframe {
                    eprintln!("{}", lang.note_flag_says_direct());
                }
                registry.resolve(&episode.url).await?
            } else {
                if episode.iframe {
                    eprintln!("{}", lang.warn_no_extractor());
                }
                ResolvedStream {
                    variants: vec![StreamVariant {
                        height: episode.quality.max(0) as u32,
                        url: episode.url.clone(),
                        kind: StreamKind::classify(None, &episode.url),
                    }],
                    ..Default::default()
                }
            };
            print_stream(&stream, output, cli.json, lang)?;
        }
    }

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

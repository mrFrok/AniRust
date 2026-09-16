// SPDX-License-Identifier: GPL-3.0-or-later

//! Debugging probe for the Anixart API.
//!
//! This exists to exercise the API layer against the live service without
//! rebuilding the GUI (Slint + Skia make that cycle slow). Diagnostics go to
//! stderr and data to stdout, so output can be piped into `jq`.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};

use anirust_api::{Client, EpisodeSort, ProfileList, SearchBy};

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

    run(&client, &cli).await
}

async fn run(client: &Client, cli: &Cli) -> Result<()> {
    match &cli.command {
        Command::Search { query, by, page } => {
            let hits = client.search_releases(query, (*by).into(), *page).await?;
            if cli.json {
                print_json(&hits)?;
            } else if hits.is_empty() {
                eprintln!("ничего не найдено");
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
                println!("{} — {}", r.id, r.title());
                println!("  оригинал   {}", r.title_original);
                println!("  год        {}", r.year);
                println!("  статус     {}", r.status_name());
                println!("  жанры      {}", r.genres);
                println!("  студия     {}", r.studio);
                println!("  серии      {}/{}", r.episodes_released, r.episodes_total);
                println!("  оценка     {:.1} ({} голосов)", r.score(), r.vote_count);
                println!("  играбелен  {}", yes_no(r.is_playable()));
                if !r.is_playable() {
                    println!(
                        "    play_disabled={} view_blocked={} deleted={}",
                        r.is_play_disabled, r.is_view_blocked, r.is_deleted
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
                    println!(
                        "{:>8}  {:<38} серий {:>4}  {}  просмотров {}",
                        d.id,
                        truncate(&d.name, 38),
                        d.episodes_count,
                        if d.is_sub { "сабы" } else { "озв. " },
                        d.view_count,
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
                        "{:>8}  {:<28} серий {:>4}  качество {}",
                        s.id,
                        truncate(&s.name, 28),
                        s.episodes_count,
                        s.quality
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
                        if e.iframe { "embed" } else { "direct" },
                        truncate(host_of(&e.url).unwrap_or("—"), 22),
                        e.url
                    );
                }
            }
        }

        Command::Chain {
            release_id,
            all_dubbers,
        } => chain(client, *release_id, *all_dubbers, cli.json).await?,

        Command::Login { login, password } => {
            let password = match password {
                Some(p) => p.clone(),
                None => rpassword::prompt_password(format!("пароль для {login}: "))
                    .context("reading the password")?,
            };
            match client.sign_in(login, &password).await {
                Ok((profile, token)) => {
                    eprintln!("вошли как {} (id {})", profile.login, profile.id);
                    // The token alone on stdout, so it can be captured:
                    //   export ANIRUST_TOKEN=$(anirust-cli login me)
                    println!("{}", token.token);
                }
                Err(e) => bail!("вход не удался: {e}"),
            }
        }

        Command::Me => {
            let p = client.my_profile().await?;
            if cli.json {
                print_json(&p)?;
            } else {
                println!("{} — {}", p.id, p.login);
                println!("  смотрю     {}", p.watching_count);
                println!("  в планах   {}", p.plan_count);
                println!("  просмотрено {}", p.completed_count);
                println!("  отложено   {}", p.hold_on_count);
                println!("  брошено    {}", p.dropped_count);
            }
        }

        Command::List { which, page } => {
            let p = client.profile_list((*which).into(), *page, None).await?;
            print_release_page(&p, cli.json)?;
        }

        Command::History { page } => {
            let p = client.history(*page).await?;
            print_release_page(&p, cli.json)?;
        }

        Command::Watching { page } => {
            let p = client.discover_watching(*page).await?;
            print_release_page(&p, cli.json)?;
        }
    }

    Ok(())
}

/// Walks dubbers → sources → episodes and tallies how many episodes are direct
/// versus embeds, grouped by host.
async fn chain(client: &Client, release_id: i64, all_dubbers: bool, json: bool) -> Result<()> {
    let release = client.release(release_id, false).await?;
    eprintln!("релиз {} — {}", release.id, release.title());
    if !release.is_playable() {
        eprintln!("  внимание: сервер помечает релиз как недоступный для воспроизведения");
    }

    let dubbers = client.dubbers(release_id).await?;
    eprintln!("озвучек: {}", dubbers.len());

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
        eprintln!(
            "  озвучка {} ({}): источников {}",
            d.id,
            d.name,
            sources.len()
        );

        for s in &sources {
            let eps = client
                .episodes(release_id, d.id, s.id, EpisodeSort::Ascending)
                .await?;
            eprintln!("    источник {} ({}): серий {}", s.id, s.name, eps.len());

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
    println!("всего серий: {total}");
    if total > 0 {
        println!(
            "  прямых ссылок: {direct} ({:.0}%)",
            direct as f64 * 100.0 / total as f64
        );
        println!(
            "  эмбедов:       {embed} ({:.0}%)  — нужен экстрактор",
            embed as f64 * 100.0 / total as f64
        );
    }
    println!();
    println!("по хостам:");
    for (host, n) in &by_host {
        println!("  {:<28} {n}", host);
    }

    Ok(())
}

fn print_release_page(page: &anirust_api::Page<anirust_api::Release>, json: bool) -> Result<()> {
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
        "страница {} из {} (всего {})",
        page.current_page + 1,
        page.total_page_count,
        page.total_count
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

fn yes_no(b: bool) -> &'static str {
    if b { "да" } else { "нет" }
}

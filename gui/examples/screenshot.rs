// SPDX-License-Identifier: GPL-3.0-or-later

//! Renders the interface to a PNG without a display.
//!
//! Layout mistakes are cheap to make and expensive to spot by description: a
//! hidden element that still holds its place, a column that collapses, a row
//! whose glyphs sit on three different centre lines. All of those are obvious
//! in a picture and invisible in the source. This draws the real components
//! with the software renderer so they can be looked at from a terminal, in CI,
//! or over SSH.
//!
//! The video surface stays empty — it is a borrowed GL texture, and there is no
//! GL here. Everything around it is the real thing.
//!
//! ```text
//! cargo run -p anirust-gui --example screenshot -- out.png [width height] [state]
//! ```
//!
//! `state` is `home`, `release` (default), `playing`, or `theatre`. Narrow is
//! a width, not a state: pass one below 900.

use std::rc::Rc;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{PhysicalSize, PlatformError};

slint::include_modules!();

/// A platform with no windowing system behind it.
struct Headless {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "screenshot.png".to_owned());
    let width: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(1440);
    let height: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(900);
    let state = args.next().unwrap_or_else(|| "release".to_owned());

    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless {
        window: window.clone(),
    }))?;

    let ui = MainWindow::new()?;
    populate(&ui);
    populate_home(&ui);
    ui.set_screen(if state.starts_with("home") || state == "sign-in" {
        "home".into()
    } else {
        "release".into()
    });
    if state == "home-signed-in" {
        ui.set_signed_in(true);
        ui.set_account_name("mrfrok".into());
    }
    if state == "failed" {
        ui.set_episode_failed(true);
    }
    if state == "sign-in" {
        ui.set_show_sign_in(true);
        ui.set_login("mrfrok".into());
        ui.set_password("hunter2".into());
        ui.set_sign_in_error("wrong-password".into());
    }
    window.set_size(PhysicalSize::new(width, height));

    let mut pixels = vec![slint::Rgb8Pixel { r: 0, g: 0, b: 0 }; (width * height) as usize];
    let mut draw = |window: &MinimalSoftwareWindow| {
        slint::platform::update_timers_and_animations();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, width as usize);
        });
        window.request_redraw();
    };

    // The first pass is where `init` runs, the breakpoint is measured and the
    // list learns its own height. Only then does starting an episode mean
    // anything — scrolling to it needs a list that has been laid out.
    draw(&window);
    if state == "playing" || state == "theatre" {
        start_playing(&ui, state == "theatre");
    }
    draw(&window);
    draw(&window);

    let mut buffer = image::RgbImage::new(width, height);
    for (pixel, out) in pixels.iter().zip(buffer.pixels_mut()) {
        *out = image::Rgb([pixel.r, pixel.g, pixel.b]);
    }
    buffer.save(&path)?;

    println!("wrote {path} ({width}x{height}, {state})");
    Ok(())
}

/// Fills the window with a release that exercises the cases worth looking at:
/// a long description, several voice-overs, episodes both watched and
/// part-watched, and one with a title of its own.
fn populate(ui: &MainWindow) {
    ui.set_lang("ru".into());
    ui.set_release_title("Демоны старшей школы".into());
    ui.set_release_original_title("High School DxD".into());
    ui.set_release_year("2012".into());
    ui.set_release_genres("комедия, романтика, сверхъестественное, экшен, этти, гарем".into());
    ui.set_release_studio("TNK".into());
    ui.set_release_status("Вышел".into());
    ui.set_release_score("4.6".into());
    ui.set_release_episodes_label("24/24".into());
    ui.set_release_description(
        "Что нужно от жизни простому 17-летнему японскому школьнику? Иссэй Хёдо отлично \
         знает ответ, ведь ради этого он и записался в бывшую женскую академию Куо! Хёдо \
         наивно полагал, что после начала совместного обучения в условиях дефицита парней \
         станет королем и познает весну жизни, однако идет второй год."
            .into(),
    );
    ui.set_release_loading(false);

    ui.set_dubbers(slint::ModelRc::new(slint::VecModel::from(vec![
        option("AniLibria", 14, false),
        option("AniDUB", 12, false),
        option("SHIZA Project", 12, false),
        option("Субтитры", 12, true),
    ])));
    ui.set_sources(slint::ModelRc::new(slint::VecModel::from(vec![
        option("Kodik", 12, false),
        option("Libria", 12, false),
    ])));

    // Long enough that the list has to scroll, which is the case worth
    // looking at: resuming episode 13 should not start the list at episode 1.
    let episodes: Vec<EpisodeItem> = (1..=24)
        .map(|position| EpisodeItem {
            position,
            name: if position == 7 {
                "Рождение".into()
            } else {
                "".into()
            },
            watched: position < 13,
            resume_at: if position == 13 {
                "8:21".into()
            } else {
                "".into()
            },
            filler: position == 9,
        })
        .collect();
    ui.set_episodes(slint::ModelRc::new(slint::VecModel::from(episodes)));
    ui.set_resume_episode(13);
}

/// Puts an episode in the player, as clicking one would.
fn start_playing(ui: &MainWindow, theatre: bool) {
    {
        ui.set_playing(true);
        ui.set_current_episode(13);
        ui.set_current_index(12);
        ui.set_episode_label("13 - AniLibria".into());
        ui.set_state("playing".into());
        ui.set_position_text("8:21".into());
        ui.set_duration_text("23:40".into());
        ui.set_progress(0.353);
        ui.set_buffered(0.48);
        ui.set_quality_label("720p → 1440p".into());
        ui.set_decoder_label("NVDEC".into());
        ui.set_upscale(2);
        ui.set_has_skip(true);
        ui.set_has_next(true);
        ui.set_has_previous(true);
        ui.set_qualities(slint::ModelRc::new(slint::VecModel::from(vec![
            slint::SharedString::from("1080p"),
            slint::SharedString::from("720p"),
            slint::SharedString::from("480p"),
        ])));
        ui.set_theatre(theatre);
    }
}

/// A grid of results, so the browsing screen can be looked at too.
fn populate_home(ui: &MainWindow) {
    let titles = [
        ("Демоны старшей школы", "2012 · 12/12", "4.6"),
        ("Стальной алхимик: Братство", "2009 · 64/64", "4.9"),
        ("Магическая битва", "2020 · 24/24", "4.8"),
        ("Клинок, рассекающий демонов", "2019 · 26/26", "4.8"),
        ("Атака титанов", "2013 · 25/25", "4.9"),
        ("Ван-Пис", "1999 · 1122", "4.7"),
        ("Наруто: Ураганные хроники", "2007 · 500/500", "4.5"),
        ("Токийский гуль", "2014 · 12/12", "4.4"),
        ("Код Гиас: Восставший Лелуш", "2006 · 25/25", "4.8"),
        ("Тетрадь смерти", "2006 · 37/37", "4.9"),
        ("Re:Zero", "2016 · 25/25", "4.7"),
        ("Доктор Стоун", "2019 · 24/24", "4.6"),
    ];

    let cards: Vec<ReleaseCard> = titles
        .iter()
        .map(|(title, subtitle, score)| ReleaseCard {
            title: (*title).into(),
            subtitle: (*subtitle).into(),
            score: (*score).into(),
            poster: slint::Image::default(),
            poster_loaded: false,
        })
        .collect();

    ui.set_results(slint::ModelRc::new(slint::VecModel::from(cards)));
}

fn option(label: &str, episodes: i32, is_sub: bool) -> PickerOption {
    PickerOption {
        label: label.into(),
        episodes,
        is_sub,
    }
}

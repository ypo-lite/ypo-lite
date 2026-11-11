//! Draws the pictures of the README and the manual that show the HUD, with
//! y-coustic's own painter (`y_coustic::paint`): no window, no screen capture,
//! nothing on the desktop. The same code draws the settings app's preview, so
//! these are what y-coustic draws.
//!
//! ```text
//! cargo run -p y-coustic --example hud_pictures            # writes to y-coustic/docs/images
//! cargo run -p y-coustic --example hud_pictures -- <folder>
//! ```
//!
//! It writes `y-coustic_hud_01.png`, `y-coustic_styles_01.png`,
//! `y-coustic_colors_01.png`, `y-coustic_themes_01.png` and
//! `y-coustic_hud_mic_01.png`: the ones of `docs/screenshots.md` that need no
//! running application. Pictures of the mixer, the tray, the settings app and
//! anything that moves still have to be taken by hand.

use std::path::PathBuf;

use y_core::overlay::Color;
use y_coustic::config::{HudConfig, Style};
use y_coustic::model::{Kind, State};
use y_coustic::paint::{Rendered, render};

/// The room around and between pictures, in pixels.
const GAP: i32 = 12;

/// What the pictures are laid on: close to the dark theme's panel, so the
/// rounded corners read.
const BACKGROUND: Color = Color::rgb(0x18, 0x18, 0x18);

fn state(level: f32, muted: bool) -> Option<State> {
    Some(State { level, muted })
}

fn draw(hud: &HudConfig, kind: Kind, state: Option<State>, dark: bool) -> Rendered {
    render(hud, kind, state, None, dark, 96).expect("a HUD draws into memory")
}

/// `rows` of pictures on `background`: each row centred, `GAP` around everything.
fn stitch(rows: &[Vec<Rendered>], background: Color) -> (i32, i32, Vec<u8>) {
    let row_width = |row: &Vec<Rendered>| {
        row.iter().map(|picture| picture.width).sum::<i32>() + GAP * (row.len() as i32 - 1)
    };
    let row_height = |row: &Vec<Rendered>| row.iter().map(|picture| picture.height).max().unwrap_or(0);
    let width = rows.iter().map(row_width).max().unwrap_or(0) + 2 * GAP;
    let height = rows.iter().map(row_height).sum::<i32>() + GAP * (rows.len() as i32 + 1);

    let mut rgb = vec![[background.r, background.g, background.b]; (width * height) as usize];
    let mut top = GAP;
    for row in rows {
        let mut left = (width - row_width(row)) / 2;
        let tallest = row_height(row);
        for picture in row {
            let offset = (tallest - picture.height) / 2;
            for y in 0..picture.height {
                for x in 0..picture.width {
                    let word = picture.pixels[(y * picture.width + x) as usize];
                    let alpha = (word >> 24) & 0xFF;
                    let at = ((top + offset + y) * width + left + x) as usize;
                    // Premultiplied over opaque: source + destination * (1 - alpha).
                    for (channel, shift) in [(0, 16), (1, 8), (2, 0)] {
                        let source = (word >> shift) & 0xFF;
                        let below = u32::from(rgb[at][channel]);
                        rgb[at][channel] = (source + below * (255 - alpha) / 255).min(255) as u8;
                    }
                }
            }
            left += picture.width + GAP;
        }
        top += tallest + GAP;
    }

    let mut rgba = Vec::with_capacity(rgb.len() * 4);
    for pixel in rgb {
        rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
    }
    (width, height, rgba)
}

fn write(folder: &std::path::Path, name: &str, rows: &[Vec<Rendered>], background: Color) {
    let (width, height, rgba) = stitch(rows, background);
    let png = y_core::png::encode_rgba(width as u32, height as u32, &rgba);
    assert!(!png.is_empty(), "{name}: the PNG encoder refused");
    let path = folder.join(name);
    std::fs::write(&path, png).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    println!("{} ({width} x {height})", path.display());
}

fn main() {
    let folder = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("y-coustic/docs/images"), PathBuf::from);
    std::fs::create_dir_all(&folder).expect("the folder can be made");
    let plain = HudConfig::default();

    // README, first table: the bar on the master, as it shows after a key.
    write(
        &folder,
        "y-coustic_hud_01.png",
        &[vec![draw(&plain, Kind::Master, state(0.4, false), true)]],
        BACKGROUND,
    );

    // The five styles on the same volume.
    let styles = [Style::Bar, Style::Compact, Style::Text, Style::VBar, Style::Ring]
        .map(|style| {
            let hud = HudConfig { style, ..HudConfig::default() };
            draw(&hud, Kind::Master, state(0.6, false), true)
        })
        .into_iter()
        .collect();
    write(&folder, "y-coustic_styles_01.png", &[styles], BACKGROUND);

    // Colours per kind: the master, a live microphone and an application in three
    // accents, and the master muted.
    let mut coloured = HudConfig::default();
    coloured.colors.all.accent = Some(Color::rgb(0x3F, 0xB9, 0x50));
    coloured.colors.microphone.accent = Some(Color::rgb(0xD9, 0x53, 0x4F));
    coloured.colors.application.accent = Some(Color::rgb(0xE8, 0xA3, 0x3D));
    write(
        &folder,
        "y-coustic_colors_01.png",
        &[
            vec![
                draw(&coloured, Kind::Master, state(0.6, false), true),
                draw(&coloured, Kind::Mic, state(0.7, false), true),
            ],
            vec![
                draw(&coloured, Kind::App, state(0.5, false), true),
                draw(&coloured, Kind::Master, state(0.6, true), true),
            ],
        ],
        BACKGROUND,
    );

    // The same HUD in both themes.
    write(
        &folder,
        "y-coustic_themes_01.png",
        &[
            vec![draw(&plain, Kind::Master, state(0.6, false), false)],
            vec![draw(&plain, Kind::Master, state(0.6, false), true)],
        ],
        Color::rgb(0x80, 0x80, 0x80),
    );

    // The microphone: live (red) above, muted below.
    write(
        &folder,
        "y-coustic_hud_mic_01.png",
        &[
            vec![draw(&plain, Kind::Mic, state(0.7, false), true)],
            vec![draw(&plain, Kind::Mic, state(0.7, true), true)],
        ],
        BACKGROUND,
    );

}

//! Website radio and hero title, with a small COHDL project credit on black.
use super::font::{self, Align, Font};
use crate::display::{Band, BAND_H, WIDTH};
use embedded_graphics::{
    pixelcolor::{raw::RawU16, Rgb565},
    prelude::*,
};

#[path = "splash_asset.rs"]
mod asset;

pub const DURATION_MS: u32 = 1200;

pub fn draw(target: &mut Band) {
    target.fill(Rgb565::BLACK);

    draw_radio(target, (WIDTH as i32 - 120) / 2, 58, 10);

    let left = (WIDTH - asset::TITLE_WIDTH) / 2;
    let top = 232;
    let first = top.max(target.y0());
    let last = (top + asset::TITLE_HEIGHT).min(target.y0() + BAND_H);
    for y in first..last {
        // The same warm cream and gold as the website's hero heading.
        let color = if y - top < asset::TITLE_ACCENT_Y {
            0xfffaec
        } else {
            0xf5c53d
        };
        for x in 0..asset::TITLE_WIDTH {
            let alpha = asset::TITLE[((y - top) * asset::TITLE_WIDTH + x) as usize];
            target.blend_px((left + x) as i32, y as i32, color, alpha);
        }
    }

    draw_credit(target, WIDTH as i32 / 2, 408, 0xa8a59c);
}

/// Integer-scaled radio shared by the splash and About screen.
pub fn draw_radio(target: &mut Band, left: i32, top: u16, scale: u16) {
    // Integer enlargement preserves the website radio's crisp pixel edges.
    let first = top.max(target.y0());
    let last = (top + asset::RADIO_HEIGHT * scale).min(target.y0() + BAND_H);
    let _ = target.draw_iter((first..last).flat_map(|y| {
        (0..asset::RADIO_WIDTH * scale).map(move |x| {
            let offset = (((y - top) / scale * asset::RADIO_WIDTH + x / scale) * 2) as usize;
            let color = u16::from_le_bytes([asset::RADIO[offset], asset::RADIO[offset + 1]]);
            Pixel(
                Point::new(left + x as i32, y as i32),
                Rgb565::from(RawU16::new(color)),
            )
        })
    }));
}

/// The small COHDL symbol and project credit, centered as a single line.
pub fn draw_credit(target: &mut Band, center_x: i32, top: u16, color: u32) {
    const CREDIT: &str = "A COHDL PROJECT";

    let text_width = font::measure(Font::Hint, CREDIT);
    let credit_width = asset::COHDL_WIDTH as i32 + 12 + text_width;
    let left = center_x - credit_width / 2;
    let first = top.max(target.y0());
    let last = (top + asset::COHDL_HEIGHT).min(target.y0() + BAND_H);
    for y in first..last {
        for x in 0..asset::COHDL_WIDTH {
            let alpha = asset::COHDL[((y - top) * asset::COHDL_WIDTH + x) as usize];
            target.blend_px(left + x as i32, y as i32, color, alpha);
        }
    }
    font::draw(
        target,
        Font::Hint,
        left + asset::COHDL_WIDTH as i32 + 12,
        top as i32 + 20,
        CREDIT,
        color,
        None,
        text_width,
        Align::Left,
    );
}

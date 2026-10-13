//! The supplied CoHDL logo, centered on a black OLED background.
use crate::display::{Band, BAND_H, HEIGHT, WIDTH};
use embedded_graphics::{pixelcolor::Rgb565, prelude::*};

#[path = "splash_asset.rs"]
mod asset;

pub const DURATION_MS: u32 = 1200;

pub fn draw(target: &mut Band) {
    target.fill(Rgb565::BLACK);
    let left = (WIDTH - asset::WIDTH) / 2;
    let top = (HEIGHT - asset::HEIGHT) / 2;
    let first = top.max(target.y0());
    let last = (top + asset::HEIGHT).min(target.y0() + BAND_H);
    let _ = target.draw_iter((first..last).flat_map(|y| {
        (0..asset::WIDTH).map(move |x| {
            let a = asset::ALPHA[((y - top) * asset::WIDTH + x) as usize];
            Pixel(
                Point::new((left + x) as i32, y as i32),
                Rgb565::new(a >> 3, a >> 2, a >> 3),
            )
        })
    }));
}

//! Hardware-independent band framebuffer shared by both firmware runtimes.

use embedded_graphics::{
    pixelcolor::{raw::RawU16, Rgb565},
    prelude::*,
    primitives::Rectangle,
};

pub use crate::orientation::{HEIGHT, WIDTH};

/// Rows rendered per pass. A full 480x480 RGB565 frame is 450KB against 511KB
/// of RAM, so the screen is drawn in horizontal bands instead: 48 rows is 45KB,
/// and ten of them tile the panel exactly.
///
/// The CO5300 requires even window coordinates, which every multiple of 48 is.
pub const BAND_H: u16 = 48;
pub const BAND_BYTES: usize = WIDTH as usize * BAND_H as usize * 2;
pub const BANDS: u16 = HEIGHT / BAND_H;

/// One horizontal band of the screen, and an `embedded_graphics` draw target
/// for it.
///
/// Drawing uses *screen* coordinates, not band-local ones: the band knows which
/// rows it covers and discards everything else, so the UI is written once
/// against a 480x480 canvas and simply drawn ten times.
///
/// A face sliding along the strip is drawn through an x origin: every drawing
/// operation and `bounding_box()` are in the face's own logical coordinates,
/// `origin_x` to the right of the panel's. Only `orient_for_panel` and the DMA
/// see absolute positions, so faces never learn the offset.
///
/// `repr(align(4))` is load-bearing -- the LCDC DMAs straight out of this array
/// and asserts 4-byte alignment.
#[repr(align(4))]
pub struct Band {
    pixels: [u8; BAND_BYTES],
    y0: u16,
    /// Absolute panel coordinates, whatever the origin.
    clip: Option<Rectangle>,
    origin_x: i32,
}

/// A clip and origin to put back after drawing one face of a slide.
#[derive(Clone, Copy)]
pub struct Window {
    clip: Option<Rectangle>,
    origin_x: i32,
}

impl Band {
    pub const fn new() -> Self {
        Self {
            pixels: [0; BAND_BYTES],
            y0: 0,
            clip: None,
            origin_x: 0,
        }
    }

    pub fn set_row(&mut self, band: u16) {
        self.y0 = band * BAND_H;
        self.clip = None;
        self.origin_x = 0;
    }

    /// Limit drawing to a rectangle (logical coordinates) inside the current
    /// clip: a damaged region, or the window of one face during a slide.
    pub fn clip_to(&mut self, area: Rectangle) {
        let area = Rectangle::new(area.top_left + Point::new(self.origin_x, 0), area.size);
        self.clip = Some(self.clip_abs().intersection(&area));
    }

    /// Panel x of logical x 0 for everything drawn from now on.
    pub fn set_origin_x(&mut self, origin_x: i32) {
        self.origin_x = origin_x;
    }

    pub fn origin_x(&self) -> i32 {
        self.origin_x
    }

    pub fn window(&self) -> Window {
        Window {
            clip: self.clip,
            origin_x: self.origin_x,
        }
    }

    pub fn restore(&mut self, window: Window) {
        self.clip = window.clip;
        self.origin_x = window.origin_x;
    }

    fn clip_abs(&self) -> Rectangle {
        self.clip.unwrap_or(Rectangle::new(
            Point::new(0, self.y0 as i32),
            Size::new(WIDTH as u32, BAND_H as u32),
        ))
    }

    /// Byte offset of an absolute pixel the clip has already admitted.
    fn offset(&self, x: i32, y: i32) -> usize {
        ((y - self.y0 as i32) as usize * WIDTH as usize + x as usize) * 2
    }

    pub fn y0(&self) -> u16 {
        self.y0
    }

    pub fn data(&self) -> &[u8] {
        &self.pixels
    }

    /// Rotate the freshly drawn rectangle for the inverted panel mounting.
    /// Keep its source offset and 480-pixel stride; only the destination window
    /// is mirrored by the caller. Call once after drawing, before starting DMA,
    /// and never mutate this band again until that transfer has completed.
    pub fn orient_for_panel(&mut self) {
        if !crate::orientation::ROTATE_180 {
            return;
        }
        let area = self.clip_abs();
        let width = area.size.width as usize;
        let height = area.size.height as usize;
        if width == 0 || height == 0 {
            return;
        }
        let first = (area.top_left.y as usize - self.y0 as usize) * WIDTH as usize
            + area.top_left.x as usize;
        for pixel in 0..width * height / 2 {
            let opposite = width * height - 1 - pixel;
            let a = (first + pixel / width * WIDTH as usize + pixel % width) * 2;
            let b = (first + opposite / width * WIDTH as usize + opposite % width) * 2;
            // Reverse pixels, not bytes: RGB565 remains native-endian for DMA.
            self.pixels.swap(a, b);
            self.pixels.swap(a + 1, b + 1);
        }
    }

    /// Flood the band with one colour.
    pub fn fill(&mut self, color: Rgb565) {
        let clip = self.bounding_box();
        let _ = self.fill_solid(&clip, color);
    }

    /// The part of logical row `y` between `x0` and `x1` that lies inside
    /// `bounding_box()`, as native-endian RGB565 byte pairs; `None` when
    /// nothing of it does. Clamp `x0..x1` to `bounding_box()` first and the
    /// slice starts at `x0`.
    #[allow(dead_code)] // For the glyph blitter's span writes.
    pub fn row_mut(&mut self, y: i32, x0: i32, x1: i32) -> Option<&mut [u8]> {
        let clip = self.bounding_box();
        if y < clip.top_left.y || y >= clip.top_left.y + clip.size.height as i32 {
            return None;
        }
        let left = x0.max(clip.top_left.x);
        let right = x1.min(clip.top_left.x + clip.size.width as i32);
        if left >= right {
            return None;
        }
        let start = self.offset(left + self.origin_x, y);
        Some(&mut self.pixels[start..start + (right - left) as usize * 2])
    }

    /// Blend one RGB888 colour over the pixel already there; `alpha` 255 is
    /// opaque. Outside the clip nothing happens.
    pub fn blend_px(&mut self, x: i32, y: i32, rgb888: u32, alpha: u8) {
        if alpha == 0 || !self.bounding_box().contains(Point::new(x, y)) {
            return;
        }
        let offset = self.offset(x + self.origin_x, y);
        let raw = u16::from_le_bytes([self.pixels[offset], self.pixels[offset + 1]]) as u32;
        let under = ((raw >> 11) * 255 / 31) << 16
            | (((raw >> 5) & 63) * 255 / 63) << 8
            | (raw & 31) * 255 / 31;
        let t = alpha as u32;
        let channel = |shift: u32| {
            (((under >> shift) & 255) * (255 - t) + ((rgb888 >> shift) & 255) * t) / 255
        };
        let color: Rgb565 =
            embedded_graphics::pixelcolor::Rgb888::new(channel(16) as u8, channel(8) as u8, channel(0) as u8)
                .into();
        let raw = RawU16::from(color).into_inner();
        self.pixels[offset..offset + 2].copy_from_slice(&raw.to_le_bytes());
    }
}

fn fill_pattern(pixels: &mut [u8], pattern: &[u8; 8]) {
    let mut blocks = pixels.chunks_exact_mut(8);
    for block in &mut blocks {
        block.copy_from_slice(pattern);
    }
    let tail = blocks.into_remainder();
    tail.copy_from_slice(&pattern[..tail.len()]);
}

impl Dimensions for Band {
    /// The clip in logical coordinates: what a face may usefully draw.
    fn bounding_box(&self) -> Rectangle {
        let clip = self.clip_abs();
        Rectangle::new(clip.top_left - Point::new(self.origin_x, 0), clip.size)
    }
}

impl DrawTarget for Band {
    type Color = Rgb565;
    type Error = core::convert::Infallible;

    fn fill_solid(&mut self, area: &Rectangle, color: Rgb565) -> Result<(), Self::Error> {
        let clipped = area.intersection(&self.bounding_box());
        if clipped.size.width == 0 || clipped.size.height == 0 {
            return Ok(());
        }
        let raw = RawU16::from(color).into_inner();
        let [lo, hi] = raw.to_le_bytes();
        let pattern = [lo, hi, lo, hi, lo, hi, lo, hi];
        for y in clipped.top_left.y..clipped.top_left.y + clipped.size.height as i32 {
            let start = self.offset(clipped.top_left.x + self.origin_x, y);
            let end = start + clipped.size.width as usize * 2;
            fill_pattern(&mut self.pixels[start..end], &pattern);
        }
        Ok(())
    }

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        let clip = self.bounding_box();
        let left = clip.top_left.x;
        let right = left + clip.size.width as i32;
        let first = clip.top_left.y;
        let last = first + clip.size.height as i32;
        for Pixel(point, color) in pixels {
            if point.x < left || point.x >= right || point.y < first || point.y >= last {
                continue;
            }
            let index = self.offset(point.x + self.origin_x, point.y);
            let raw = RawU16::from(color).into_inner();
            // The LCDC's layer DMA reads RGB565 as 16-bit words out of memory,
            // so these are stored native-endian, not in wire order.
            self.pixels[index] = raw as u8;
            self.pixels[index + 1] = (raw >> 8) as u8;
        }
        Ok(())
    }
}

//! A single on-demand preview, using 16.5 KiB rather than a full framebuffer.
//! The device pulls one chunk at a time; PTT pauses the next request.
use super::{
    font::{self, Align, Font},
    kit,
    layout::Rect,
    state::{Target, TouchAct, UiState, Update},
    theme,
};
use crate::{
    control::{Button, Control, Edge},
    deck::msg::{Event, HostMsg},
    display::Band,
};
use embedded_graphics::{
    pixelcolor::{raw::RawU16, Rgb565},
    prelude::*,
};

const CAP: usize = 512 + 128 * 128;
const CLOSE: Rect = Rect::new(24, 410, 432, 48);
#[derive(Clone, Copy)]
pub struct View {
    pub open: bool,
    agent: u8,
    rev: u16,
    index: u8,
    received: u16,
    total: u16,
    width: u16,
    height: u16,
    asked_at: Option<u32>,
    retries: u8,
    failed: bool,
    pixels: [u8; CAP],
}
impl View {
    pub const fn new() -> Self {
        Self {
            open: false,
            agent: 0,
            rev: 0,
            index: 0,
            received: 0,
            total: 0,
            width: 0,
            height: 0,
            asked_at: None,
            retries: 0,
            failed: false,
            pixels: [0; CAP],
        }
    }
}
pub fn open(ui: &mut UiState, index: u8, rev: u16, at: u32, out: &mut Update) {
    let v = &mut ui.image;
    v.open = true;
    v.agent = ui.agents.agent;
    v.rev = rev;
    v.index = index;
    v.received = 0;
    v.total = 0;
    v.width = 0;
    v.height = 0;
    v.asked_at = None;
    v.retries = 0;
    v.failed = false;
    tick(ui, at, out);
    out.dirty = true;
}
pub fn tick(ui: &mut UiState, at: u32, out: &mut Update) {
    if !ui.image.open {
        return;
    }
    if ui.image.agent != ui.agents.agent
        || ui
            .deck
            .agent(ui.image.agent)
            .is_none_or(|a| a.reply_rev != ui.image.rev)
    {
        ui.image.open = false;
        out.dirty = true;
        return;
    }
    if !ui.deck.host_present {
        if !ui.image.failed {
            ui.image.failed = true;
            out.dirty = true;
        }
        return;
    }
    if ui.global.ptt.is_some() {
        return;
    }
    let v = &mut ui.image;
    if v.failed || (v.total > 0 && v.received == v.total) {
        return;
    }
    if v.asked_at.is_some_and(|t| at.wrapping_sub(t) < 6000) {
        return;
    }
    if v.retries >= 3 {
        v.failed = true;
        out.dirty = true;
        return;
    }
    out.push_deck(Event::Image {
        agent: v.agent,
        rev: v.rev,
        index: v.index,
        offset: v.received,
    });
    v.asked_at = Some(at);
    v.retries += 1;
}
pub fn host(ui: &mut UiState, msg: &HostMsg, out: &mut Update) {
    if matches!(msg, HostMsg::Synced | HostMsg::Hello { .. }) {
        ui.image.open = false;
        return;
    }
    let HostMsg::Image {
        agent,
        rev,
        index,
        offset,
        total,
        width,
        height,
        len,
        data,
    } = msg
    else {
        return;
    };
    let v = &mut ui.image;
    if !v.open || *agent != v.agent || *rev != v.rev || *index != v.index || *offset != v.received {
        return;
    }
    let expected = 512usize + usize::from(*width) * usize::from(*height);
    let end = usize::from(*offset) + usize::from(*len);
    if *width == 0
        || *height == 0
        || *width > 128
        || *height > 128
        || usize::from(*total) != expected
        || end > expected
        || *len == 0
        || *len > 384
        || (*offset != 0 && (*total != v.total || *width != v.width || *height != v.height))
        || (end < expected && *len != 384)
    {
        v.failed = true;
        out.dirty = true;
        return;
    }
    v.pixels[usize::from(*offset)..end].copy_from_slice(&data[..usize::from(*len)]);
    v.received = end as u16;
    v.total = *total;
    v.width = *width;
    v.height = *height;
    v.asked_at = None;
    v.retries = 0;
    out.dirty = true;
}
pub fn control(ui: &mut UiState, control: Control, out: &mut Update) -> bool {
    if !ui.image.open {
        return false;
    }
    if matches!(control, Control::Button(Button::Cancel, Edge::Press)) {
        ui.image.open = false;
        out.dirty = true;
    }
    true
}
pub fn hit(x: i32, y: i32) -> Option<Target> {
    CLOSE.contains(x, y).then(|| Target::tap(CLOSE, 0))
}
pub fn touch(ui: &mut UiState, act: TouchAct, out: &mut Update) {
    if matches!(act, TouchAct::Activate) {
        ui.image.open = false;
        out.dirty = true;
    }
}
fn text(band: &mut Band, y: i32, value: &str) {
    font::draw(
        band,
        Font::Body,
        240,
        y,
        value,
        theme::TEXT_PRIMARY,
        Some(theme::CANVAS),
        416,
        Align::Center,
    );
}
pub fn draw(band: &mut Band, ui: &UiState) {
    let v = &ui.image;
    band.fill(kit::rgb(theme::CANVAS));
    kit::header(band, "Image", kit::header_right(ui, kit::HeaderRight::None));
    if v.total > 0 && v.received == v.total && !v.failed {
        let scale = (384 / i32::from(v.width))
            .min(320 / i32::from(v.height))
            .max(1);
        let w = i32::from(v.width) * scale;
        let h = i32::from(v.height) * scale;
        let x0 = (480 - w) / 2;
        let y0 = 72 + (320 - h) / 2;
        let rect = embedded_graphics::primitives::Rectangle::new(
            Point::new(x0, y0),
            Size::new(w as u32, h as u32),
        );
        // Only visit this display band's pixels, not the entire image per band.
        let rect = rect.intersection(&band.bounding_box());
        let _ = band.draw_iter(rect.points().map(|p| {
            let i = ((p.y - y0) / scale) as usize * usize::from(v.width)
                + ((p.x - x0) / scale) as usize;
            let palette = usize::from(v.pixels[512 + i]) * 2;
            let rgb = u16::from_le_bytes([v.pixels[palette], v.pixels[palette + 1]]);
            Pixel(p, Rgb565::from(RawU16::new(rgb)))
        }));
    } else if v.failed {
        text(band, 220, "Preview unavailable");
        text(band, 258, "Close and tap the image to retry.");
    } else {
        use core::fmt::Write;
        let mut label = crate::deck::text::Text::<48>::new();
        if v.total > 0 {
            let _ = write!(
                label,
                "Loading image… {}%",
                u32::from(v.received) * 100 / u32::from(v.total)
            );
        } else {
            label.set("Loading image…");
        }
        text(band, 240, label.as_str());
    }
    text(band, 442, "Close · CANCEL");
}
pub fn signature(ui: &UiState, sig: &mut super::signature::SigBuilder) {
    sig.mode(0x494d47);
    sig.region_with(crate::ui::faces::agents::HEADER_LANE, |h| {
        h.header_right(kit::header_right(ui, kit::HeaderRight::None));
    });
    sig.region_with(Rect::new(0, 60, 480, 420), |h| {
        let v = &ui.image;
        h.u8(v.agent)
            .u16(v.rev)
            .u8(v.index)
            .u16(v.received)
            .u16(v.total)
            .bool(v.failed);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chunks_are_ordered_and_bound_to_the_view() {
        let mut ui = UiState::new();
        let mut out = Update::new();
        ui.image.open = true;
        ui.image.agent = 1;
        ui.image.rev = 3;
        let chunk = |rev, offset, len| HostMsg::Image {
            agent: 1,
            rev,
            index: 0,
            offset,
            total: 513,
            width: 1,
            height: 1,
            len,
            data: [0; 384],
        };
        host(&mut ui, &chunk(2, 0, 384), &mut out);
        assert_eq!(ui.image.received, 0);
        host(&mut ui, &chunk(3, 384, 129), &mut out);
        assert_eq!(ui.image.received, 0);
        host(&mut ui, &chunk(3, 0, 384), &mut out);
        assert_eq!(ui.image.received, 384);
        host(&mut ui, &chunk(3, 384, 129), &mut out);
        assert_eq!(ui.image.received, 513);
        assert!(!ui.image.failed);
    }
    #[test]
    fn transfer_times_out_and_cancel_stops_requests() {
        let mut ui = UiState::new();
        ui.reset();
        ui.deck.host_present = true;
        ui.agents.agent = 1;
        ui.deck.agents[0].present = true;
        ui.deck.agents[0].reply_rev = 3;
        let mut out = Update::new();
        open(&mut ui, 0, 3, 0, &mut out);
        assert_eq!(ui.image.retries, 1);
        tick(&mut ui, 5999, &mut out);
        assert_eq!(ui.image.retries, 1);
        tick(&mut ui, 6000, &mut out);
        tick(&mut ui, 12000, &mut out);
        tick(&mut ui, 18000, &mut out);
        assert!(ui.image.failed);
        assert!(control(
            &mut ui,
            Control::Button(Button::Cancel, Edge::Press),
            &mut out
        ));
        assert!(!ui.image.open);
        let retries = ui.image.retries;
        tick(&mut ui, 30000, &mut out);
        assert_eq!(ui.image.retries, retries);
    }
}

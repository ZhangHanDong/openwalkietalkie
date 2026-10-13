//! What a frame looks like, without its pixels: a mode plus a list of screen
//! regions, each with a hash of exactly the inputs its pixels depend on. Two
//! signatures tell `Damage` what to redraw; the previous `UiState` is never
//! needed. About 600 bytes: keep them in statics and fill them in place.
//!
//! Rules (see `damage.rs`): region lists are matched by index, so a face must
//! register the same regions in the same order for as long as its mode is the
//! same, using `Rect::EMPTY` for an element that is currently absent. Anything
//! that changes the list itself belongs in the mode.
use super::kit::HeaderRight;
use super::layout::Rect;
use super::state::UiState;
use crate::deck::text::Text;

pub const MAX_REGIONS: usize = 48;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Signature {
    mode: u32,
    count: u8,
    overflow: bool,
    regions: [(Rect, u32); MAX_REGIONS],
}

impl Signature {
    pub const fn new() -> Self {
        Self {
            mode: 0,
            count: 0,
            overflow: false,
            regions: [(Rect::EMPTY, 0); MAX_REGIONS],
        }
    }
    pub fn clear(&mut self) {
        self.mode = 0;
        self.count = 0;
        self.overflow = false;
    }
    pub fn mode(&self) -> u32 {
        self.mode
    }
    /// More regions were registered than fit: the frame is redrawn whole.
    pub fn overflowed(&self) -> bool {
        self.overflow
    }
    /// Absolute panel rectangles, clipped to the panel.
    pub fn regions(&self) -> &[(Rect, u32)] {
        &self.regions[..self.count as usize]
    }
    /// In-place copy for `static` to `static`; a by-value move would cost the
    /// stack as much again.
    pub fn copy_into(&self, dst: &mut Signature) {
        // SAFETY: both are valid, distinct (`&` vs `&mut`) and `Copy`.
        unsafe { core::ptr::copy_nonoverlapping(self, dst, 1) };
    }
}

/// Collects one frame's signature. The scene dispatcher sets the origin of the
/// face being described, so faces register regions in their own coordinates.
pub struct SigBuilder<'a> {
    sig: &'a mut Signature,
    origin_x: i32,
}

impl<'a> SigBuilder<'a> {
    /// Starts from an empty signature.
    pub fn new(sig: &'a mut Signature) -> Self {
        sig.clear();
        Self { sig, origin_x: 0 }
    }
    pub fn set_origin_x(&mut self, origin_x: i32) {
        self.origin_x = origin_x;
    }
    pub fn origin_x(&self) -> i32 {
        self.origin_x
    }
    /// Fold a value into the frame mode: anything that changes WHICH regions
    /// exist (view kind, overlay, slide position). A different mode redraws
    /// the whole frame.
    pub fn mode(&mut self, value: u32) {
        let mut hasher = Hasher::new();
        hasher.u32(self.sig.mode);
        hasher.u32(value);
        self.sig.mode = hasher.finish();
    }
    /// One region in logical face coordinates and the hash of its inputs.
    pub fn region(&mut self, rect: Rect, hash: u32) {
        let Some(slot) = self.sig.regions.get_mut(self.sig.count as usize) else {
            self.sig.overflow = true;
            return;
        };
        // The part of this face that is on the panel right now.
        let face = Rect::SCREEN.intersection(&rect).offset(self.origin_x, 0);
        *slot = (Rect::SCREEN.intersection(&face), hash);
        self.sig.count += 1;
    }
    pub fn region_with(&mut self, rect: Rect, inputs: impl FnOnce(&mut Hasher)) {
        let mut hasher = Hasher::new();
        inputs(&mut hasher);
        self.region(rect, hasher.finish());
    }
    /// The correct-by-construction default of a face or overlay: its whole
    /// area, hashed by `revision`, which every dirty update bumps.
    pub fn fallback(&mut self, ui: &UiState) {
        self.region(Rect::SCREEN, ui.revision);
    }
}

/// FNV-1a, 32 bit.
#[derive(Clone, Copy)]
pub struct Hasher(u32);

impl Hasher {
    pub const fn new() -> Self {
        Self(0x811C_9DC5)
    }
    pub fn bytes(&mut self, bytes: &[u8]) -> &mut Self {
        for &byte in bytes {
            self.0 = (self.0 ^ byte as u32).wrapping_mul(0x0100_0193);
        }
        self
    }
    pub fn u8(&mut self, value: u8) -> &mut Self {
        self.bytes(&[value])
    }
    pub fn bool(&mut self, value: bool) -> &mut Self {
        self.bytes(&[value as u8])
    }
    pub fn u16(&mut self, value: u16) -> &mut Self {
        self.bytes(&value.to_le_bytes())
    }
    pub fn u32(&mut self, value: u32) -> &mut Self {
        self.bytes(&value.to_le_bytes())
    }
    pub fn i32(&mut self, value: i32) -> &mut Self {
        self.bytes(&value.to_le_bytes())
    }
    /// `None` and `Some(0)` differ.
    pub fn opt_u32(&mut self, value: Option<u32>) -> &mut Self {
        self.bool(value.is_some()).u32(value.unwrap_or(0))
    }
    /// Length-prefixed, so adjacent strings cannot trade characters.
    pub fn str(&mut self, value: &str) -> &mut Self {
        self.u16(value.len() as u16).bytes(value.as_bytes())
    }
    pub fn text<const N: usize>(&mut self, value: &Text<N>) -> &mut Self {
        self.str(value.as_str())
    }
    /// A header's right side, as resolved by `kit::header_right`/`face_tag`.
    pub fn header_right(&mut self, value: HeaderRight) -> &mut Self {
        match value {
            HeaderRight::None => self.u8(0),
            HeaderRight::Text(text) => self.u8(1).str(text),
            HeaderRight::Clock(minute) => self.u8(2).u16(minute),
            HeaderRight::Battery(percent) => self.u8(3).u8(percent),
        }
    }
    pub fn rect(&mut self, value: Rect) -> &mut Self {
        self.u16(value.x as u16)
            .u16(value.y as u16)
            .u16(value.w as u16)
            .u16(value.h as u16)
    }
    pub fn finish(&self) -> u32 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_stay_small_enough_for_three_statics() {
        assert!(core::mem::size_of::<Signature>() <= 1024);
    }

    #[test]
    fn regions_are_translated_by_the_origin_and_clipped_to_the_panel() {
        let mut sig = Signature::new();
        let mut builder = SigBuilder::new(&mut sig);
        builder.region(Rect::new(32, 70, 416, 74), 1);
        builder.set_origin_x(-100);
        builder.region(Rect::new(32, 70, 416, 74), 2);
        builder.set_origin_x(380);
        builder.region(Rect::new(32, 70, 416, 74), 3);
        builder.region(Rect::new(200, 70, 100, 74), 4);
        builder.region(Rect::EMPTY, 5);
        assert_eq!(
            sig.regions(),
            [
                (Rect::new(32, 70, 416, 74), 1),
                (Rect::new(0, 70, 348, 74), 2),
                (Rect::new(412, 70, 68, 74), 3),
                (Rect::EMPTY, 4),
                (Rect::EMPTY, 5),
            ]
        );
    }

    #[test]
    fn overflow_is_remembered_instead_of_panicking() {
        let mut sig = Signature::new();
        let mut builder = SigBuilder::new(&mut sig);
        for index in 0..MAX_REGIONS as u32 + 5 {
            builder.region(Rect::new(0, 0, 2, 2), index);
        }
        assert!(sig.overflowed());
        assert_eq!(sig.regions().len(), MAX_REGIONS);
        SigBuilder::new(&mut sig);
        assert!(!sig.overflowed() && sig.regions().is_empty());
    }

    #[test]
    fn hashes_separate_adjacent_fields() {
        let hash = |a: &str, b: &str| Hasher::new().str(a).str(b).finish();
        assert_ne!(hash("ab", "c"), hash("a", "bc"));
        assert_ne!(
            Hasher::new().opt_u32(None).finish(),
            Hasher::new().opt_u32(Some(0)).finish()
        );
        let mut a = Signature::new();
        let mut b = Signature::new();
        SigBuilder::new(&mut a).mode(1);
        SigBuilder::new(&mut b).mode(2);
        assert_ne!(a.mode(), b.mode());
        let mut copy = Signature::new();
        a.copy_into(&mut copy);
        assert_eq!(a, copy);
    }
}

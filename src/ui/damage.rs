//! Pixel damage between two frame signatures. Each region fits one band.
use super::signature::Signature;
use crate::display::{BANDS, BAND_H, HEIGHT, WIDTH};

/// Exclusive right/bottom edges; all edges are even for the CO5300 and DMA.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Region {
    pub x0: u16,
    pub y0: u16,
    pub x1: u16,
    pub y1: u16,
}

impl Region {
    /// Destination in the panel's unchanged native address space. Right and
    /// bottom are exclusive, so a 180-degree turn preserves even DMA edges.
    pub fn panel(self) -> Self {
        if crate::orientation::ROTATE_180 {
            Self {
                x0: WIDTH - self.x1,
                y0: HEIGHT - self.y1,
                x1: WIDTH - self.x0,
                y1: HEIGHT - self.y0,
            }
        } else {
            self
        }
    }
}

pub struct Damage {
    regions: [Option<Region>; BANDS as usize],
}

impl Damage {
    pub const fn empty() -> Self {
        Self {
            regions: [None; BANDS as usize],
        }
    }

    pub fn full() -> Self {
        let mut damage = Self::empty();
        damage.add(0, 0, WIDTH as i32, HEIGHT as i32);
        damage
    }

    /// Regions are matched by index: one whose rectangle or hash differs
    /// damages both its old and its new rectangle (a moved element must also
    /// be erased). No previous frame, another mode, another region count or a
    /// builder overflow redraw everything.
    pub fn between(previous: Option<&Signature>, next: &Signature) -> Self {
        let Some(previous) = previous else {
            return Self::full();
        };
        if previous.mode() != next.mode()
            || previous.regions().len() != next.regions().len()
            || previous.overflowed()
            || next.overflowed()
        {
            return Self::full();
        }
        let mut damage = Self::empty();
        for (old, new) in previous.regions().iter().zip(next.regions()) {
            if old != new {
                for rect in [old.0, new.0] {
                    damage.add(rect.left(), rect.top(), rect.width(), rect.height());
                }
            }
        }
        damage
    }

    fn add(&mut self, x: i32, y: i32, width: i32, height: i32) {
        let x0 = (x.clamp(0, WIDTH as i32) as u16) & !1;
        let y0 = (y.clamp(0, HEIGHT as i32) as u16) & !1;
        let x1 = ((x + width).clamp(0, WIDTH as i32) as u16 + 1) & !1;
        let y1 = ((y + height).clamp(0, HEIGHT as i32) as u16 + 1) & !1;
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        for band in y0 / BAND_H..=(y1 - 1) / BAND_H {
            let rect = Region {
                x0,
                y0: y0.max(band * BAND_H),
                x1,
                y1: y1.min((band + 1) * BAND_H),
            };
            let slot = &mut self.regions[band as usize];
            *slot = Some(match *slot {
                Some(old) => Region {
                    x0: old.x0.min(rect.x0),
                    y0: old.y0.min(rect.y0),
                    x1: old.x1.max(rect.x1),
                    y1: old.y1.max(rect.y1),
                },
                None => rect,
            });
        }
    }

    pub fn next(&mut self) -> Option<Region> {
        // Keep transfers in ascending native panel row order after rotation.
        if crate::orientation::ROTATE_180 {
            self.regions.iter_mut().rev().find_map(Option::take)
        } else {
            self.regions.iter_mut().find_map(Option::take)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.regions.iter().all(Option::is_none)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::layout::Rect;
    use crate::ui::signature::SigBuilder;

    fn sig(mode: u32, regions: &[(Rect, u32)]) -> Signature {
        let mut sig = Signature::new();
        let mut builder = SigBuilder::new(&mut sig);
        builder.mode(mode);
        for &(rect, hash) in regions {
            builder.region(rect, hash);
        }
        sig
    }

    fn area(mut damage: Damage) -> (usize, std::vec::Vec<Region>) {
        let mut regions = std::vec::Vec::new();
        while let Some(region) = damage.next() {
            regions.push(region);
        }
        let pixels = regions
            .iter()
            .map(|r| (r.x1 - r.x0) as usize * (r.y1 - r.y0) as usize)
            .sum();
        (pixels, regions)
    }

    const A: Rect = Rect::new(32, 70, 416, 74);
    const B: Rect = Rect::new(32, 147, 416, 74);

    #[test]
    fn identical_signatures_need_nothing_and_no_previous_frame_needs_everything() {
        let one = sig(1, &[(A, 10), (B, 20)]);
        assert!(Damage::between(Some(&one), &one).is_empty());
        assert_eq!(area(Damage::between(None, &one)).0, 480 * 480);
        assert_eq!(area(Damage::full()).0, 480 * 480);
    }

    #[test]
    fn a_changed_hash_damages_only_its_region_split_per_band_with_even_edges() {
        let before = sig(1, &[(A, 10), (B, 20)]);
        let after = sig(1, &[(A, 10), (B, 21)]);
        let (pixels, regions) = area(Damage::between(Some(&before), &after));
        // Rows 147..221 grow to the even edges 146..222.
        assert_eq!(pixels, 416 * 76);
        for region in &regions {
            assert_eq!((region.x0, region.x1), (32, 448));
            assert_eq!((region.x0 | region.y0 | region.x1 | region.y1) & 1, 0);
            assert_eq!(region.y0 / BAND_H, (region.y1 - 1) / BAND_H);
            assert!(region.y0 >= 146 && region.y1 <= 222);
        }
    }

    #[test]
    fn a_moved_region_damages_where_it_was_and_where_it_is() {
        let before = sig(1, &[(Rect::new(48, 0, 384, 20), 5)]);
        let after = sig(1, &[(Rect::new(48, 300, 384, 20), 5)]);
        let (pixels, regions) = area(Damage::between(Some(&before), &after));
        assert_eq!(pixels, 2 * 384 * 20);
        assert_eq!(regions.len(), 2);
        // An element that appears from nothing damages only its new place.
        let hidden = sig(1, &[(Rect::EMPTY, 0)]);
        assert_eq!(area(Damage::between(Some(&hidden), &after)).0, 384 * 20);
    }

    #[test]
    fn mode_count_or_overflow_mismatch_redraws_the_frame() {
        let one = sig(1, &[(A, 10)]);
        for other in [sig(2, &[(A, 10)]), sig(1, &[(A, 10), (B, 20)])] {
            assert_eq!(area(Damage::between(Some(&one), &other)).0, 480 * 480);
            assert_eq!(area(Damage::between(Some(&other), &one)).0, 480 * 480);
        }
        let many = [(A, 1); crate::ui::signature::MAX_REGIONS + 1];
        let overflowed = sig(1, &many);
        assert!(overflowed.overflowed());
        assert_eq!(
            area(Damage::between(Some(&overflowed), &overflowed)).0,
            480 * 480
        );
    }

    #[test]
    fn odd_rectangles_grow_to_even_edges_and_stay_on_the_panel() {
        let before = sig(1, &[(Rect::new(223, 9, 35, 7), 1)]);
        let after = sig(1, &[(Rect::new(223, 9, 35, 7), 2)]);
        let (_, regions) = area(Damage::between(Some(&before), &after));
        assert_eq!(
            regions,
            [Region {
                x0: 222,
                y0: 8,
                x1: 258,
                y1: 16
            }]
        );
    }
}

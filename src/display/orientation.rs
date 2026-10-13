//! Shared per-board build facts: the display and touch orientation and the
//! board's name, selected by the `hw-*` features at build time.

pub const WIDTH: u16 = 480;
pub const HEIGHT: u16 = 480;
/// The custom Rev A hardware (`hw-a`) mounts the panel upside down. The default
/// build is the LilyGo T-Display SF32 development board, which is upright.
/// Evaluated per consumer crate, so every manifest declares the feature. Both
/// runtimes log the result at boot and Settings → About shows it.
pub const ROTATE_180: bool = cfg!(feature = "hw-a");

/// The board this image was built for: `hw-b` (which implies `hw-a`) is the
/// Rev B main board, `hw-a` alone Rev A, neither the dev board. Settings →
/// About shows it and `sys.version` reports it, so tools/flash-usb.py can
/// refuse an image built for another board than the one on the cable.
pub const BOARD: &str = if cfg!(feature = "hw-b") {
    "Rev B"
} else if cfg!(feature = "hw-a") {
    "Rev A"
} else {
    "LilyGo"
};

/// `BOARD` as the number the native image's C side compares with its own
/// board define (`OWT_BOARD_HW_A` / `OWT_BOARD_HW_B`): 0 LilyGo, 1 Rev A,
/// 2 Rev B. Only the native core reads it.
#[allow(dead_code)]
pub const BOARD_CODE: u8 = if cfg!(feature = "hw-b") {
    2
} else if cfg!(feature = "hw-a") {
    1
} else {
    0
};

/// Map between panel coordinates and logical UI coordinates.
/// A half-turn is its own inverse, so display and touch use the same mapping.
pub const fn point(x: u16, y: u16) -> Option<(u16, u16)> {
    point_with(ROTATE_180, x, y)
}

/// `point` for an explicit mounting, so both mappings are tested in every build.
pub const fn point_with(rotate: bool, x: u16, y: u16) -> Option<(u16, u16)> {
    // Reject off-panel samples before subtraction to avoid wrapping them.
    if x >= WIDTH || y >= HEIGHT {
        return None;
    }
    Some(if rotate {
        (WIDTH - 1 - x, HEIGHT - 1 - y)
    } else {
        (x, y)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_turn_maps_all_corners_and_round_trips_points() {
        for (raw, logical) in [
            ((0, 0), (479, 479)),
            ((479, 0), (0, 479)),
            ((0, 479), (479, 0)),
            ((479, 479), (0, 0)),
            ((293, 185), (186, 294)),
        ] {
            assert_eq!(point_with(true, raw.0, raw.1), Some(logical));
            assert_eq!(point_with(true, logical.0, logical.1), Some(raw));
        }
    }

    #[test]
    fn upright_mounting_keeps_every_point_in_place() {
        for (x, y) in [(0, 0), (479, 0), (0, 479), (479, 479), (186, 294)] {
            assert_eq!(point_with(false, x, y), Some((x, y)));
        }
    }

    #[test]
    fn board_feature_selects_the_mounting_used_by_display_and_touch() {
        assert_eq!(ROTATE_180, cfg!(feature = "hw-a"));
        for (x, y) in [(0, 0), (479, 0), (20, 400), (293, 185), (479, 479)] {
            assert_eq!(point(x, y), point_with(ROTATE_180, x, y));
            // Either mounting is its own inverse.
            let (lx, ly) = point(x, y).unwrap();
            assert_eq!(point(lx, ly), Some((x, y)));
        }
    }

    #[test]
    fn board_name_and_code_follow_the_features_and_only_the_dev_board_is_upright() {
        let expected = if cfg!(feature = "hw-b") {
            "Rev B"
        } else if cfg!(feature = "hw-a") {
            "Rev A"
        } else {
            "LilyGo"
        };
        assert_eq!(BOARD, expected);
        let names = ["LilyGo", "Rev A", "Rev B"];
        assert_eq!(names[BOARD_CODE as usize], BOARD);
        assert_eq!(ROTATE_180, BOARD != "LilyGo");
    }

    #[test]
    fn off_panel_points_are_rejected_before_rotation() {
        for rotate in [false, true] {
            for (x, y) in [(480, 0), (0, 480), (480, 480), (u16::MAX, u16::MAX)] {
                assert_eq!(point_with(rotate, x, y), None);
            }
        }
        assert_eq!(point(480, 0), None);
    }
}

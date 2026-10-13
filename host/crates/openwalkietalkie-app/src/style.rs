//! The window's look: one glass material over the blurred desktop, alpha-ink
//! hairlines and washes instead of grey slabs, three text tones, Geist at
//! 11–20 px with Medium (not Bold) for emphasis, and one violet accent kept
//! for focus, switches and "this one is live".
//!
//! The values are Zeron's (github.com/zeronsh/zeron, MIT), resolved from its
//! theme crate's seeds rather than copied from screenshots. Two rules carry
//! most of the look and are worth keeping when a value changes:
//!
//! * **Lines and fills are ink at an alpha, never a grey hex.** A hairline is
//!   white at 10 % on dark and black at 12 % on light; a hover is an 11 % /
//!   6 % wash. Over glass that reads as tinted glass, where a grey would read
//!   as a slab stuck on top.
//! * **Light is designed, not inverted.** Each appearance has its own seeds
//!   (`#303035` text on light, not `#e8e8ea` flipped), and the status colours
//!   step down to their 600-level hues so they keep their contrast on white.
//!
//! Everything a screen paints comes from [`Palette`]. gpui-component's own
//! furniture (text fields, tooltips, notifications) reads its `Theme`
//! instead, so [`install`] writes the same values there: the two cannot
//! drift, and a later `Theme::change` cannot quietly undo them because
//! [`install`] is what runs on every appearance change.
use deck_model::Tone;
use gpui::{px, rgb, rgba, App, Global, Hsla, WindowAppearance};
use gpui_component::{Theme, ThemeMode};
use std::borrow::Cow;

/// The sans family every screen is set in.
pub const SANS: &str = "Geist";
/// The family for anything an operator might copy: paths, ports, tokens,
/// commands, log lines.
pub const MONO: &str = "Geist Mono";

/// Pins the appearance instead of following the system:
/// `OPENWALKIETALKIE_APPEARANCE=light|dark`. For screenshots and for checking
/// one appearance without changing the Mac's.
pub const APPEARANCE_ENV: &str = "OPENWALKIETALKIE_APPEARANCE";

/// `OPENWALKIETALKIE_GLASS=0` paints the shell opaque instead of over the
/// blurred desktop.
pub const GLASS_ENV: &str = "OPENWALKIETALKIE_GLASS";

/// The shell tint's alpha over the blurred desktop.
///
/// Zeron's is 0.80, but Zeron runs a gpui fork that fixed how a translucent
/// fill writes the framebuffer's alpha on a transparent window. Stock 0.2.2
/// blends alpha like colour, so a fill of alpha `a` leaves `a²` behind for
/// the window server: 0.80 lets 36 % of the desktop through instead of 20 %
/// (measured 2026-10-04: the dark shell came out `#303030`, not near-black).
/// 0.894² is 0.80, which puts the look back where Zeron's is.
pub const GLASS_ALPHA: f32 = 0.894;

/// Every colour a screen paints, for one appearance.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub dark: bool,
    /// Whether the window behind the glass is blurred. Without the blur an
    /// 80 % tint shows the desktop through, unblurred, which is worse than
    /// no glass at all, so [`Palette::glass`] is opaque then.
    pub frosted: bool,
    /// The root fill: the shell tint over the blurred desktop.
    pub glass: Hsla,
    pub text: Hsla,
    /// Secondary text, and icons at rest.
    pub muted: Hsla,
    /// Captions, placeholders, the time on a row.
    pub faint: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    /// Rows and chrome under the pointer.
    pub hover: Hsla,
    /// The selected row, nav item and segment.
    pub selected: Hsla,
    pub accent: Hsla,
    pub on_accent: Hsla,
    /// The primary button's plate: near-white on dark, near-black on light.
    /// The primary action is neutral; the accent is not a call to action.
    pub solid: Hsla,
    pub on_solid: Hsla,
    /// Menus, tooltips and notifications: opaque, because gpui 0.2 has no
    /// backdrop blur and a translucent plate without one looks muddy.
    pub popover: Hsla,
    /// Dialogs, one step up from the popover on dark.
    pub dialog: Hsla,
    /// What a dialog dims the window with.
    pub scrim: Hsla,
    /// The opaque plate behind an outlined button or the composer.
    pub plate: Hsla,
    /// The log viewer and code blocks.
    pub terminal: Hsla,
    pub danger: Hsla,
    /// Text on a danger tint.
    pub danger_muted: Hsla,
    pub warning: Hsla,
    pub warning_muted: Hsla,
    pub success: Hsla,
    pub success_muted: Hsla,
    /// The firmware's blue ("working"). Zeron has no info hue; this is its
    /// blue accent preset.
    pub blue: Hsla,
    /// Claude's own orange, the one coloured glyph in the window.
    pub claude: Hsla,
    /// The Gemini mark's blue.
    pub gemini: Hsla,
    /// The switch track when on, and off.
    pub switch_on: Hsla,
    pub switch_off: Hsla,
    /// The off-white (92 % L) or off-black (10 % L) that washes are made of:
    /// softer than pure ink, so a hover reads as tinted glass.
    wash_base: Hsla,
    /// Pure white or black.
    ink_base: Hsla,
}

impl Global for Palette {}

fn hex(value: u32) -> Hsla {
    rgb(value).into()
}

fn hexa(value: u32) -> Hsla {
    rgba(value).into()
}

impl Palette {
    pub fn dark(frosted: bool) -> Palette {
        let accent = hex(0x8b7cf6);
        Palette {
            dark: true,
            frosted,
            glass: if frosted {
                hex(0x0d0d0d).alpha(GLASS_ALPHA)
            } else {
                hex(0x0d0d0d)
            },
            text: hex(0xe8e8ea),
            muted: hex(0xa9a9ae),
            faint: hex(0x85858a),
            border: hexa(0xffffff1a),
            border_strong: hexa(0xffffff2e),
            hover: hexa(0xffffff1c),
            selected: hexa(0xebebeb1c),
            accent,
            on_accent: hex(0x000000),
            solid: hex(0xebebef),
            on_solid: hex(0x000000),
            popover: hex(0x27272a),
            dialog: hex(0x151516),
            scrim: hexa(0x00000059),
            plate: hex(0x27272a),
            terminal: hex(0x090909),
            danger: hex(0xf87171),
            danger_muted: hex(0xf49293),
            warning: hex(0xfacc15),
            warning_muted: hex(0xf6d34a),
            success: hex(0x34d399),
            success_muted: hex(0x61d8ad),
            blue: hex(0x60a5fa),
            claude: hex(0xd97757),
            gemini: hex(0x7aa2ff),
            switch_on: hex(0x786bd4),
            switch_off: hex(0x424242),
            wash_base: hex(0xebebeb),
            ink_base: hex(0xffffff),
        }
    }

    pub fn light(frosted: bool) -> Palette {
        let accent = hex(0x5b43e8);
        Palette {
            dark: false,
            frosted,
            glass: if frosted {
                hex(0xf3f3f5).alpha(GLASS_ALPHA)
            } else {
                hex(0xf3f3f5)
            },
            text: hex(0x303035),
            muted: hex(0x62626a),
            faint: hex(0x797981),
            border: hexa(0x0000001f),
            border_strong: hexa(0x00000038),
            hover: hexa(0x0000000f),
            selected: hexa(0x1a1a1a0f),
            accent,
            on_accent: hex(0xffffff),
            solid: hex(0x232328),
            on_solid: hex(0xffffff),
            popover: hex(0xffffff),
            dialog: hex(0xfefefe),
            scrim: hexa(0x00000030),
            plate: hex(0xffffff),
            terminal: hex(0xfafafa),
            danger: hex(0xdc2626),
            danger_muted: hex(0xac292a),
            warning: hex(0xa16207),
            warning_muted: hex(0x855612),
            success: hex(0x15803d),
            success_muted: hex(0x1c6c3b),
            blue: hex(0x2563eb),
            claude: hex(0xd97757),
            gemini: hex(0x3b6fe0),
            switch_on: hex(0x6e59eb),
            switch_off: hex(0xd6d6d8),
            wash_base: hex(0x1a1a1a),
            ink_base: hex(0x000000),
        }
    }

    /// The palette for `appearance`.
    pub fn for_appearance(appearance: WindowAppearance, frosted: bool) -> Palette {
        if is_dark(appearance) {
            Palette::dark(frosted)
        } else {
            Palette::light(frosted)
        }
    }

    /// A softened-ink fill at `alpha`: the family hover, selection and
    /// cards are all drawn from. The alphas are the same in both
    /// appearances; only hairlines are scaled for light.
    pub fn wash(&self, alpha: f32) -> Hsla {
        self.wash_base.alpha(alpha)
    }

    /// Pure ink at `alpha`.
    pub fn ink(&self, alpha: f32) -> Hsla {
        self.ink_base.alpha(alpha)
    }

    /// A 1 px line at `alpha`. Black needs more alpha than white to read as
    /// the same weight, so light scales it by 1.35.
    pub fn hairline(&self, alpha: f32) -> Hsla {
        if self.dark {
            self.ink_base.alpha(alpha)
        } else {
            self.ink_base.alpha((alpha * 1.35).min(0.5))
        }
    }

    /// A section card: a 4.5 % wash, no border, no shadow.
    pub fn card(&self) -> Hsla {
        self.wash(0.045)
    }

    /// The rule between two rows of a card, inset to the text edge.
    pub fn divider(&self) -> Hsla {
        self.border.opacity(0.6)
    }

    /// A text field's well.
    pub fn field(&self) -> Hsla {
        self.ink(0.04)
    }

    /// The accent wash behind an accent word or a live chip.
    pub fn accent_wash(&self) -> Hsla {
        self.accent.alpha(if self.dark { 0.22 } else { 0.12 })
    }

    /// The firmware's five tones (§8.2: a red dot here and a red dot on the
    /// deck mean the same thing).
    pub fn tone(&self, tone: Tone) -> Hsla {
        match tone {
            Tone::Blue => self.blue,
            Tone::Amber => self.warning,
            Tone::Green => self.success,
            Tone::Red => self.danger,
            Tone::Gray => self.faint,
        }
    }

    /// Text in `tone` that has to sit on that tone's tint and still read.
    pub fn tone_text(&self, tone: Tone) -> Hsla {
        match tone {
            Tone::Blue => self.blue,
            Tone::Amber => self.warning_muted,
            Tone::Green => self.success_muted,
            Tone::Red => self.danger_muted,
            Tone::Gray => self.muted,
        }
    }
}

/// The palette the window is drawn with.
pub fn palette(cx: &App) -> &Palette {
    cx.global::<Palette>()
}

pub fn is_dark(appearance: WindowAppearance) -> bool {
    matches!(
        appearance,
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    )
}

/// What [`APPEARANCE_ENV`] asks for, if anything.
pub fn pinned_appearance() -> Option<bool> {
    match std::env::var(APPEARANCE_ENV).ok()?.trim() {
        s if s.eq_ignore_ascii_case("dark") => Some(true),
        s if s.eq_ignore_ascii_case("light") => Some(false),
        _ => None,
    }
}

/// Whether [`GLASS_ENV`] allows the frosted shell.
pub fn glass_allowed() -> bool {
    std::env::var(GLASS_ENV).map_or(true, |v| v.trim() != "0")
}

/// Load Geist and Geist Mono from the binary. They are registered with the
/// text system and never served through the `AssetSource`, so a dev build
/// cannot lose them and the asset bundle stays icons only.
pub fn register_fonts(cx: &mut App) {
    let faces: Vec<Cow<'static, [u8]>> = vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/Geist.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Geist-Medium.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Geist-SemiBold.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Geist-Bold.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/GeistMono.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/GeistMono-Medium.ttf").as_slice()),
    ];
    if let Err(e) = cx.text_system().add_fonts(faces) {
        // The system font still draws every string; this is a look, not a
        // function, so it is a line on stderr and not a refusal to start.
        eprintln!("openwalkietalkie-app: the bundled fonts did not load ({e})");
    }
}

/// Make `palette` the window's: the global every screen reads, and the
/// gpui-component `Theme` its own widgets read. Runs at boot and again on
/// every appearance change, after `Theme::change` has reset the stock
/// values for the new mode.
pub fn install(palette: Palette, cx: &mut App) {
    let mode = if palette.dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    Theme::change(mode, None, cx);
    let p = palette;
    let theme = Theme::global_mut(cx);
    theme.font_family = SANS.into();
    theme.mono_font_family = MONO.into();
    // One rem stays 16 px (`Root` sets the rem from this), and every
    // screen sets its own sizes in pixels on top of it.
    theme.font_size = px(16.);
    theme.mono_font_size = px(12.5);
    theme.radius = px(8.);
    theme.radius_lg = px(12.);
    // Shadows only where there is no frost; the widgets that would get
    // one here (inputs, buttons) sit on glass.
    theme.shadow = false;
    let c = &mut theme.colors;
    c.background = p.glass;
    c.foreground = p.text;
    c.muted = p.wash(0.06);
    c.muted_foreground = p.muted;
    c.border = p.border;
    c.input = p.hairline(0.08);
    c.ring = p.accent;
    c.caret = p.accent;
    c.selection = p.accent.alpha(if p.dark { 0.35 } else { 0.24 });
    c.accent = p.hover;
    c.accent_foreground = p.text;
    c.popover = p.popover;
    c.popover_foreground = p.text;
    c.overlay = p.scrim;
    c.list = gpui::transparent_black();
    c.list_hover = p.hover;
    c.list_active = p.selected;
    c.list_active_border = gpui::transparent_black();
    c.primary = p.solid;
    c.primary_hover = p.solid.opacity(0.9);
    c.primary_active = p.solid.opacity(0.85);
    c.primary_foreground = p.on_solid;
    c.secondary = p.wash(0.06);
    c.secondary_hover = p.wash(0.10);
    c.secondary_active = p.wash(0.12);
    c.secondary_foreground = p.text;
    c.danger = p.danger;
    c.danger_hover = p.danger.opacity(0.9);
    c.danger_active = p.danger.opacity(0.85);
    c.danger_foreground = gpui::white();
    c.warning = p.warning;
    c.warning_foreground = p.on_solid;
    c.success = p.success;
    c.success_foreground = p.on_solid;
    c.info = p.blue;
    c.info_foreground = p.on_solid;
    c.link = p.accent;
    c.link_hover = p.accent.opacity(0.85);
    c.sidebar = p.wash(0.05);
    c.sidebar_border = p.border;
    c.sidebar_accent = p.selected;
    c.sidebar_accent_foreground = p.text;
    c.sidebar_foreground = p.text;
    c.title_bar = gpui::transparent_black();
    c.title_bar_border = gpui::transparent_black();
    c.window_border = gpui::transparent_black();
    c.skeleton = p.ink(0.04);
    c.scrollbar = gpui::transparent_black();
    c.scrollbar_thumb = p.text.alpha(0.30);
    c.scrollbar_thumb_hover = p.text.alpha(0.42);
    c.switch = p.switch_off;
    c.switch_thumb = gpui::white();
    c.progress_bar = p.accent;
    c.tab_active = p.wash(0.10);
    c.drag_border = p.accent;
    c.drop_target = p.accent_wash();
    cx.set_global(palette);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_appearances_keep_text_well_clear_of_the_glass_they_sit_on() {
        // WCAG relative luminance, enough to catch a palette edit that
        // swapped a light seed into the dark set or the other way round.
        fn luminance(c: Hsla) -> f32 {
            let rgb = c.to_rgb();
            let lin = |v: f32| {
                if v <= 0.039_28 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * lin(rgb.r) + 0.7152 * lin(rgb.g) + 0.0722 * lin(rgb.b)
        }
        fn contrast(a: Hsla, b: Hsla) -> f32 {
            let (la, lb) = (luminance(a), luminance(b));
            (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
        }
        for p in [Palette::dark(false), Palette::light(false)] {
            assert!(contrast(p.text, p.glass) > 10.0, "{p:?}");
            assert!(contrast(p.muted, p.glass) > 4.5, "{p:?}");
            assert!(contrast(p.faint, p.glass) > 3.5, "{p:?}");
            assert!(contrast(p.on_solid, p.solid) > 10.0, "{p:?}");
        }
    }

    #[test]
    fn the_glass_is_translucent_only_when_something_blurs_behind_it() {
        // What the window server ends up blending is alpha squared (see
        // `GLASS_ALPHA`): Zeron's 80 %.
        assert!((GLASS_ALPHA * GLASS_ALPHA - 0.80).abs() < 0.01);
        assert!(Palette::dark(true).glass.a < 1.0);
        assert!(Palette::light(true).glass.a < 1.0);
        assert_eq!(Palette::dark(false).glass.a, 1.0);
        assert_eq!(Palette::light(false).glass.a, 1.0);
    }

    #[test]
    fn lines_and_washes_are_ink_at_an_alpha_and_light_lines_weigh_more() {
        let (dark, light) = (Palette::dark(true), Palette::light(true));
        for p in [dark, light] {
            assert!(p.border.a < 0.2 && p.hover.a < 0.2 && p.selected.a < 0.2);
            assert!((p.wash(0.11).a - 0.11).abs() < 1e-6);
        }
        assert!(light.hairline(0.08).a > dark.hairline(0.08).a);
        assert!(
            light.hairline(0.9).a <= 0.5,
            "a light hairline never goes past half black"
        );
    }

    #[test]
    fn the_firmwares_five_tones_stay_five_different_colours() {
        for p in [Palette::dark(true), Palette::light(true)] {
            let tones = [Tone::Blue, Tone::Amber, Tone::Green, Tone::Red, Tone::Gray];
            for (i, a) in tones.iter().enumerate() {
                for b in &tones[i + 1..] {
                    assert_ne!(p.tone(*a), p.tone(*b), "{a:?} and {b:?}");
                }
            }
        }
    }
}

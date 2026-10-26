//! Artwork light: the colour a tile emits when focused (DESIGN.md §3.2).

use serde::{Deserialize, Serialize};

/// `houselight` from the house palette, as opaque ARGB.
pub const HOUSELIGHT: u32 = 0xFFF2B263;
pub const VELVET: u32 = 0xFF1A1016;
pub const LIMELIGHT: u32 = 0xFFF6EFE7;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Palette {
    pub dominant: u32,
    pub vibrant: Option<u32>,
    pub muted: Option<u32>,
}

/// Take `vibrant` if its chroma is above 0.12, else `dominant`; clamp lightness to 0.55–0.72.
pub fn light_from_palette(p: &Palette) -> u32 {
    let base = match p.vibrant {
        Some(v) if chroma(v) > 0.12 => v,
        _ => p.dominant,
    };
    let (h, s, l) = rgb_to_hsl(base);
    if s < 0.08 {
        return HOUSELIGHT;
    }
    let l = l.clamp(0.55, 0.72);
    let s = s.max(0.35);
    hsl_to_argb(h, s, l)
}


pub fn chroma(argb: u32) -> f32 {
    let (r, g, b) = rgb_f(argb);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    max - min
}

pub fn rgb_f(argb: u32) -> (f32, f32, f32) {
    let r = ((argb >> 16) & 0xFF) as f32 / 255.0;
    let g = ((argb >> 8) & 0xFF) as f32 / 255.0;
    let b = (argb & 0xFF) as f32 / 255.0;
    (r, g, b)
}

pub fn rgb_to_hsl(argb: u32) -> (f32, f32, f32) {
    let (r, g, b) = rgb_f(argb);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if (max - r).abs() < f32::EPSILON {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if (max - g).abs() < f32::EPSILON {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

pub fn hsl_to_argb(h: f32, s: f32, l: f32) -> u32 {
    fn hue(p: f32, q: f32, mut t: f32) -> f32 {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    }
    let (r, g, b) = if s <= 0.0 {
        (l, l, l)
    } else {
        let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
        let p = 2.0 * l - q;
        (hue(p, q, h + 1.0 / 3.0), hue(p, q, h), hue(p, q, h - 1.0 / 3.0))
    };
    let to = |v: f32| ((v.clamp(0.0, 1.0) * 255.0).round() as u32) & 0xFF;
    0xFF00_0000 | (to(r) << 16) | (to(g) << 8) | to(b)
}

fn luminance(argb: u32) -> f32 {
    let lin = |c: f32| {
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = rgb_f(argb);
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

pub fn contrast_ratio(a: u32, b: u32) -> f32 {
    let la = luminance(a);
    let lb = luminance(b);
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// OKLab of an sRGB colour given as 0–1 channels.
pub fn oklab(r: f32, g: f32, b: f32) -> [f32; 3] {
    let (r, g, b) = (srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b));
    let l = (0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// The opaque sRGB colour of an OKLab triple, clamped into gamut.
pub fn from_oklab([l, a, b]: [f32; 3]) -> u32 {
    let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    let r = 4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_;
    let g = -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_;
    let bl = -0.004_196_086_3 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_;
    let to = |v: f32| ((linear_to_srgb(v) * 255.0).round() as u32) & 0xFF;
    0xFF00_0000 | (to(r) << 16) | (to(g) << 8) | to(bl)
}

/// A picture's colour as the collapsed Stage's light: lightness held to 0.2–0.5 so it glows in
/// the dark room without shouting, chroma lifted by half up to 0.17 (DESIGN.md §5.2).
pub fn glow_tone(argb: u32) -> u32 {
    let (r, g, b) = rgb_f(argb);
    let [l, a, bb] = oklab(r, g, b);
    let c = (a * a + bb * bb).sqrt();
    let k = if c > 1e-5 { (c * 1.5).min(0.17) / c } else { 0.0 };
    from_oklab([l.clamp(0.2, 0.5), a * k, bb * k])
}

/// `n` colours evenly across the band from its pools, eased between neighbouring pools in OKLab
/// so each pool reads as a soft patch of its own colour and two strong colours never muddy.
pub fn glow_stops(pools: &[u32], n: usize) -> Vec<u32> {
    if pools.is_empty() || n == 0 {
        return Vec::new();
    }
    let labs: Vec<[f32; 3]> = pools.iter().map(|&c| { let (r, g, b) = rgb_f(c); oklab(r, g, b) }).collect();
    let k = labs.len();
    (0..n)
        .map(|i| {
            let x = if n == 1 { 0.5 } else { i as f32 / (n - 1) as f32 };
            // Pool centres sit at the middle of each of the k equal slices.
            let f = (x * k as f32 - 0.5).clamp(0.0, (k - 1) as f32);
            let lo = (f.floor() as usize).min(k - 1);
            let hi = (lo + 1).min(k - 1);
            let t = f - lo as f32;
            let t = t * t * (3.0 - 2.0 * t);
            let mix = |j: usize| labs[lo][j] + (labs[hi][j] - labs[lo][j]) * t;
            from_oklab([mix(0), mix(1), mix(2)])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklab_roundtrips() {
        for &c in &[0xFFE8894Cu32, 0xFF6FB7D9, 0xFF123456, 0xFFFFFFFF, 0xFF000000, 0xFFFFA86B] {
            let (r, g, b) = rgb_f(c);
            let back = from_oklab(oklab(r, g, b));
            let (r2, g2, b2) = rgb_f(back);
            assert!((r - r2).abs() < 0.01 && (g - g2).abs() < 0.01 && (b - b2).abs() < 0.01, "{c:08x} came back {back:08x}");
        }
    }

    #[test]
    fn glow_stays_in_the_dark_and_keeps_its_hue() {
        // A white coat and a deep navy both land in the glow's lightness range.
        for &c in &[0xFFF8F8F0u32, 0xFF050818] {
            let (r, g, b) = rgb_f(glow_tone(c));
            let l = oklab(r, g, b)[0];
            assert!((0.19..=0.51).contains(&l), "{c:08x} toned to lightness {l}");
        }
        // A yellow coat stays yellow, only toned.
        let (r, g, b) = rgb_f(glow_tone(0xFFD4A520));
        assert!(r > b + 0.2 && g > b + 0.15, "yellow lost its hue: {r} {g} {b}");
        // Grey stays grey: black and white films get silver, not invented colour.
        let (r, g, b) = rgb_f(glow_tone(0xFF808080));
        assert!((r - g).abs() < 0.01 && (g - b).abs() < 0.01);
    }

    #[test]
    fn stops_hold_each_pool_at_its_centre() {
        let pools = [0xFF203060u32, 0xFF604020, 0xFF206040];
        let stops = glow_stops(&pools, 25);
        assert_eq!(stops.len(), 25);
        // The ends hold the outer pools, and the middle stop the middle pool.
        let near = |a: u32, b: u32| {
            let (r1, g1, b1) = rgb_f(a);
            let (r2, g2, b2) = rgb_f(b);
            (r1 - r2).abs() < 0.02 && (g1 - g2).abs() < 0.02 && (b1 - b2).abs() < 0.02
        };
        assert!(near(stops[0], pools[0]) && near(stops[24], pools[2]) && near(stops[12], pools[1]));
        assert!(glow_stops(&[], 25).is_empty());
    }

    #[test]
    fn grey_palette_falls_back_to_houselight() {
        let p = Palette { dominant: 0xFF333333, vibrant: None, muted: None };
        assert_eq!(light_from_palette(&p), HOUSELIGHT);
    }

    #[test]
    fn dark_vibrant_is_lifted_into_range() {
        let p = Palette { dominant: 0xFF111111, vibrant: Some(0xFF401208), muted: None };
        let l = light_from_palette(&p);
        let (_, _, light) = rgb_to_hsl(l);
        assert!(light >= 0.54 && light <= 0.73, "lightness {light}");
    }

    #[test]
    fn white_poster_does_not_blind() {
        let p = Palette { dominant: 0xFFF0E8FF, vibrant: Some(0xFFE0D0FF), muted: None };
        let l = light_from_palette(&p);
        let (_, _, light) = rgb_to_hsl(l);
        assert!(light <= 0.73, "lightness {light}");
    }

    #[test]
    fn houselight_contrast_matches_design_doc() {
        let c = contrast_ratio(HOUSELIGHT, VELVET);
        assert!(c > 9.5 && c < 10.5, "{c}");
        assert!(contrast_ratio(LIMELIGHT, VELVET) > 15.0);
    }

    #[test]
    fn hsl_roundtrip() {
        for &c in &[0xFFE8894Cu32, 0xFF6FB7D9, 0xFF123456, 0xFFFFFFFF, 0xFF000000] {
            let (h, s, l) = rgb_to_hsl(c);
            let back = hsl_to_argb(h, s, l);
            let (r1, g1, b1) = rgb_f(c);
            let (r2, g2, b2) = rgb_f(back);
            assert!((r1 - r2).abs() < 0.01 && (g1 - g2).abs() < 0.01 && (b1 - b2).abs() < 0.01);
        }
    }
}

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

/// Text colour that contrasts with `light`: limelight or velvet.
pub fn text_on(light: u32) -> u32 {
    if contrast_ratio(light, VELVET) >= 4.5 {
        VELVET
    } else {
        LIMELIGHT
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(text_on(HOUSELIGHT), VELVET);
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

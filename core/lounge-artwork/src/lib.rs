//! Index-time artwork pipeline: decode once, crop to the tile's aspect, pre-scale to the
//! exact sizes the UI draws, write PNGs to the disk cache, and extract a palette.

use fast_image_resize::{images::Image, images::ImageRef, FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use image::{ImageFormat, RgbaImage};
use lounge_graph::Palette;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ArtworkError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("image: {0}")]
    Image(#[from] image::ImageError),
    #[error("resize: {0}")]
    Resize(String),
}

pub type Result<T> = std::result::Result<T, ArtworkError>;

/// Sources larger than this are not decoded at all; a picture for a TV tile never needs it.
pub const MAX_SOURCE_BYTES: u64 = 12 * 1024 * 1024;
const MAX_SOURCE_SIDE: u32 = 8192;
/// Most pixels a source may decode to: a 12 megapixel photo, about 52 MB as RGBA.
const MAX_SOURCE_PIXELS: u64 = 13_000_000;
/// The lowered Stage's band: full width × 200 at 1080p.
const BAND_ASPECT: f64 = 1728.0 / 200.0;

/// Pixel sizes at 1080p, the largest tile of each style (DESIGN.md §5.3) times the
/// focus scale, so a focused tile never upsamples.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Style {
    Poster,
    Landscape,
    AppBanner,
    /// App banner at its native 640×360, shown 1:1 inside the Stage frame.
    HeroBanner,
    /// Full-bleed Stage key art, its vignette baked in.
    Hero,
    /// Portrait key art at the Stage frame's full height.
    PosterLarge,
    /// Key art of any aspect fitted inside the Stage frame (no crop), for framed content.
    HeroFit,
    /// 160×90 thumbnail, Gaussian-blurred at index time, that the Stage and the collapsed band
    /// scale up as a lit backdrop: a real blur with no render-time cost.
    Backdrop,
    /// The slice of the key art the lowered Stage shows: 1728×200 around the picture's focal line.
    Band,
    Icon,
    /// A channel or publisher logo fitted inside 160×160 with its alpha kept: never cropped.
    Logo,
    /// A screensaver picture at the full 1920×1080, nothing baked in.
    Wallpaper,
    /// A screensaver picture fitted whole inside 1920×1080, its own shape kept: a painting.
    WallpaperFit,
    /// [`Style::Wallpaper`] for a 2160p panel.
    WallpaperUhd,
    /// [`Style::WallpaperFit`] for a 2160p panel.
    WallpaperFitUhd,
}

impl Style {
    pub fn target(self) -> (u32, u32) {
        match self {
            Style::Poster => (264, 396),
            Style::Landscape => (476, 268),
            Style::AppBanner => (340, 192),
            Style::HeroBanner => (640, 360),
            Style::Hero => (1120, 630),
            Style::PosterLarge => (420, 630),
            Style::HeroFit => (1120, 630),
            Style::Backdrop => (160, 90),
            Style::Band => (1728, 200),
            Style::Icon => (160, 160),
            Style::Logo => (160, 160),
            Style::Wallpaper => (1920, 1080),
            Style::WallpaperFit => (1920, 1080),
            Style::WallpaperUhd => (3840, 2160),
            Style::WallpaperFitUhd => (3840, 2160),
        }
    }

    /// The wallpaper styles for a panel: 2160p ones when `uhd`.
    pub fn wallpaper(framed: bool, uhd: bool) -> Style {
        match (framed, uhd) {
            (false, false) => Style::Wallpaper,
            (true, false) => Style::WallpaperFit,
            (false, true) => Style::WallpaperUhd,
            (true, true) => Style::WallpaperFitUhd,
        }
    }

    /// Styles that fit the whole image inside the target instead of cover-cropping it.
    fn fits(self) -> bool {
        matches!(self, Style::HeroFit | Style::Logo | Style::WallpaperFit | Style::WallpaperFitUhd)
    }

    /// A screensaver picture is never made larger than it came: the panel scales it as well as
    /// the resampler would, and the file stays small.
    fn never_upsamples(self) -> bool {
        matches!(self, Style::Wallpaper | Style::WallpaperFit | Style::WallpaperUhd | Style::WallpaperFitUhd)
    }

    pub fn slug(self) -> &'static str {
        match self {
            Style::Poster => "poster",
            Style::Landscape => "land",
            Style::AppBanner => "banner",
            Style::HeroBanner => "herobanner",
            Style::Hero => "hero",
            Style::PosterLarge => "posterlarge",
            Style::HeroFit => "herofit",
            Style::Backdrop => "backdrop",
            Style::Band => "band",
            Style::Icon => "icon",
            Style::Logo => "logo",
            Style::Wallpaper => "wall",
            Style::WallpaperFit => "wallfit",
            Style::WallpaperUhd => "walluhd",
            Style::WallpaperFitUhd => "wallfituhd",
        }
    }

    /// Blur sigma applied after scaling, in output pixels.
    fn blur_sigma(self) -> Option<f32> {
        match self {
            Style::Backdrop => Some(3.5),
            _ => None,
        }
    }

    /// Styles whose output is a slice of the 16:9 frame and so carry the picture's focal line.
    fn wants_focal(self) -> bool {
        matches!(self, Style::Hero | Style::Band)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prepared {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub palette: Palette,
    /// Dimensions of the source image, so callers know its aspect.
    #[serde(default)]
    pub src_width: u32,
    #[serde(default)]
    pub src_height: u32,
    /// Where the picture's interest sits vertically within its 16:9 frame (0 top, 1 bottom).
    #[serde(default = "half")]
    pub focal_y: f32,
}

fn half() -> f32 {
    0.5
}

/// Decode `src`, cover-crop it to `style`'s aspect, scale it to the style's target size,
/// write `<out_dir>/<hash>-<style>.jpg` (or `.png` when it has transparency) and return the output path with its palette.
pub fn prepare(src: &Path, out_dir: &Path, style: Style) -> Result<Prepared> {
    prepare_many(src, out_dir, &[style]).pop().unwrap_or_else(|| Err(ArtworkError::Resize("no style".into())))
}

/// As [`prepare`] for several styles of one source: the file is read, hashed, decoded and
/// measured once, and every style is cut from that one decode.
pub fn prepare_many(src: &Path, out_dir: &Path, styles: &[Style]) -> Vec<Result<Prepared>> {
    let shared = (|| -> Result<(String, RgbaImage, Palette, bool, f32)> {
        std::fs::create_dir_all(out_dir)?;
        let size = std::fs::metadata(src)?.len();
        if size > MAX_SOURCE_BYTES {
            return Err(ArtworkError::Resize(format!("{size} bytes is too large a picture")));
        }
        let bytes = std::fs::read(src)?;
        let hash = blake3::hash(&bytes).to_hex().to_string();
        let decoded = decode(&bytes)?;
        drop(bytes);
        let (sw, sh) = decoded.dimensions();
        let palette = palette_of(&decoded);
        // Opaque pictures become JPEGs: smaller, quicker to decode, and drawn without blending.
        let opaque = decoded.pixels().all(|p| p.0[3] == 255);
        let focal_y = if styles.iter().any(|s| s.wants_focal()) { focal_line(&decoded, sw, sh)? } else { 0.5 };
        Ok((hash, decoded, palette, opaque, focal_y))
    })();
    let (hash, decoded, palette, opaque, focal_y) = match shared {
        Ok(v) => v,
        Err(e) => {
            let msg = e.to_string();
            return styles.iter().map(|_| Err(ArtworkError::Resize(msg.clone()))).collect();
        }
    };
    let (sw, sh) = decoded.dimensions();
    styles
        .iter()
        .map(|&style| {
            let (mut tw, mut th) = style.target();
            let out_path = out_dir.join(format!("{}-{}.{}", &hash[..20], style.slug(), if opaque { "jpg" } else { "png" }));
            if style.fits() && sw > 0 && sh > 0 {
                let mut scale = (tw as f64 / sw as f64).min(th as f64 / sh as f64);
                if style.never_upsamples() {
                    scale = scale.min(1.0);
                }
                tw = ((sw as f64 * scale).round() as u32).max(1);
                th = ((sh as f64 * scale).round() as u32).max(1);
            } else if style.never_upsamples() && sw > 0 && sh > 0 {
                // The target's shape at the source's own scale, when the source is smaller.
                let scale = (tw as f64 / sw as f64).max(th as f64 / sh as f64);
                if scale > 1.0 {
                    tw = ((tw as f64 / scale).round() as u32).max(1);
                    th = ((th as f64 / scale).round() as u32).max(1);
                }
            }
            if !out_path.exists() {
                let mut scaled = if style == Style::Band {
                    crop_resize(&decoded, sw, sh, band_rect(sw, sh, focal_y), tw, th)?
                } else {
                    cover_resize(&decoded, sw, sh, tw, th)?
                };
                if let Some(sigma) = style.blur_sigma() {
                    scaled = image::imageops::blur(&scaled, sigma);
                }
                if style == Style::Hero {
                    vignette(&mut scaled);
                }
                // Search and the worker can prepare the same picture at once; each writes its own file.
                static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                let tmp = out_path.with_extension(format!("{}.tmp", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
                let written = (|| -> Result<()> {
                    if opaque {
                        let rgb = image::DynamicImage::ImageRgba8(scaled).to_rgb8();
                        let file = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
                        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(file, 90);
                        rgb.write_with_encoder(encoder)?;
                    } else {
                        scaled.save_with_format(&tmp, ImageFormat::Png)?;
                    }
                    std::fs::rename(&tmp, &out_path)?;
                    Ok(())
                })();
                if let Err(e) = written {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(e);
                }
            }
            Ok(Prepared {
                path: out_path.to_string_lossy().into_owned(),
                width: tw,
                height: th,
                palette: palette.clone(),
                src_width: sw,
                src_height: sh,
                focal_y,
            })
        })
        .collect()
}

/// Decode with hard limits: a 4 MB file that unpacks to a 500 MB bitmap is refused before
/// anything is allocated. JPEGs decode straight to RGBA, so no second full-size copy is made.
fn decode(bytes: &[u8]) -> Result<RgbaImage> {
    let too_big = |w: u64, h: u64| w > MAX_SOURCE_SIDE as u64 || h > MAX_SOURCE_SIDE as u64 || w * h > MAX_SOURCE_PIXELS;
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        use zune_jpeg::zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};
        let options = DecoderOptions::default()
            .jpeg_set_out_colorspace(ColorSpace::RGBA)
            .set_max_width(MAX_SOURCE_SIDE as usize)
            .set_max_height(MAX_SOURCE_SIDE as usize);
        let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), options);
        decoder.decode_headers().map_err(|e| ArtworkError::Resize(format!("jpeg: {e:?}")))?;
        let (w, h) = decoder.dimensions().ok_or_else(|| ArtworkError::Resize("jpeg has no size".into()))?;
        if too_big(w as u64, h as u64) {
            return Err(ArtworkError::Resize(format!("{w}×{h} is too large a picture")));
        }
        let pixels = decoder.decode().map_err(|e| ArtworkError::Resize(format!("jpeg: {e:?}")))?;
        return RgbaImage::from_raw(w as u32, h as u32, pixels).ok_or_else(|| ArtworkError::Resize("jpeg buffer size mismatch".into()));
    }
    let (w, h) = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?.into_dimensions()?;
    if too_big(w as u64, h as u64) {
        return Err(ArtworkError::Resize(format!("{w}×{h} is too large a picture")));
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_SIDE);
    limits.max_image_height = Some(MAX_SOURCE_SIDE);
    limits.max_alloc = Some(MAX_SOURCE_PIXELS * 4);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|e| match e {
            // Say what the bytes were, so an unsupported format is recognisable from the log.
            image::ImageError::Unsupported(_) => ArtworkError::Resize(format!(
                "{e} ({} bytes, starts {:02x?})",
                bytes.len(),
                &bytes[..bytes.len().min(8)]
            )),
            other => ArtworkError::Image(other),
        })?
        .into_rgba8();
    Ok(decoded)
}

/// The 16:9 cover crop of a `sw`×`sh` source: (left, top, width, height).
fn cover_rect(sw: u32, sh: u32, aspect: f64) -> (f64, f64, f64, f64) {
    let src_ratio = sw as f64 / sh as f64;
    let (cw, ch) = if src_ratio > aspect {
        ((sh as f64 * aspect).round().max(1.0), sh as f64)
    } else {
        (sw as f64, (sw as f64 / aspect).round().max(1.0))
    };
    (((sw as f64 - cw) / 2.0).max(0.0), ((sh as f64 - ch) / 2.0).max(0.0), cw, ch)
}

/// The band's crop: a slice of the 16:9 frame, `BAND_ASPECT` wide, centred on the focal line.
fn band_rect(sw: u32, sh: u32, focal_y: f32) -> (f64, f64, f64, f64) {
    let (left, top, cw, ch) = cover_rect(sw, sh, 16.0 / 9.0);
    let bh = (cw / BAND_ASPECT).min(ch).max(1.0);
    let btop = (top + (focal_y as f64 * ch - bh / 2.0).clamp(0.0, ch - bh)).max(0.0);
    (left, btop, cw, bh)
}

/// Where the interest is: the horizontal band of the 16:9 frame with the most edge energy, weighted
/// toward the upper third. Measured on a 160-wide thumbnail.
fn focal_line(img: &RgbaImage, sw: u32, sh: u32) -> Result<f32> {
    let (left, top, cw, ch) = cover_rect(sw, sh, 16.0 / 9.0);
    let tw = 160u32;
    let th = 90u32;
    let small = crop_resize(img, sw, sh, (left, top, cw, ch), tw, th)?;
    let grey: Vec<f32> = small.pixels().map(|p| 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32).collect();
    let w = tw as usize;
    let h = th as usize;
    let mut energy = vec![0f32; h];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            energy[y] += (grey[i + 1] - grey[i - 1]).abs() + (grey[i + w] - grey[i - w]).abs();
        }
    }
    let win = ((tw as f64 / BAND_ASPECT).round() as usize).clamp(1, h);
    let mut best = 0usize;
    let mut best_score = f32::MIN;
    for top in 0..=(h - win) {
        let centre = (top as f32 + win as f32 / 2.0) / h as f32;
        let sum: f32 = energy[top..top + win].iter().sum();
        let bias = 1.0 + 0.6 * (1.0 - ((centre - 0.33) / 0.67).abs().min(1.0));
        let score = sum * bias;
        if score > best_score {
            best_score = score;
            best = top;
        }
    }
    Ok(((best as f32 + win as f32 / 2.0) / h as f32).clamp(0.0, 1.0))
}

/// The Stage's vignette, baked: velvet creeping in from 60 % of the half-diagonal to the corners.
fn vignette(img: &mut RgbaImage) {
    let (w, h) = img.dimensions();
    let cx = w as f32 / 2.0;
    let cy = h as f32 / 2.0;
    let half_diag = (cx * cx + cy * cy).sqrt().max(1.0);
    const VELVET: [f32; 3] = [0x1A as f32, 0x10 as f32, 0x16 as f32];
    for (x, y, p) in img.enumerate_pixels_mut() {
        let dx = x as f32 - cx;
        let dy = y as f32 - cy;
        let r = (dx * dx + dy * dy).sqrt() / half_diag;
        let t = ((r - 0.6) / 0.4).clamp(0.0, 1.0);
        let a = t * t * (3.0 - 2.0 * t) * 0.45;
        if a <= 0.0 {
            continue;
        }
        for c in 0..3 {
            p[c] = (p[c] as f32 * (1.0 - a) + VELVET[c] * a).round().clamp(0.0, 255.0) as u8;
        }
    }
}

fn cover_resize(src: &RgbaImage, sw: u32, sh: u32, tw: u32, th: u32) -> Result<RgbaImage> {
    let rect = cover_rect(sw, sh, tw as f64 / th as f64);
    crop_resize(src, sw, sh, rect, tw, th)
}

fn crop_resize(src: &RgbaImage, sw: u32, sh: u32, (left, top, cw, ch): (f64, f64, f64, f64), tw: u32, th: u32) -> Result<RgbaImage> {
    let src_img = ImageRef::new(sw, sh, src.as_raw(), PixelType::U8x4)
        .map_err(|e| ArtworkError::Resize(e.to_string()))?;
    let mut dst = Image::new(tw, th, PixelType::U8x4);

    let alg = if tw < sw || th < sh {
        ResizeAlg::Convolution(FilterType::Lanczos3)
    } else {
        ResizeAlg::Convolution(FilterType::Mitchell)
    };
    let options = ResizeOptions::new().resize_alg(alg).crop(left, top, cw, ch);
    let mut resizer = Resizer::new();
    resizer
        .resize(&src_img, &mut dst, &options)
        .map_err(|e| ArtworkError::Resize(e.to_string()))?;

    RgbaImage::from_raw(tw, th, dst.into_vec())
        .ok_or_else(|| ArtworkError::Resize("buffer size mismatch".into()))
}

/// k-means over a 48×48 thumbnail: dominant = largest cluster, vibrant = most saturated
/// mid-lightness cluster with real weight, muted = the largest low-saturation cluster.
pub fn palette_of(img: &RgbaImage) -> Palette {
    let (w, h) = img.dimensions();
    let step_x = (w / 48).max(1);
    let step_y = (h / 48).max(1);
    let mut samples: Vec<[f32; 3]> = Vec::with_capacity(48 * 48);
    for y in (0..h).step_by(step_y as usize) {
        for x in (0..w).step_by(step_x as usize) {
            let p = img.get_pixel(x, y);
            if p[3] < 128 {
                continue;
            }
            samples.push([p[0] as f32, p[1] as f32, p[2] as f32]);
        }
    }
    if samples.is_empty() {
        return Palette {
            dominant: lounge_graph::color::VELVET,
            vibrant: None,
            muted: None,
        };
    }

    const K: usize = 6;
    let mut centers: Vec<[f32; 3]> = (0..K)
        .map(|i| samples[(i * samples.len()) / K])
        .collect();
    let mut assign = vec![0usize; samples.len()];
    for _ in 0..10 {
        for (i, s) in samples.iter().enumerate() {
            let mut best = 0;
            let mut best_d = f32::MAX;
            for (c, center) in centers.iter().enumerate() {
                let d = dist2(s, center);
                if d < best_d {
                    best_d = d;
                    best = c;
                }
            }
            assign[i] = best;
        }
        let mut sums = vec![[0f32; 3]; K];
        let mut counts = vec![0usize; K];
        for (i, s) in samples.iter().enumerate() {
            let c = assign[i];
            sums[c][0] += s[0];
            sums[c][1] += s[1];
            sums[c][2] += s[2];
            counts[c] += 1;
        }
        for c in 0..K {
            if counts[c] > 0 {
                centers[c] = [
                    sums[c][0] / counts[c] as f32,
                    sums[c][1] / counts[c] as f32,
                    sums[c][2] / counts[c] as f32,
                ];
            }
        }
    }
    let mut counts = vec![0usize; K];
    for &a in &assign {
        counts[a] += 1;
    }
    let total = samples.len() as f32;
    let clusters: Vec<(u32, f32)> = centers
        .iter()
        .zip(counts.iter())
        .filter(|&(_, &n)| n > 0)
        .map(|(c, &n)| (to_argb(c), n as f32 / total))
        .collect();

    let dominant = clusters
        .iter()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|c| c.0)
        .unwrap_or(lounge_graph::color::VELVET);

    let vibrant = clusters
        .iter()
        .filter(|(c, weight)| {
            let (_, s, l) = lounge_graph::color::rgb_to_hsl(*c);
            s > 0.3 && (0.2..=0.85).contains(&l) && *weight > 0.04
        })
        .max_by(|a, b| score_vibrant(a).total_cmp(&score_vibrant(b)))
        .map(|c| c.0);

    let muted = clusters
        .iter()
        .filter(|(c, _)| {
            let (_, s, l) = lounge_graph::color::rgb_to_hsl(*c);
            s <= 0.3 && (0.15..=0.85).contains(&l)
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|c| c.0);

    Palette {
        dominant,
        vibrant,
        muted,
    }
}

fn score_vibrant(c: &(u32, f32)) -> f32 {
    let (_, s, l) = lounge_graph::color::rgb_to_hsl(c.0);
    let mid = 1.0 - (l - 0.5).abs() * 1.6;
    s * mid.max(0.1) * c.1.sqrt()
}

fn dist2(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    let dr = a[0] - b[0];
    let dg = a[1] - b[1];
    let db = a[2] - b[2];
    dr * dr + dg * dg + db * db
}

fn to_argb(c: &[f32; 3]) -> u32 {
    let r = c[0].round().clamp(0.0, 255.0) as u32;
    let g = c[1].round().clamp(0.0, 255.0) as u32;
    let b = c[2].round().clamp(0.0, 255.0) as u32;
    0xFF00_0000 | (r << 16) | (g << 8) | b
}

pub fn file_signature(path: &Path) -> std::io::Result<(u64, i64)> {
    let m = std::fs::metadata(path)?;
    let mtime = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Ok((m.len(), mtime))
}

pub fn out_dir_for(cache_dir: &Path) -> PathBuf {
    cache_dir.join("art")
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn synthetic(w: u32, h: u32) -> RgbaImage {
        let mut img = RgbaImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let px = if x < w / 2 { Rgba([230, 120, 60, 255]) } else { Rgba([20, 20, 20, 255]) };
                img.put_pixel(x, y, px);
            }
        }
        img
    }

    #[test]
    fn palette_finds_the_orange() {
        let img = synthetic(200, 120);
        let p = palette_of(&img);
        let v = p.vibrant.expect("vibrant");
        let (h, s, _) = lounge_graph::color::rgb_to_hsl(v);
        assert!(s > 0.5, "saturation {s}");
        assert!(h < 0.12, "hue {h}");
    }

    #[test]
    fn prepare_writes_exact_size_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("banner.png");
        synthetic(640, 360).save(&src).unwrap();
        let out = dir.path().join("art");
        let a = prepare(&src, &out, Style::AppBanner).unwrap();
        let b = prepare(&src, &out, Style::AppBanner).unwrap();
        assert_eq!(a, b);
        let (tw, th) = Style::AppBanner.target();
        let img = image::open(&a.path).unwrap();
        assert_eq!((img.width(), img.height()), (tw, th));
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 1);
    }

    #[test]
    fn backdrop_is_blurred() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("banner.png");
        synthetic(640, 360).save(&src).unwrap();
        let p = prepare(&src, &dir.path().join("art"), Style::Backdrop).unwrap();
        let img = image::open(&p.path).unwrap().to_rgba8();
        let (w, h) = img.dimensions();
        assert_eq!((w, h), Style::Backdrop.target());
        // The hard edge in the middle of the source must have become a gradient.
        let mid = img.get_pixel(w / 2, h / 2)[0];
        let left = img.get_pixel(w / 2 - 6, h / 2)[0];
        let right = img.get_pixel(w / 2 + 6, h / 2)[0];
        assert!(left > mid && mid > right, "left {left} mid {mid} right {right}");
    }

    #[test]
    fn hero_fit_keeps_the_whole_image() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("poster.png");
        synthetic(400, 600).save(&src).unwrap();
        let p = prepare(&src, &dir.path().join("art"), Style::HeroFit).unwrap();
        assert_eq!((p.width, p.height), (420, 630));
        assert_eq!((p.src_width, p.src_height), (400, 600));
    }

    #[test]
    fn a_wallpaper_is_never_made_larger_than_it_came() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("small.png");
        RgbaImage::from_pixel(1280, 960, Rgba([120, 90, 60, 255])).save(&src).unwrap();
        let p = prepare(&src, dir.path(), Style::WallpaperUhd).unwrap();
        assert_eq!((p.width, p.height), (1280, 720));
        let p = prepare(&src, dir.path(), Style::WallpaperFitUhd).unwrap();
        assert_eq!((p.width, p.height), (1280, 960));
        assert_eq!(Style::wallpaper(true, true), Style::WallpaperFitUhd);
        assert_eq!(Style::wallpaper(false, false), Style::Wallpaper);
    }

    #[test]
    fn a_fitted_wallpaper_keeps_its_shape() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("portrait.png");
        synthetic(600, 800).save(&src).unwrap();
        let p = prepare(&src, &dir.path().join("art"), Style::WallpaperFit).unwrap();
        assert_eq!((p.width, p.height), (600, 800));
        let src = dir.path().join("tall.png");
        synthetic(1500, 2000).save(&src).unwrap();
        let p = prepare(&src, &dir.path().join("art"), Style::WallpaperFit).unwrap();
        assert_eq!((p.width, p.height), (810, 1080));
    }

    #[test]
    fn cover_crop_keeps_aspect_from_odd_sources() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("tall.png");
        synthetic(100, 400).save(&src).unwrap();
        let p = prepare(&src, &dir.path().join("art"), Style::Landscape).unwrap();
        assert_eq!((p.width, p.height), Style::Landscape.target());
    }

    #[test]
    fn band_follows_the_focal_line() {
        // A frame that is dark except for a bright detailed stripe near the top: the band must find it.
        let mut img = RgbaImage::from_pixel(1920, 1080, Rgba([10, 10, 10, 255]));
        for y in 180..300 {
            for x in 0..1920 {
                let v = if (x / 7 + y / 5) % 2 == 0 { 240 } else { 30 };
                img.put_pixel(x, y, Rgba([v, v, v, 255]));
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("frame.png");
        img.save(&src).unwrap();
        let p = prepare(&src, &dir.path().join("art"), Style::Band).unwrap();
        assert_eq!((p.width, p.height), Style::Band.target());
        assert!((0.15..0.32).contains(&p.focal_y), "focal {}", p.focal_y);
        let out = image::open(&p.path).unwrap().to_rgba8();
        let bright = out.pixels().filter(|px| px[0] > 128).count();
        assert!(bright > (1728 * 200) / 4, "band missed the stripe: {bright} bright pixels");
    }

    #[test]
    fn hero_has_its_vignette() {
        let img = RgbaImage::from_pixel(1920, 1080, Rgba([200, 200, 200, 255]));
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("flat.png");
        img.save(&src).unwrap();
        let p = prepare(&src, &dir.path().join("art"), Style::Hero).unwrap();
        let out = image::open(&p.path).unwrap().to_rgba8();
        let centre = out.get_pixel(560, 315)[0];
        let corner = out.get_pixel(4, 4)[0];
        assert!(centre > 190 && corner < 150, "centre {centre} corner {corner}");
    }

    #[test]
    fn jpegs_decode_to_the_same_picture() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("photo.jpg");
        let img = image::DynamicImage::ImageRgba8(synthetic(320, 180)).to_rgb8();
        img.save(&src).unwrap();
        let p = prepare(&src, &dir.path().join("art"), Style::Landscape).unwrap();
        assert_eq!((p.src_width, p.src_height), (320, 180));
        let out = image::open(&p.path).unwrap().to_rgba8();
        let left = out.get_pixel(20, 100);
        assert!(left[0] > 200 && left[2] < 100, "left {left:?}");
    }

    #[test]
    fn huge_jpegs_are_refused_before_decoding() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("huge.jpg");
        // 5000×5000 is under the side limit and over the pixel limit.
        let img = image::RgbImage::new(5000, 5000);
        img.save(&src).unwrap();
        let err = prepare(&src, &dir.path().join("art"), Style::Landscape).unwrap_err();
        assert!(err.to_string().contains("too large"), "{err}");
    }

    #[test]
    fn oversized_pictures_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("huge.png");
        // A PNG header claiming 20000×20000 is enough to be refused before any allocation.
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        bytes.extend_from_slice(&20000u32.to_be_bytes());
        bytes.extend_from_slice(&20000u32.to_be_bytes());
        bytes.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
        std::fs::write(&src, &bytes).unwrap();
        assert!(prepare(&src, &dir.path().join("art"), Style::Landscape).is_err());
    }
}

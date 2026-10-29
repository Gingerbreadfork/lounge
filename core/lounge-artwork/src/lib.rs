//! Index-time artwork pipeline: decode once, crop to the tile's aspect, pre-scale to the
//! exact sizes the UI draws, write PNGs to the disk cache, and extract a palette.

use image::metadata::Orientation;
use image::{ImageDecoder, ImageFormat, RgbaImage};
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
/// How many colours the lowered Stage's light is read in, left to right.
pub const GLOW_POOLS: usize = 6;

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
    /// 160×90 thumbnail, Gaussian-blurred at index time, that the Stage scales up as a lit
    /// backdrop: a real blur with no render-time cost.
    Backdrop,
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

    /// The Stage's key art carries the picture's focal line and the light the lowered Stage shows.
    fn wants_focal(self) -> bool {
        matches!(self, Style::Hero)
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
    /// The lowered Stage's light, for key art: [`GLOW_POOLS`] colours across the focal line.
    #[serde(default)]
    pub glow: Vec<u32>,
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
    let shared = (|| -> Result<(String, (RgbaImage, (u32, u32), bool), Palette, f32, Vec<u32>)> {
        std::fs::create_dir_all(out_dir)?;
        let size = std::fs::metadata(src)?.len();
        if size > MAX_SOURCE_BYTES {
            return Err(ArtworkError::Resize(format!("{size} bytes is too large a picture")));
        }
        let bytes = std::fs::read(src)?;
        let hash = blake3::hash(&bytes).to_hex().to_string();
        let decoded = decode(&bytes, |sw, sh| largest_target(styles, sw, sh))?;
        drop(bytes);
        // Opaque pictures become JPEGs: smaller, quicker to decode, and drawn without blending.
        let opaque = decoded.opaque || decoded.image.pixels().all(|p| p.0[3] == 255);
        let (sw, sh) = decoded.source;
        let mid = reduce(decoded.image, styles, sw, sh)?;
        let (mw, mh) = mid.dimensions();
        let palette = palette_of(&mid);
        let (focal_y, glow) = if styles.iter().any(|s| s.wants_focal()) {
            let focal_y = focal_line(&mid, mw, mh)?;
            (focal_y, glow_of(&mid, mw, mh, focal_y)?)
        } else {
            (0.5, Vec::new())
        };
        Ok((hash, (mid, (sw, sh), opaque), palette, focal_y, glow))
    })();
    let (hash, (mid, (sw, sh), opaque), palette, focal_y, glow) = match shared {
        Ok(v) => v,
        Err(e) => return styles.iter().map(|_| Err(again(&e))).collect(),
    };
    let (dw, dh) = mid.dimensions();
    styles
        .iter()
        .map(|&style| {
            let (tw, th) = target_for(style, sw, sh);
            let out_path = out_dir.join(format!("{}-{}.{}", &hash[..20], style.slug(), if opaque { "jpg" } else { "png" }));
            if !out_path.exists() {
                let mut scaled = cover_resize(&mid, dw, dh, tw, th)?;
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
                        std::fs::write(&tmp, encode_jpeg(&scaled)?)?;
                    } else {
                        scaled.save_with_format(&tmp, ImageFormat::Png).map_err(writing)?;
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
                glow: if style.wants_focal() { glow.clone() } else { Vec::new() },
            })
        })
        .collect()
}

/// The size `style` draws a `sw`×`sh` source at: its target, or the source fitted inside it, held
/// to the source's own scale where the style never upsamples.
fn target_for(style: Style, sw: u32, sh: u32) -> (u32, u32) {
    let (mut tw, mut th) = style.target();
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
    (tw, th)
}

/// The decode brought down, once, to the least the styles need: scaled by exactly what the most
/// demanding style asks, so that style comes out as a plain crop and every other style, the focal
/// line and the glow are cut from a small picture instead of the whole decode. A decode no larger
/// than needed is kept as it is.
fn reduce(image: RgbaImage, styles: &[Style], sw: u32, sh: u32) -> Result<RgbaImage> {
    let (dw, dh) = image.dimensions();
    let s = scale_needed(styles, sw, sh, dw, dh);
    if !(s < 0.999) {
        return Ok(image);
    }
    let mw = ((dw as f64 * s).round() as u32).max(1);
    let mh = ((dh as f64 * s).round() as u32).max(1);
    crop_resize(&image, dw, dh, (0.0, 0.0, dw as f64, dh as f64), mw, mh)
}

/// The least a `dw`×`dh` decode of a `sw`×`sh` source must be scaled by for every style to be cut
/// from it without upsampling.
fn scale_needed(styles: &[Style], sw: u32, sh: u32, dw: u32, dh: u32) -> f64 {
    styles
        .iter()
        .map(|&style| {
            let (tw, th) = target_for(style, sw, sh);
            let (_, _, cw, ch) = cover_rect(dw, dh, tw as f64 / th as f64);
            (tw as f64 / cw).max(th as f64 / ch)
        })
        .fold(0.0, f64::max)
}

/// The widest and tallest any of `styles` draws a `sw`×`sh` source: what a decode must cover.
fn largest_target(styles: &[Style], sw: u32, sh: u32) -> (u32, u32) {
    styles.iter().map(|&s| target_for(s, sw, sh)).fold((0, 0), |(w, h), (tw, th)| (w.max(tw), h.max(th)))
}

/// The picture as a JPEG at quality 90 with 4:2:2 chroma.
fn encode_jpeg(img: &RgbaImage) -> Result<Vec<u8>> {
    let (w, h) = img.dimensions();
    let mut compressor = turbojpeg::Compressor::new().map_err(jpeg)?;
    compressor.set_quality(90).map_err(jpeg)?;
    compressor.set_subsamp(turbojpeg::Subsamp::Sub2x1).map_err(jpeg)?;
    let image = turbojpeg::Image { pixels: img.as_raw().as_slice(), width: w as usize, pitch: w as usize * 4, height: h as usize, format: turbojpeg::PixelFormat::RGBA };
    compressor.compress_to_vec(image).map_err(jpeg)
}

fn jpeg(e: turbojpeg::Error) -> ArtworkError {
    ArtworkError::Resize(format!("jpeg: {e}"))
}

/// The shared stage's error again, for one more style. An I/O error stays one, so the caller can
/// tell a full disk or a missing folder from a picture that will not decode.
fn again(e: &ArtworkError) -> ArtworkError {
    match e {
        ArtworkError::Io(io) => ArtworkError::Io(std::io::Error::new(io.kind(), io.to_string())),
        other => ArtworkError::Resize(other.to_string()),
    }
}

/// An encoder's I/O error while writing the output is the disk's, not the picture's.
fn writing(e: image::ImageError) -> ArtworkError {
    match e {
        image::ImageError::IoError(io) => ArtworkError::Io(io),
        other => ArtworkError::Image(other),
    }
}

/// A decoded source: its pixels, possibly at a fraction of the source's size, and the source's
/// own size as oriented.
struct Decoded {
    image: RgbaImage,
    source: (u32, u32),
    /// Known opaque without a scan: a JPEG has no alpha.
    opaque: bool,
}

/// Decode with hard limits: a 4 MB file that unpacks to a 500 MB bitmap is refused before
/// anything is allocated. A JPEG decodes straight to RGBA at the smallest fraction of its size,
/// in eighths, that still covers what `want` asks of its full size, so a 12 megapixel poster never
/// becomes a 48 MB bitmap for a tile. The picture comes back turned the way its EXIF orientation
/// says.
fn decode(bytes: &[u8], want: impl FnOnce(u32, u32) -> (u32, u32)) -> Result<Decoded> {
    let too_big = |w: u64, h: u64| w > MAX_SOURCE_SIDE as u64 || h > MAX_SOURCE_SIDE as u64 || w * h > MAX_SOURCE_PIXELS;
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        let mut decoder = turbojpeg::Decompressor::new().map_err(jpeg)?;
        let header = decoder.read_header(bytes).map_err(jpeg)?;
        let (w, h) = (header.width as u32, header.height as u32);
        if too_big(w as u64, h as u64) {
            return Err(ArtworkError::Resize(format!("{w}×{h} is too large a picture")));
        }
        let orientation = exif_chunk(bytes).and_then(Orientation::from_exif_chunk).unwrap_or(Orientation::NoTransforms);
        let turned = transposes(orientation);
        let source = if turned { (h, w) } else { (w, h) };
        let (ww, wh) = want(source.0, source.1);
        let factor = scaling_for((w, h), if turned { (wh, ww) } else { (ww, wh) });
        decoder.set_scaling_factor(factor).map_err(jpeg)?;
        let (dw, dh) = (factor.scale(w as usize), factor.scale(h as usize));
        let mut pixels = vec![0u8; dw * dh * 4];
        // A CMYK or YCCK picture comes out as CMYK, which libjpeg-turbo will not turn into RGB itself.
        let inks = matches!(header.colorspace, turbojpeg::Colorspace::CMYK | turbojpeg::Colorspace::YCCK);
        let format = if inks { turbojpeg::PixelFormat::CMYK } else { turbojpeg::PixelFormat::RGBA };
        let output = turbojpeg::Image { pixels: &mut pixels[..], width: dw, pitch: dw * 4, height: dh, format };
        decoder.decompress(bytes, output).map_err(jpeg)?;
        if inks {
            inks_to_rgba(&mut pixels);
        }
        let img = RgbaImage::from_raw(dw as u32, dh as u32, pixels).ok_or_else(|| ArtworkError::Resize("jpeg buffer size mismatch".into()))?;
        return Ok(Decoded { image: oriented(img, orientation), source, opaque: true });
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return decode_webp(bytes, want, too_big);
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return decode_png(bytes, too_big);
    }
    let mut probe = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?.into_decoder()?;
    let (w, h) = probe.dimensions();
    let orientation = probe.orientation().unwrap_or(Orientation::NoTransforms);
    drop(probe);
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
    let image = oriented(decoded, orientation);
    let source = image.dimensions();
    Ok(Decoded { image, source, opaque: false })
}

/// A WebP through libwebp, scaled on the way out to the smallest size, in its own proportions,
/// that still covers what `want` asks of its full size.
fn decode_webp(bytes: &[u8], want: impl FnOnce(u32, u32) -> (u32, u32), too_big: impl Fn(u64, u64) -> bool) -> Result<Decoded> {
    use libwebp_sys as webp;
    let failed = |what: &str| ArtworkError::Resize(format!("webp: {what}"));
    // SAFETY: libwebp reads the bytes it is given and writes only the structs and buffer handed to it,
    // which are sized as it requires.
    unsafe {
        let mut features: webp::WebPBitstreamFeatures = std::mem::zeroed();
        if webp::WebPGetFeatures(bytes.as_ptr(), bytes.len(), &mut features) != webp::VP8StatusCode::VP8_STATUS_OK {
            return Err(failed("unreadable header"));
        }
        let (w, h) = (features.width as u32, features.height as u32);
        if w == 0 || h == 0 || too_big(w as u64, h as u64) {
            return Err(ArtworkError::Resize(format!("{w}×{h} is too large a picture")));
        }
        let orientation = webp_exif_chunk(bytes).and_then(Orientation::from_exif_chunk).unwrap_or(Orientation::NoTransforms);
        let turned = transposes(orientation);
        let source = if turned { (h, w) } else { (w, h) };
        let (ww, wh) = want(source.0, source.1);
        let (ww, wh) = if turned { (wh, ww) } else { (ww, wh) };
        let s = (ww as f64 / w as f64).max(wh as f64 / h as f64);
        let (dw, dh) = if s < 1.0 {
            (((w as f64 * s).ceil() as u32).clamp(1, w), ((h as f64 * s).ceil() as u32).clamp(1, h))
        } else {
            (w, h)
        };
        let mut pixels = vec![0u8; dw as usize * dh as usize * 4];
        let mut config: webp::WebPDecoderConfig = std::mem::zeroed();
        if !webp::WebPInitDecoderConfig(&mut config) {
            return Err(failed("library mismatch"));
        }
        config.output.colorspace = webp::WEBP_CSP_MODE::MODE_RGBA;
        config.output.width = dw as i32;
        config.output.height = dh as i32;
        config.output.is_external_memory = 1;
        config.output.u.RGBA = webp::WebPRGBABuffer { rgba: pixels.as_mut_ptr(), stride: dw as i32 * 4, size: pixels.len() };
        if (dw, dh) != (w, h) {
            config.options.use_scaling = 1;
            config.options.scaled_width = dw as i32;
            config.options.scaled_height = dh as i32;
        }
        let status = webp::WebPDecode(bytes.as_ptr(), bytes.len(), &mut config);
        if status != webp::VP8StatusCode::VP8_STATUS_OK {
            return Err(failed(&format!("{status:?}")));
        }
        let img = RgbaImage::from_raw(dw, dh, pixels).ok_or_else(|| failed("buffer size mismatch"))?;
        Ok(Decoded { image: oriented(img, orientation), source, opaque: features.has_alpha == 0 })
    }
}

/// A PNG through libpng, whole, with its EXIF orientation read from the file's chunks.
fn decode_png(bytes: &[u8], too_big: impl Fn(u64, u64) -> bool) -> Result<Decoded> {
    let (mut w, mut h) = (0u32, 0u32);
    // SAFETY: the shim reads the bytes it is given and writes only the sizes and the buffer handed
    // to it, which holds w*h*4 bytes as it requires.
    if unsafe { lounge_png_size(bytes.as_ptr(), bytes.len(), &mut w, &mut h) } != 0 {
        return Err(ArtworkError::Resize("png: unreadable header".into()));
    }
    if too_big(w as u64, h as u64) {
        return Err(ArtworkError::Resize(format!("{w}×{h} is too large a picture")));
    }
    let mut pixels = vec![0u8; w as usize * h as usize * 4];
    if unsafe { lounge_png_decode(bytes.as_ptr(), bytes.len(), pixels.as_mut_ptr(), w, h) } != 0 {
        return Err(ArtworkError::Resize("png: will not decode".into()));
    }
    let orientation = png_exif_chunk(bytes).and_then(Orientation::from_exif_chunk).unwrap_or(Orientation::NoTransforms);
    let img = RgbaImage::from_raw(w, h, pixels).ok_or_else(|| ArtworkError::Resize("png buffer size mismatch".into()))?;
    let image = oriented(img, orientation);
    let source = image.dimensions();
    Ok(Decoded { image, source, opaque: false })
}

/// A PNG's eXIf chunk, wherever it sits before the end.
fn png_exif_chunk(bytes: &[u8]) -> Option<&[u8]> {
    let mut i = 8;
    while i + 12 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
        let kind = &bytes[i + 4..i + 8];
        if kind == b"IEND" {
            return None;
        }
        if kind == b"eXIf" {
            return bytes.get(i + 8..i + 8 + len);
        }
        i += 12 + len;
    }
    None
}

unsafe extern "C" {
    /// The PNG shim in `csrc/png.c`.
    fn lounge_png_size(data: *const u8, len: usize, w: *mut u32, h: *mut u32) -> std::os::raw::c_int;
    fn lounge_png_decode(data: *const u8, len: usize, out: *mut u8, w: u32, h: u32) -> std::os::raw::c_int;
}

/// A WebP's EXIF chunk from its RIFF container, any "Exif\0\0" tag stripped.
fn webp_exif_chunk(bytes: &[u8]) -> Option<&[u8]> {
    let mut i = 12;
    while i + 8 <= bytes.len() {
        let size = u32::from_le_bytes([bytes[i + 4], bytes[i + 5], bytes[i + 6], bytes[i + 7]]) as usize;
        let payload = bytes.get(i + 8..i + 8 + size)?;
        if &bytes[i..i + 4] == b"EXIF" {
            return Some(payload.strip_prefix(b"Exif\0\0").unwrap_or(payload));
        }
        // Chunks are padded to an even size.
        i += 8 + size + (size & 1);
    }
    None
}

/// CMYK pixels, stored inverted as Photoshop and every JPEG CMYK writer in practice does, to RGBA
/// in place: each ink scaled by the black.
fn inks_to_rgba(pixels: &mut [u8]) {
    for p in pixels.chunks_exact_mut(4) {
        let k = p[3] as u32;
        for c in 0..3 {
            p[c] = ((p[c] as u32 * k + 127) / 255) as u8;
        }
        p[3] = 255;
    }
}

/// The smallest fraction, in eighths up to a whole, at which a `w`×`h` JPEG still covers `want`.
fn scaling_for((w, h): (u32, u32), (ww, wh): (u32, u32)) -> turbojpeg::ScalingFactor {
    (1..=8)
        .map(|n| turbojpeg::ScalingFactor::new(n, 8))
        .find(|f| f.scale(w as usize) >= ww as usize && f.scale(h as usize) >= wh as usize)
        .unwrap_or(turbojpeg::ScalingFactor::ONE)
}

/// A JPEG's EXIF block, after its "Exif\0\0" tag, from the first APP1 segment before the scan.
fn exif_chunk(bytes: &[u8]) -> Option<&[u8]> {
    let mut i = 2;
    while i + 4 <= bytes.len() && bytes[i] == 0xFF {
        let marker = bytes[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if marker == 0xDA || marker == 0xD9 {
            return None;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        let payload = bytes.get(i + 4..(i + 2 + len).max(i + 4))?;
        if marker == 0xE1 && payload.starts_with(b"Exif\0\0") {
            return Some(&payload[6..]);
        }
        i += 2 + len;
    }
    None
}

/// Whether `o` turns the picture on its side, swapping its width and height.
fn transposes(o: Orientation) -> bool {
    matches!(o, Orientation::Rotate90 | Orientation::Rotate270 | Orientation::Rotate90FlipH | Orientation::Rotate270FlipH)
}

fn oriented(img: RgbaImage, orientation: Orientation) -> RgbaImage {
    let mut img = image::DynamicImage::ImageRgba8(img);
    img.apply_orientation(orientation);
    img.into_rgba8()
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

/// The lowered Stage's light: the picture read in [`GLOW_POOLS`] columns across a strip around its
/// focal line, each column's colour weighted toward its most saturated pixels so a yellow coat
/// survives instead of averaging into grey, then toned as light (DESIGN.md §5.2).
fn glow_of(img: &RgbaImage, sw: u32, sh: u32, focal_y: f32) -> Result<Vec<u32>> {
    let (left, top, cw, ch) = cover_rect(sw, sh, 16.0 / 9.0);
    let strip_h = (ch * 0.55).max(1.0);
    let strip_top = top + (focal_y as f64 * ch - strip_h / 2.0).clamp(0.0, ch - strip_h);
    let (tw, th) = (96u32, 24u32);
    let small = crop_resize(img, sw, sh, (left, strip_top, cw, strip_h), tw, th)?;
    let per = (tw as usize / GLOW_POOLS).max(1);
    let pools = (0..GLOW_POOLS)
        .map(|i| {
            let mut sum = [0f32; 3];
            let mut total = 0f32;
            for y in 0..th {
                for x in (i * per) as u32..((i + 1) * per) as u32 {
                    let p = small.get_pixel(x, y);
                    let (r, g, b) = (p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
                    let [_, a, bb] = lounge_graph::color::oklab(r, g, b);
                    let w = 0.02 + (a * a + bb * bb) * 40.0;
                    sum[0] += r * w;
                    sum[1] += g * w;
                    sum[2] += b * w;
                    total += w;
                }
            }
            let to = |v: f32| ((v / total * 255.0).round().clamp(0.0, 255.0) as u32) & 0xFF;
            lounge_graph::color::glow_tone(0xFF00_0000 | (to(sum[0]) << 16) | (to(sum[1]) << 8) | to(sum[2]))
        })
        .collect();
    Ok(pools)
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
    let half_diag2 = (cx * cx + cy * cy).max(1.0);
    // Inside six tenths of the half diagonal nothing changes, so no root is taken there.
    let inner2 = 0.36 * half_diag2;
    let half_diag = half_diag2.sqrt();
    const VELVET: [f32; 3] = [0x1A as f32, 0x10 as f32, 0x16 as f32];
    for (x, y, p) in img.enumerate_pixels_mut() {
        let dx = x as f32 - cx;
        let dy = y as f32 - cy;
        let r2 = dx * dx + dy * dy;
        if r2 <= inner2 {
            continue;
        }
        let t = ((r2.sqrt() / half_diag - 0.6) / 0.4).clamp(0.0, 1.0);
        let a = t * t * (3.0 - 2.0 * t) * 0.45;
        for c in 0..3 {
            p[c] = (p[c] as f32 * (1.0 - a) + VELVET[c] * a).round().clamp(0.0, 255.0) as u8;
        }
    }
}

fn cover_resize(src: &RgbaImage, sw: u32, sh: u32, tw: u32, th: u32) -> Result<RgbaImage> {
    let rect = cover_rect(sw, sh, tw as f64 / th as f64);
    crop_resize(src, sw, sh, rect, tw, th)
}

/// The `(left, top, cw, ch)` region of `src` at `tw`×`th`. A region already that size is copied.
/// A region at least twice the size is halved, box filtered, until it is within twice of it: a pass
/// over the pixels each time, so the resampler, whose cost goes with its source, never runs over
/// more than four times the output.
fn crop_resize(src: &RgbaImage, sw: u32, sh: u32, (left, top, cw, ch): (f64, f64, f64, f64), tw: u32, th: u32) -> Result<RgbaImage> {
    if (cw - tw as f64).abs() < 1.0 && (ch - th as f64).abs() < 1.0 {
        let (x, y) = (left.round() as u32, top.round() as u32);
        if x + tw <= sw && y + th <= sh {
            return Ok(image::imageops::crop_imm(src, x, y, tw, th).to_image());
        }
    }
    let halved;
    let (source, rect) = if cw >= 2.0 * tw as f64 && ch >= 2.0 * th as f64 {
        let (x, y) = (left.floor() as u32, top.floor() as u32);
        let mut region = image::imageops::crop_imm(src, x, y, (cw.ceil() as u32).min(sw - x), (ch.ceil() as u32).min(sh - y)).to_image();
        while region.width() >= 2 * tw && region.height() >= 2 * th {
            region = halve(&region);
        }
        halved = region;
        let (w, h) = halved.dimensions();
        (&halved, (0.0, 0.0, w as f64, h as f64))
    } else {
        (src, (left, top, cw, ch))
    };
    let (sw, sh) = source.dimensions();
    // Lanczos3 going down, Mitchell going up.
    let kernel = if tw < sw || th < sh { 0 } else { 1 };
    let mut out = vec![0u8; tw as usize * th as usize * 4];
    // SAFETY: the source buffer holds sw*sh*4 bytes and the output tw*th*4, as the C side expects.
    let rc = unsafe { lounge_resize_rgba(source.as_raw().as_ptr(), sw, sh, rect.0, rect.1, rect.2, rect.3, out.as_mut_ptr(), tw, th, kernel) };
    if rc != 0 {
        return Err(ArtworkError::Resize("out of memory resampling".into()));
    }
    RgbaImage::from_raw(tw, th, out).ok_or_else(|| ArtworkError::Resize("buffer size mismatch".into()))
}

/// A picture decoded whole, for checking the decoders across platforms.
#[doc(hidden)]
pub fn decode_whole(bytes: &[u8]) -> Result<RgbaImage> {
    decode(bytes, |w, h| (w, h)).map(|d| d.image)
}

/// The resampler on its own, for checking it across platforms.
#[doc(hidden)]
pub fn resample(src: &RgbaImage, rect: (f64, f64, f64, f64), tw: u32, th: u32) -> Result<RgbaImage> {
    let (sw, sh) = src.dimensions();
    crop_resize(src, sw, sh, rect, tw, th)
}

unsafe extern "C" {
    /// The resampler in `csrc/resize.c`.
    fn lounge_resize_rgba(src: *const u8, sw: u32, sh: u32, left: f64, top: f64, cw: f64, ch: f64, dst: *mut u8, tw: u32, th: u32, kernel: std::os::raw::c_int) -> std::os::raw::c_int;
}

/// Half the size, each pixel the mean of its 2×2 block.
fn halve(img: &RgbaImage) -> RgbaImage {
    let (w, h) = (img.width() / 2, img.height() / 2);
    let stride = img.width() as usize * 4;
    let src = img.as_raw();
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        let a = &src[2 * y * stride..2 * y * stride + w as usize * 8];
        let b = &src[(2 * y + 1) * stride..(2 * y + 1) * stride + w as usize * 8];
        for (pa, pb) in a.chunks_exact(8).zip(b.chunks_exact(8)) {
            for c in 0..4 {
                out.push(((pa[c] as u16 + pa[c + 4] as u16 + pb[c] as u16 + pb[c + 4] as u16 + 2) >> 2) as u8);
            }
        }
    }
    RgbaImage::from_raw(w, h, out).expect("halved buffer fits")
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
    fn the_focal_line_finds_the_detail() {
        // A frame that is dark except for a bright detailed stripe near the top.
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
        let p = prepare(&src, &dir.path().join("art"), Style::Hero).unwrap();
        assert!((0.15..0.32).contains(&p.focal_y), "focal {}", p.focal_y);
    }

    #[test]
    fn glow_reads_the_picture_left_to_right() {
        // Grey on the left, a yellow coat on the right: the pools follow, and the yellow survives
        // its grey surroundings.
        let mut img = RgbaImage::from_pixel(1920, 1080, Rgba([120, 120, 120, 255]));
        for y in 300..800 {
            for x in 1400..1700 {
                img.put_pixel(x, y, Rgba([212, 165, 32, 255]));
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("coat.png");
        img.save(&src).unwrap();
        let hero = prepare(&src, &dir.path().join("art"), Style::Hero).unwrap();
        assert_eq!(hero.glow.len(), GLOW_POOLS);
        let (r0, g0, b0) = lounge_graph::color::rgb_f(hero.glow[0]);
        assert!((r0 - g0).abs() < 0.02 && (g0 - b0).abs() < 0.02, "the grey side took a colour");
        let (r, g, b) = lounge_graph::color::rgb_f(hero.glow[4]);
        assert!(r > b + 0.15 && g > b + 0.1, "the coat's pool lost its yellow: {r} {g} {b}");
        // Only key art carries the light.
        let land = prepare(&src, &dir.path().join("art"), Style::Landscape).unwrap();
        assert!(land.glow.is_empty());
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
    fn a_jpeg_decodes_at_the_smallest_fraction_that_covers_its_styles() {
        let f = scaling_for((4000, 3000), (1120, 630));
        assert_eq!((f.num(), f.denom()), (3, 8));
        let f = scaling_for((2240, 1260), (1120, 630));
        assert_eq!((f.num(), f.denom()), (1, 2));
        let f = scaling_for((800, 600), (1120, 630));
        assert_eq!((f.num(), f.denom()), (1, 1));
    }

    #[test]
    fn a_big_jpeg_comes_out_at_size_and_reports_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("poster.jpg");
        image::DynamicImage::ImageRgba8(synthetic(2400, 1350)).to_rgb8().save(&src).unwrap();
        let p = prepare(&src, &dir.path().join("art"), Style::Landscape).unwrap();
        assert_eq!((p.src_width, p.src_height), (2400, 1350));
        assert_eq!((p.width, p.height), Style::Landscape.target());
        let out = image::open(&p.path).unwrap().to_rgba8();
        let (left, right) = (out.get_pixel(40, 100), out.get_pixel(430, 100));
        assert!(left[0] > 200 && left[2] < 100, "left {left:?}");
        assert!(right[0] < 60, "right {right:?}");
    }

    #[test]
    fn the_most_demanding_style_sets_the_scale() {
        let styles = [Style::Landscape, Style::Hero, Style::Poster, Style::Backdrop];
        let s = scale_needed(&styles, 1920, 1080, 1920, 1080);
        assert!((s - 1120.0 / 1920.0).abs() < 1e-6, "{s}");
        // A portrait source has to be upsampled for the Hero, so the decode is kept whole.
        let s = scale_needed(&styles, 1000, 1500, 1000, 1500);
        assert!(s > 1.0, "{s}");
        let mid = reduce(RgbaImage::new(1920, 1080), &styles, 1920, 1080).unwrap();
        assert_eq!(mid.dimensions(), (1120, 630));
    }

    #[test]
    fn a_crop_that_is_already_the_size_is_copied() {
        let src = synthetic(1120, 840);
        let out = crop_resize(&src, 1120, 840, (0.0, 105.0, 1120.0, 630.0), 1120, 630).unwrap();
        assert_eq!(out.dimensions(), (1120, 630));
        assert_eq!(out.get_pixel(10, 10), src.get_pixel(10, 115));
        assert_eq!(out.get_pixel(1110, 600), src.get_pixel(1110, 705));
    }

    #[test]
    fn halving_averages_each_block() {
        let mut img = RgbaImage::new(4, 2);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = Rgba([(x * 10 + y * 100) as u8, 0, 255, 255]);
        }
        let half = halve(&img);
        assert_eq!(half.dimensions(), (2, 1));
        // The block at (0,0): 0, 10, 100, 110 -> 55.
        assert_eq!(half.get_pixel(0, 0), &Rgba([55, 0, 255, 255]));
        assert_eq!(half.get_pixel(1, 0), &Rgba([75, 0, 255, 255]));
        // A big region reaches the target through halving and still lands at size.
        let big = synthetic(2000, 1200);
        let out = crop_resize(&big, 2000, 1200, (0.0, 0.0, 2000.0, 1200.0), 160, 90).unwrap();
        assert_eq!(out.dimensions(), (160, 90));
        assert!(out.get_pixel(20, 45)[0] > 200 && out.get_pixel(140, 45)[0] < 60);
    }

    #[test]
    fn resampling_keeps_a_flat_colour_and_a_one_to_one_copy_exact() {
        let flat = RgbaImage::from_pixel(300, 200, Rgba([77, 150, 33, 255]));
        for (tw, th) in [(100u32, 66u32), (600, 400), (300, 200)] {
            let out = crop_resize(&flat, 300, 200, (0.0, 0.0, 300.0, 200.0), tw, th).unwrap();
            assert!(out.pixels().all(|p| *p == Rgba([77, 150, 33, 255])), "{tw}x{th}");
        }
        let detailed = synthetic(300, 200);
        let same = crop_resize(&detailed, 300, 200, (0.3, 0.2, 300.0, 200.0), 300, 200).unwrap();
        assert_eq!(same.as_raw(), detailed.as_raw());
    }

    #[test]
    fn resampling_keeps_a_gradient_in_order() {
        let mut img = RgbaImage::new(512, 8);
        for (x, _, p) in img.enumerate_pixels_mut() {
            *p = Rgba([(x / 2) as u8, 0, 0, 255]);
        }
        for tw in [128u32, 200, 1024] {
            let out = crop_resize(&img, 512, 8, (0.0, 0.0, 512.0, 8.0), tw, 4).unwrap();
            let row: Vec<u8> = (0..tw).map(|x| out.get_pixel(x, 2)[0]).collect();
            assert!(row.windows(2).all(|w| w[0] <= w[1]), "{tw}: {row:?}");
            assert!(row[0] < 4 && row[tw as usize - 1] > 250, "{tw}: ends {} {}", row[0], row[tw as usize - 1]);
        }
    }

    #[test]
    fn a_webp_comes_out_right_and_a_big_one_decodes_small() {
        let dir = tempfile::tempdir().unwrap();
        for (w, h) in [(320u32, 180u32), (2400, 1350)] {
            let src = dir.path().join(format!("{w}.webp"));
            let img = image::DynamicImage::ImageRgba8(synthetic(w, h)).to_rgb8();
            img.write_with_encoder(image::codecs::webp::WebPEncoder::new_lossless(std::fs::File::create(&src).unwrap())).unwrap();
            let p = prepare(&src, &dir.path().join("art"), Style::Landscape).unwrap();
            assert_eq!((p.src_width, p.src_height), (w, h));
            assert_eq!((p.width, p.height), Style::Landscape.target());
            assert!(p.path.ends_with(".jpg"), "{}", p.path);
            let out = image::open(&p.path).unwrap().to_rgba8();
            let (left, right) = (out.get_pixel(40, 100), out.get_pixel(430, 100));
            assert!(left[0] > 200 && left[2] < 100, "{w}: left {left:?}");
            assert!(right[0] < 60, "{w}: right {right:?}");
        }
    }

    #[test]
    fn a_webp_s_exif_chunk_is_found_past_padded_chunks() {
        let mut riff = b"RIFF\0\0\0\0WEBP".to_vec();
        riff.extend_from_slice(b"VP8X");
        riff.extend_from_slice(&3u32.to_le_bytes());
        riff.extend_from_slice(&[1, 2, 3, 0]);
        riff.extend_from_slice(b"EXIF");
        let exif = exif_orientation(6);
        let payload = [b"Exif\0\0".as_slice(), &exif].concat();
        riff.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        riff.extend_from_slice(&payload);
        assert_eq!(webp_exif_chunk(&riff), Some(exif.as_slice()));
        assert_eq!(Orientation::from_exif_chunk(webp_exif_chunk(&riff).unwrap()), Some(Orientation::Rotate90));
    }

    #[test]
    fn a_cmyk_jpeg_is_read_through_its_inks() {
        // Cyan ink on the left, no ink on the right, in the inverted convention the files use.
        let (w, h) = (320u32, 180u32);
        let mut cmyk = vec![255u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w / 2 {
                cmyk[((y * w + x) * 4) as usize] = 0;
            }
        }
        let mut compressor = turbojpeg::Compressor::new().unwrap();
        compressor.set_quality(95).unwrap();
        let data = compressor
            .compress_to_vec(turbojpeg::Image { pixels: &cmyk[..], width: w as usize, pitch: w as usize * 4, height: h as usize, format: turbojpeg::PixelFormat::CMYK })
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("inks.jpg");
        std::fs::write(&src, &data).unwrap();
        let p = prepare(&src, &dir.path().join("art"), Style::Landscape).unwrap();
        let out = image::open(&p.path).unwrap().to_rgba8();
        let (left, right) = (out.get_pixel(40, 100), out.get_pixel(430, 100));
        assert!(left[0] < 40 && left[1] > 200 && left[2] > 200, "left {left:?}");
        assert!(right[0] > 230 && right[1] > 230 && right[2] > 230, "right {right:?}");
    }

    #[test]
    fn pngs_of_every_depth_come_out_as_rgba() {
        let dir = tempfile::tempdir().unwrap();
        let rgba = synthetic(320, 180);
        let sources: Vec<(&str, image::DynamicImage)> = vec![
            ("rgb16.png", image::DynamicImage::ImageRgba8(rgba.clone()).to_rgb16().into()),
            ("gray.png", image::DynamicImage::ImageLuma8(image::GrayImage::from_fn(320, 180, |x, _| image::Luma([if x < 160 { 200 } else { 20 }])))),
            ("rgba.png", image::DynamicImage::ImageRgba8(rgba)),
        ];
        for (name, img) in sources {
            let src = dir.path().join(name);
            img.save(&src).unwrap();
            let p = prepare(&src, &dir.path().join("art"), Style::Landscape).unwrap();
            assert_eq!((p.src_width, p.src_height), (320, 180), "{name}");
            let out = image::open(&p.path).unwrap().to_rgba8();
            let (left, right) = (out.get_pixel(40, 100), out.get_pixel(430, 100));
            assert!(left[0] > 180 && right[0] < 60, "{name}: left {left:?} right {right:?}");
        }
    }

    /// A minimal big-endian EXIF block holding only an orientation.
    fn exif_orientation(value: u8) -> Vec<u8> {
        vec![b'M', b'M', 0, 42, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, value, 0, 0, 0, 0, 0, 0]
    }

    /// Orange on the left, dark on the right, saved with EXIF orientation 6: shown turned a
    /// quarter clockwise, so the orange should end up on top.
    fn sideways<E: image::ImageEncoder>(path: &Path, encoder: impl FnOnce(std::fs::File) -> E) {
        let img = image::DynamicImage::ImageRgba8(synthetic(64, 32)).to_rgb8();
        let mut encoder = encoder(std::fs::File::create(path).unwrap());
        encoder.set_exif_metadata(exif_orientation(6)).unwrap();
        encoder.write_image(img.as_raw(), 64, 32, image::ExtendedColorType::Rgb8).unwrap();
    }

    fn assert_stood_up(src: &Path, dir: &Path) {
        let p = prepare(src, &dir.join("art"), Style::WallpaperFit).unwrap();
        assert_eq!((p.src_width, p.src_height), (32, 64));
        assert_eq!((p.width, p.height), (32, 64));
        let out = image::open(&p.path).unwrap().to_rgba8();
        assert_eq!(out.dimensions(), (32, 64));
        let (top, bottom) = (out.get_pixel(16, 8), out.get_pixel(16, 56));
        assert!(top[0] > 200 && top[2] < 100, "top {top:?}");
        assert!(bottom[0] < 60, "bottom {bottom:?}");
    }

    #[test]
    fn a_sideways_jpeg_is_stood_up() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("portrait.jpg");
        sideways(&src, |f| image::codecs::jpeg::JpegEncoder::new_with_quality(f, 95));
        assert_stood_up(&src, dir.path());
    }

    #[test]
    fn a_sideways_webp_is_stood_up() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("portrait.webp");
        sideways(&src, image::codecs::webp::WebPEncoder::new_lossless);
        assert_stood_up(&src, dir.path());
    }

    #[test]
    fn a_sideways_png_is_stood_up() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("portrait.png");
        sideways(&src, image::codecs::png::PngEncoder::new);
        assert_stood_up(&src, dir.path());
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
    fn a_folder_that_cannot_be_made_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("banner.png");
        synthetic(64, 36).save(&src).unwrap();
        // A file where the output folder's parent should be.
        let out = src.join("art");
        let results = prepare_many(&src, &out, &[Style::AppBanner, Style::Icon]);
        assert_eq!(results.len(), 2);
        for r in results {
            assert!(matches!(r, Err(ArtworkError::Io(_))), "{r:?}");
        }
    }

    #[test]
    fn a_missing_source_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = prepare(&dir.path().join("gone.png"), &dir.path().join("art"), Style::AppBanner).unwrap_err();
        assert!(matches!(err, ArtworkError::Io(_)), "{err:?}");
    }

    #[test]
    fn a_picture_that_will_not_decode_is_not_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("noise.png");
        std::fs::write(&src, b"not a picture at all").unwrap();
        let err = prepare(&src, &dir.path().join("art"), Style::AppBanner).unwrap_err();
        assert!(!matches!(err, ArtworkError::Io(_)), "{err:?}");
    }

    #[cfg(unix)]
    #[test]
    fn an_output_that_cannot_be_written_is_an_io_error() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("logo.png");
        RgbaImage::from_pixel(64, 64, Rgba([200, 40, 40, 128])).save(&src).unwrap();
        let out = dir.path().join("art");
        std::fs::create_dir(&out).unwrap();
        std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o555)).unwrap();
        // Permissions do not hold back root.
        if std::fs::write(out.join("probe"), b"").is_ok() {
            return;
        }
        let err = prepare(&src, &out, Style::Logo).unwrap_err();
        std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(err, ArtworkError::Io(_)), "{err:?}");
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

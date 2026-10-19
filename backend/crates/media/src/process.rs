//! 图片处理（plan 第 12 节）：按内容判断真实格式 → 解码（按 EXIF 方向转正）→ 缩放 → WebP（质量 80）。
//!
//! 解码出像素再重新编码，原图的 EXIF（含 GPS）、XMP、ICC 等元数据都不会带到输出里；输出只有 WebP 图像数据。
//! 动图（GIF、动态 WebP）逐帧缩放后输出动态 WebP。
//!
//! 处理是 CPU 密集的同步代码，调用方放在 `spawn_blocking` 里并限制并发。plan 定的 libvips 在这台机器上没有，
//! 且无法做 musl 静态链接，所以先实现 [`RustProcessor`]（image + libwebp）；换 libvips 只需另一个 [`ImageProcessor`]。

use std::io::Cursor;

use image::codecs::gif::GifDecoder;
use image::codecs::webp::WebPDecoder;
use image::imageops::FilterType;
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageReader, Limits, RgbaImage};

/// 输出的三种尺寸：最长边不超过这些像素（不放大，原图更小时只出原尺寸）。
pub const SIZES: [u32; 3] = [320, 1280, 2560];
/// WebP 质量。
pub const QUALITY: f32 = 80.0;
/// 解码上限：单边像素与解码内存（防解压炸弹）。
const MAX_SIDE: u32 = 12_000;
const MAX_ALLOC: u64 = 256 * 1024 * 1024;
/// 动图上限：帧数与「帧数 × 像素」。
const MAX_FRAMES: usize = 300;
const MAX_ANIMATION_PIXELS: u64 = 120_000_000;

/// 按文件内容判断出的格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Jpeg,
    Png,
    Gif,
    WebP,
    Heic,
}

impl Format {
    pub fn mime(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Gif => "image/gif",
            Self::WebP => "image/webp",
            Self::Heic => "image/heic",
        }
    }

    /// 允许上传的声明类型（浏览器给出的 `Content-Type`）。
    pub fn from_mime(mime: &str) -> Option<Self> {
        Some(match mime {
            "image/jpeg" => Self::Jpeg,
            "image/png" => Self::Png,
            "image/gif" => Self::Gif,
            "image/webp" => Self::WebP,
            "image/heic" | "image/heif" => Self::Heic,
            _ => return None,
        })
    }
}

/// 看文件头判断真实格式，不信任文件名和声明的类型。
pub fn sniff(b: &[u8]) -> Option<Format> {
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(Format::Jpeg);
    }
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(Format::Png);
    }
    if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        return Some(Format::Gif);
    }
    if b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        return Some(Format::WebP);
    }
    // ISO BMFF：`ftyp` 盒子的主品牌
    if b.len() >= 12
        && &b[4..8] == b"ftyp"
        && matches!(
            &b[8..12],
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1"
        )
    {
        return Some(Format::Heic);
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProcessError {
    /// 不是支持的图片，或这个处理器解不了的格式（[`RustProcessor`] 不支持 HEIC）。
    #[error("unsupported")]
    Unsupported,
    /// 尺寸或帧数超过上限。
    #[error("too_large")]
    TooLarge,
    #[error("corrupt")]
    Corrupt,
}

impl ProcessError {
    /// 写进 `uploads.error` 的代码。
    pub fn code(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::TooLarge => "too_large",
            Self::Corrupt => "corrupt",
        }
    }
}

/// 一种尺寸的输出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    pub width: u32,
    pub height: u32,
    pub webp: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Processed {
    /// 转正后的原图尺寸。
    pub width: u32,
    pub height: u32,
    pub animated: bool,
    /// 从小到大，尺寸各不相同。
    pub variants: Vec<Variant>,
}

/// 图片处理后端。
pub trait ImageProcessor: Send + Sync {
    fn process(&self, bytes: &[u8]) -> Result<Processed, ProcessError>;
}

/// image（解码、缩放）+ libwebp（编码）。
#[derive(Debug, Clone, Copy, Default)]
pub struct RustProcessor;

impl ImageProcessor for RustProcessor {
    fn process(&self, bytes: &[u8]) -> Result<Processed, ProcessError> {
        let format = sniff(bytes).ok_or(ProcessError::Unsupported)?;
        if let Some(frames) = animation(format, bytes)? {
            return encode_animation(&frames);
        }
        let img = decode_still(bytes)?;
        let (width, height) = (img.width(), img.height());
        let mut variants: Vec<Variant> = Vec::new();
        for (w, h) in target_sizes(width, height) {
            let resized = if (w, h) == (width, height) {
                img.clone()
            } else {
                img.resize_exact(w, h, FilterType::Lanczos3)
            };
            variants.push(Variant {
                width: w,
                height: h,
                webp: encode_still(&resized)?,
            });
        }
        Ok(Processed {
            width,
            height,
            animated: false,
            variants,
        })
    }
}

fn limits() -> Limits {
    let mut l = Limits::default();
    l.max_image_width = Some(MAX_SIDE);
    l.max_image_height = Some(MAX_SIDE);
    l.max_alloc = Some(MAX_ALLOC);
    l
}

fn decode_error(e: image::ImageError) -> ProcessError {
    match e {
        image::ImageError::Limits(_) => ProcessError::TooLarge,
        image::ImageError::Unsupported(_) => ProcessError::Unsupported,
        _ => ProcessError::Corrupt,
    }
}

/// 静态图：解码并按 EXIF 方向转正（之后 EXIF 不再使用）。
fn decode_still(bytes: &[u8]) -> Result<DynamicImage, ProcessError> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| ProcessError::Corrupt)?;
    reader.limits(limits());
    let mut decoder = reader.into_decoder().map_err(decode_error)?;
    let orientation = decoder.orientation().map_err(decode_error)?;
    let mut img = DynamicImage::from_decoder(decoder).map_err(decode_error)?;
    img.apply_orientation(orientation);
    Ok(img)
}

/// 动图的帧（已合成为整幅画布）与每帧时长（毫秒）；不是动图返回 `None`。
fn animation(format: Format, bytes: &[u8]) -> Result<Option<Vec<(RgbaImage, u32)>>, ProcessError> {
    let frames = match format {
        Format::Gif => {
            let mut d = GifDecoder::new(Cursor::new(bytes)).map_err(decode_error)?;
            d.set_limits(limits()).map_err(decode_error)?;
            collect_frames(d.into_frames())?
        }
        Format::WebP => {
            let mut d = WebPDecoder::new(Cursor::new(bytes)).map_err(decode_error)?;
            if !d.has_animation() {
                return Ok(None);
            }
            d.set_limits(limits()).map_err(decode_error)?;
            collect_frames(d.into_frames())?
        }
        _ => return Ok(None),
    };
    // 单帧 GIF 按静态图处理
    Ok((frames.len() > 1).then_some(frames))
}

fn collect_frames(frames: image::Frames<'_>) -> Result<Vec<(RgbaImage, u32)>, ProcessError> {
    let mut out = Vec::new();
    let mut pixels: u64 = 0;
    for frame in frames {
        let frame = frame.map_err(decode_error)?;
        let (num, den) = frame.delay().numer_denom_ms();
        let ms = num.checked_div(den).unwrap_or(100);
        let buf = frame.into_buffer();
        pixels += u64::from(buf.width()) * u64::from(buf.height());
        if out.len() >= MAX_FRAMES || pixels > MAX_ANIMATION_PIXELS {
            return Err(ProcessError::TooLarge);
        }
        out.push((buf, ms));
    }
    Ok(out)
}

/// 最长边缩到 [`SIZES`] 各档（不放大），去掉重复尺寸。
fn target_sizes(width: u32, height: u32) -> Vec<(u32, u32)> {
    let long = width.max(height).max(1);
    let mut out: Vec<(u32, u32)> = Vec::new();
    for limit in SIZES {
        let size = if long <= limit {
            (width, height)
        } else {
            let scale = |v: u32| {
                u32::try_from(u64::from(v) * u64::from(limit) / u64::from(long))
                    .unwrap_or(1)
                    .max(1)
            };
            (scale(width), scale(height))
        };
        if out.last() != Some(&size) {
            out.push(size);
        }
    }
    out
}

fn config() -> Result<webp::WebPConfig, ProcessError> {
    let mut c = webp::WebPConfig::new().map_err(|()| ProcessError::Corrupt)?;
    c.quality = QUALITY;
    c.lossless = 0;
    Ok(c)
}

fn encode_still(img: &DynamicImage) -> Result<Vec<u8>, ProcessError> {
    let cfg = config()?;
    let (w, h) = (img.width(), img.height());
    let mem = if img.color().has_alpha() {
        let rgba = img.to_rgba8();
        webp::Encoder::from_rgba(rgba.as_raw(), w, h).encode_advanced(&cfg)
    } else {
        let rgb = img.to_rgb8();
        webp::Encoder::from_rgb(rgb.as_raw(), w, h).encode_advanced(&cfg)
    }
    .map_err(|_| ProcessError::Corrupt)?;
    Ok(mem.to_vec())
}

fn encode_animation(frames: &[(RgbaImage, u32)]) -> Result<Processed, ProcessError> {
    let (width, height) = frames
        .first()
        .map(|(f, _)| (f.width(), f.height()))
        .ok_or(ProcessError::Corrupt)?;
    let cfg = config()?;
    let mut variants = Vec::new();
    for (w, h) in target_sizes(width, height) {
        let scaled: Vec<RgbaImage> = frames
            .iter()
            .map(|(f, _)| {
                if (w, h) == (f.width(), f.height()) {
                    f.clone()
                } else {
                    image::imageops::resize(f, w, h, FilterType::Triangle)
                }
            })
            .collect();
        let mut enc = webp::AnimEncoder::new(w, h, &cfg);
        let mut t: i32 = 0;
        for (img, (_, ms)) in scaled.iter().zip(frames) {
            enc.add_frame(webp::AnimFrame::from_rgba(img.as_raw(), w, h, t));
            t = t.saturating_add(i32::try_from(*ms).unwrap_or(i32::MAX));
        }
        let mem = enc.try_encode().map_err(|_| ProcessError::Corrupt)?;
        variants.push(Variant {
            width: w,
            height: h,
            webp: mem.to_vec(),
        });
    }
    Ok(Processed {
        width,
        height,
        animated: true,
        variants,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_never_upscale_and_dedupe() {
        assert_eq!(target_sizes(200, 100), vec![(200, 100)]);
        assert_eq!(target_sizes(1000, 500), vec![(320, 160), (1000, 500)]);
        assert_eq!(
            target_sizes(3000, 6000),
            vec![(160, 320), (640, 1280), (1280, 2560)]
        );
    }

    #[test]
    fn sniff_ignores_declared_type() {
        assert_eq!(sniff(b"\xFF\xD8\xFF\xE0rest"), Some(Format::Jpeg));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(Format::WebP));
        assert_eq!(sniff(b"\0\0\0\x18ftypheic\0\0\0\0"), Some(Format::Heic));
        assert_eq!(sniff(b"<svg xmlns="), None);
    }
}

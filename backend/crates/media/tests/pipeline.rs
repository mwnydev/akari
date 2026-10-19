//! 图片处理管线：真实格式判断、EXIF（GPS）清除、方向转正、三种尺寸、动图。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use std::io::Cursor;

use forum_media::{ImageProcessor, ProcessError, RustProcessor};
use image::codecs::gif::GifEncoder;
use image::{Delay, Frame, ImageFormat, Rgba, RgbaImage};

/// 64×48 的 JPEG，SOI 之后是带 GPS（北纬 35°41'22.80"）与方向 6（顺时针转 90°）的 EXIF 段，
/// 即手机照片的典型情况（由 image 编码后手工插入大端 TIFF 段生成）。
const FIXTURE: &[u8] = include_bytes!("fixtures/gps-orientation6.jpg");

fn gps_fields(bytes: &[u8]) -> Option<usize> {
    let exif = exif::Reader::new()
        .read_from_container(&mut Cursor::new(bytes))
        .ok()?;
    Some(
        exif.fields()
            .filter(|f| f.ifd_num == exif::In::PRIMARY && f.tag.context() == exif::Context::Gps)
            .count(),
    )
}

#[test]
fn jpeg_exif_gps_is_stripped_and_orientation_applied() {
    let input = FIXTURE;
    assert_eq!(gps_fields(input), Some(2), "fixture carries GPS");

    let out = RustProcessor.process(input).unwrap();
    // 方向 6：64×48 转正为 48×64；原图比 320 小，只出一种尺寸
    assert_eq!((out.width, out.height), (48, 64));
    assert_eq!(out.variants.len(), 1);
    let webp = &out.variants[0].webp;
    assert_eq!(&webp[..4], b"RIFF");
    assert_eq!(&webp[8..12], b"WEBP");
    assert!(
        !webp.windows(4).any(|w| w == b"EXIF" || w == b"XMP "),
        "no metadata chunks"
    );
    assert_eq!(gps_fields(webp), None, "output has no EXIF at all");
    let decoded = image::load_from_memory(webp).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (48, 64));
}

#[test]
fn large_png_gets_three_sizes_and_keeps_alpha() {
    let img = RgbaImage::from_fn(3000, 1500, |x, _| Rgba([255, 0, 0, (x % 256) as u8]));
    let mut png = Vec::new();
    img.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .unwrap();
    let out = RustProcessor.process(&png).unwrap();
    let sizes: Vec<(u32, u32)> = out.variants.iter().map(|v| (v.width, v.height)).collect();
    assert_eq!(sizes, vec![(320, 160), (1280, 640), (2560, 1280)]);
    let small = image::load_from_memory(&out.variants[0].webp).unwrap();
    assert!(small.color().has_alpha());
}

#[test]
fn animated_gif_becomes_animated_webp() {
    let mut gif = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut gif);
        for i in 0..3u8 {
            let f = RgbaImage::from_pixel(40, 30, Rgba([i * 80, 0, 0, 255]));
            enc.encode_frame(Frame::from_parts(
                f,
                0,
                0,
                Delay::from_numer_denom_ms(100, 1),
            ))
            .unwrap();
        }
    }
    let out = RustProcessor.process(&gif).unwrap();
    assert!(out.animated);
    assert_eq!((out.width, out.height), (40, 30));
    let webp = &out.variants[0].webp;
    assert!(webp.windows(4).any(|w| w == b"ANIM"), "animated WebP");
}

#[test]
fn rejects_non_images_and_disguised_files() {
    assert_eq!(
        RustProcessor.process(b"<svg onload=alert(1)>"),
        Err(ProcessError::Unsupported)
    );
    // HEIC 判断得出，但这个处理器解不了
    assert_eq!(
        RustProcessor.process(b"\0\0\0\x18ftypheic\0\0\0\0mif1heic"),
        Err(ProcessError::Unsupported)
    );
    // 声称是 JPEG 但内容坏了
    assert_eq!(
        RustProcessor.process(b"\xFF\xD8\xFF\xE0garbage"),
        Err(ProcessError::Corrupt)
    );
}

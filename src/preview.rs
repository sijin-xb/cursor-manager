//! Xcursor 文件 → gpui RenderImage。

use std::fs;
use std::path::Path;
use std::sync::Arc;

use gpui_kit::RenderImage;
use image::{Frame, RgbaImage};
use xcursor::parser::{Image as XImage, parse_xcursor};

/// 解析一个 Xcursor 文件的所有帧。
pub fn parse_file(path: &Path) -> Option<Vec<XImage>> {
    let bytes = fs::read(path).ok()?;
    parse_xcursor(&bytes)
}

/// 选出名义尺寸最接近 target 的帧。
pub fn pick(images: &[XImage], target: u32) -> Option<XImage> {
    images
        .iter()
        .min_by_key(|im| (i64::from(im.size) - i64::from(target)).abs())
        .cloned()
}

/// RGBA（直通 alpha）→ BGRA（预乘 alpha），gpui 渲染管线要求的字节序。
fn to_bgra_pa(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        let (r, g, b, a) = (u16::from(px[0]), u16::from(px[1]), u16::from(px[2]), u16::from(px[3]));
        out.push(((b * a + 127) / 255) as u8);
        out.push(((g * a + 127) / 255) as u8);
        out.push(((r * a + 127) / 255) as u8);
        out.push(a as u8);
    }
    out
}

pub fn to_render_image(x: &XImage) -> Option<Arc<RenderImage>> {
    let buf = RgbaImage::from_raw(x.width, x.height, to_bgra_pa(&x.pixels_rgba))?;
    Some(Arc::new(RenderImage::new(vec![Frame::new(buf)])))
}

/// 一个形状的动画帧序列（同一名义尺寸的多帧按文件顺序播放）。
/// 每帧：(渲染图, 显示边长, 热点, 名义尺寸)。
pub struct AnimShape {
    pub frames: Vec<(Arc<RenderImage>, f32, (u32, u32), u32)>,
    pub delays_ms: Vec<u32>,
}

/// XCursor 同一名义尺寸可以有多个图像块 = 动画帧。
/// 取名义尺寸最接近 target 的那一组。
pub fn build_anim(images: Vec<XImage>, target: u32) -> Option<AnimShape> {
    use std::collections::BTreeMap;
    let mut groups: BTreeMap<u32, Vec<XImage>> = BTreeMap::new();
    for im in images {
        groups.entry(im.size).or_default().push(im);
    }
    let (_, group) = groups
        .into_iter()
        .min_by_key(|(s, _)| (i64::from(*s) - i64::from(target)).abs())?;
    let mut frames = Vec::new();
    let mut delays = Vec::new();
    for x in group {
        if x.width == 0 || x.height == 0 {
            continue;
        }
        let disp = (x.width as f32).min(64.);
        let hotspot = (x.xhot.min(x.width - 1), x.yhot.min(x.height - 1));
        frames.push((to_render_image(&x)?, disp, hotspot, x.width));
        delays.push(x.delay.max(10));
    }
    if frames.is_empty() {
        return None;
    }
    Some(AnimShape {
        frames,
        delays_ms: delays,
    })
}

/// 从磁盘图片文件（png/jpeg 等）构建 RenderImage（缩略图用）。
pub fn image_from_file(path: &Path) -> Option<Arc<RenderImage>> {
    let img = image::open(path).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    if w == 0 || h == 0 {
        return None;
    }
    let buf = RgbaImage::from_raw(w, h, to_bgra_pa(rgba.as_raw()))?;
    Some(Arc::new(RenderImage::new(vec![Frame::new(buf)])))
}

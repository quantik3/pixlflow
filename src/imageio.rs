//! Reading and writing image files. `-` as a path means stdin/stdout (PNG on output).

use crate::{color, Rgb};
use image::{imageops::FilterType, ColorType, DynamicImage, GrayImage, ImageFormat, RgbImage, RgbaImage};
use std::io::{Cursor, Read, Write};
use std::path::Path;

pub struct Picture {
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<Rgb>,
    /// Kept in place and written back; never sorted.
    pub alpha: Option<Vec<u8>>,
    /// The file had more than 8 bits per channel and was reduced to 8.
    pub deep: bool,
}

fn is_stdio(path: &Path) -> bool {
    path == Path::new("-")
}

pub fn read(path: &Path) -> Result<DynamicImage, String> {
    let name = path.display();
    let decoded = if is_stdio(path) {
        let mut buf = Vec::new();
        std::io::stdin().read_to_end(&mut buf).map_err(|e| format!("cannot read stdin: {e}"))?;
        image::load_from_memory(&buf)
    } else {
        image::ImageReader::open(path)
            .and_then(|r| r.with_guessed_format())
            .map_err(|e| format!("cannot open '{name}': {e}"))?
            .decode()
    };
    decoded.map_err(|e| format!("cannot read image '{name}': {e}"))
}

pub fn load(path: &Path) -> Result<Picture, String> {
    let img = read(path)?;
    let deep = !matches!(img.color(), ColorType::L8 | ColorType::La8 | ColorType::Rgb8 | ColorType::Rgba8);
    let alpha = img.color().has_alpha().then(|| img.to_rgba8().pixels().map(|p| p[3]).collect());
    Ok(Picture {
        w: img.width() as usize,
        h: img.height() as usize,
        rgb: img.to_rgb8().pixels().map(|p| p.0).collect(),
        alpha,
        deep,
    })
}

/// Brightness of an image file. With `resize` it is scaled (bilinear) to w×h; without, sizes must match.
pub fn load_gray(path: &Path, w: usize, h: usize, resize: bool) -> Result<Vec<u8>, String> {
    let mut img = read(path)?;
    if (img.width() as usize, img.height() as usize) != (w, h) {
        if !resize {
            return Err(format!(
                "'{}' is {}x{}, but the image is {w}x{h}",
                path.display(),
                img.width(),
                img.height()
            ));
        }
        img = img.resize_exact(w as u32, h as u32, FilterType::Triangle);
    }
    Ok(img.to_rgb8().pixels().map(|p| color::brightness(p.0) as u8).collect())
}

pub fn resize_rgb(px: &[Rgb], w: usize, h: usize, nw: usize, nh: usize) -> Vec<Rgb> {
    let img = RgbImage::from_raw(w as u32, h as u32, px.concat()).expect("buffer matches size");
    image::imageops::resize(&img, nw as u32, nh as u32, FilterType::Triangle).pixels().map(|p| p.0).collect()
}

pub fn resize_gray(px: &[u8], w: usize, h: usize, nw: usize, nh: usize) -> Vec<u8> {
    let img = GrayImage::from_raw(w as u32, h as u32, px.to_vec()).expect("buffer matches size");
    image::imageops::resize(&img, nw as u32, nh as u32, FilterType::Triangle).into_raw()
}

pub fn save(path: &Path, w: usize, h: usize, px: &[Rgb], alpha: Option<&[u8]>) -> Result<(), String> {
    let name = path.display();
    let fmt = if is_stdio(path) {
        ImageFormat::Png
    } else {
        ImageFormat::from_path(path)
            .map_err(|_| format!("unknown output format for '{name}' (use .png, .jpg, .webp or .gif)"))?
    };
    let (w, h) = (w as u32, h as u32);
    let img = match alpha {
        Some(a) if fmt != ImageFormat::Jpeg => DynamicImage::ImageRgba8(
            RgbaImage::from_raw(w, h, px.iter().zip(a).flat_map(|(p, &a)| [p[0], p[1], p[2], a]).collect())
                .expect("buffer matches size"),
        ),
        _ => DynamicImage::ImageRgb8(RgbImage::from_raw(w, h, px.concat()).expect("buffer matches size")),
    };
    let img = if fmt == ImageFormat::Gif { DynamicImage::ImageRgba8(img.to_rgba8()) } else { img };
    if is_stdio(path) {
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, fmt).map_err(|e| format!("cannot encode PNG: {e}"))?;
        std::io::stdout().write_all(buf.get_ref()).map_err(|e| format!("cannot write stdout: {e}"))
    } else {
        img.save_with_format(path, fmt).map_err(|e| format!("cannot save '{name}': {e}"))
    }
}

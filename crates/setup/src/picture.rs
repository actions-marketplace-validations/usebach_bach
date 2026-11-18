//! Cutting a picture to a frame's shape. GPUI rounds an image's corners on
//! the image's own bounds, and a "cover" fit makes those bounds bigger than
//! the frame, so the rounding falls outside it. A picture cut to the
//! frame's shape fills it exactly and keeps its rounded corners.

use std::sync::Arc;

use gpui::{Image, ImageFormat};

/// `picture` cut around its middle to `width` over `height`, or itself
/// when it already has that shape or can't be read.
pub fn fit(picture: &Arc<Image>, width: f32, height: f32) -> Arc<Image> {
    match cut(picture, width, height) {
        Some(cut) => Arc::new(cut),
        None => picture.clone(),
    }
}

fn cut(picture: &Image, width: f32, height: f32) -> Option<Image> {
    if !(width > 1.0 && height > 1.0) {
        return None;
    }
    let decoded = image::load_from_memory(picture.bytes()).ok()?;
    let (w, h) = (decoded.width() as f32, decoded.height() as f32);
    let wanted = width / height;
    let (cut_w, cut_h) = if w / h > wanted { (h * wanted, h) } else { (w, w / wanted) };
    if (cut_w - w).abs() < 2.0 && (cut_h - h).abs() < 2.0 {
        return None;
    }
    let (x, y) = (((w - cut_w) / 2.0) as u32, ((h - cut_h) / 2.0) as u32);
    let cropped = decoded.crop_imm(x, y, (cut_w as u32).max(1), (cut_h as u32).max(1));
    // BMP: quick to write and to read again, and the picture stays in memory only.
    let mut bytes = std::io::Cursor::new(Vec::new());
    cropped.to_rgba8().write_to(&mut bytes, image::ImageFormat::Bmp).ok()?;
    Some(Image::from_bytes(ImageFormat::Bmp, bytes.into_inner()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Arc<Image> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::RgbaImage::new(width, height).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        Arc::new(Image::from_bytes(ImageFormat::Png, bytes.into_inner()))
    }

    #[test]
    fn a_picture_is_cut_to_the_frame() {
        let cut = fit(&png(400, 200), 100.0, 100.0);
        let decoded = image::load_from_memory(cut.bytes()).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (200, 200));
        let tall = image::load_from_memory(fit(&png(300, 300), 100.0, 200.0).bytes()).unwrap();
        assert_eq!((tall.width(), tall.height()), (150, 300));
    }

    #[test]
    fn a_fitting_or_unreadable_picture_is_left_alone() {
        let same = png(200, 100);
        assert!(Arc::ptr_eq(&fit(&same, 400.0, 200.0), &same));
        let broken = Arc::new(Image::from_bytes(ImageFormat::Png, b"not a png".to_vec()));
        assert!(Arc::ptr_eq(&fit(&broken, 100.0, 100.0), &broken));
    }
}

//! Pictures the motor draws itself, as PNG files an `image_source` shows:
//! the circle mask of the camera. The words (the cards) are libobs's own
//! text source.

use std::path::{Path, PathBuf};

/// Where the PNGs go: beside the socket.
pub fn folder() -> PathBuf {
    let dir = remuxd_domain::socket::default_path().with_file_name("obs");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// A mask the size given: white inside the circle that fits it, clear
/// outside. What `mask_filter` cuts a camera to.
pub fn circle_mask(path: &Path, (width, height): (u32, u32)) -> Result<PathBuf, String> {
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    let (cx, cy) = (width as f64 / 2.0, height as f64 / 2.0);
    let radius = cx.min(cy);
    for y in 0..height {
        for x in 0..width {
            let d = ((x as f64 + 0.5 - cx).powi(2) + (y as f64 + 0.5 - cy).powi(2)).sqrt();
            let a = ((radius - d + 0.5).clamp(0.0, 1.0) * 255.0) as u8;
            let at = ((y * width + x) * 4) as usize;
            rgba[at..at + 4].copy_from_slice(&[255, 255, 255, a]);
        }
    }
    let file =
        std::fs::File::create(path).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
    Ok(path.to_path_buf())
}

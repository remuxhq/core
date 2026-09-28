//! Pictures the motor draws itself, as PNG files libobs reads: the circle
//! mask of a camera.

use std::path::{Path, PathBuf};

/// Where the PNGs go: beside the socket.
pub fn folder() -> PathBuf {
    let dir = remuxd_domain::socket::default_path().with_file_name("obs");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// A mask the size of a source: white inside the circle that fits the
/// square `region` of it, clear everywhere else. `mask_filter` stretches its
/// image over the whole source before the item's crop, so the circle is drawn
/// where the crop will leave it.
pub fn circle_mask(
    path: &Path,
    (width, height): (u32, u32),
    region: crate::place::Region,
) -> Result<PathBuf, String> {
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    let cx = f64::from(region.x) + f64::from(region.width) / 2.0;
    let cy = f64::from(region.y) + f64::from(region.height) / 2.0;
    let radius = f64::from(region.width.min(region.height)) / 2.0;
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

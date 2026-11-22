use image::DynamicImage;
use palette::Srgb;
use serde::{Deserialize, Serialize};

pub const DEFAULT_ACCENT_HEX: &str = "#FFCC00"; // Yandex Yellow
pub const BASE_DARK_HEX: &str = "#121212";
pub const SIDEBAR_DARK_HEX: &str = "#181818";
pub const CARD_DARK_HEX: &str = "#222222";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractedPalette {
    pub dominant_hex: String,
    pub accent_hex: String,
    pub secondary_hex: String,
    pub background_tint_hex: String,
}

impl Default for ExtractedPalette {
    fn default() -> Self {
        Self {
            dominant_hex: BASE_DARK_HEX.to_string(),
            accent_hex: DEFAULT_ACCENT_HEX.to_string(),
            secondary_hex: "#FFD633".to_string(),
            background_tint_hex: SIDEBAR_DARK_HEX.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemePalette {
    pub background: String,
    pub sidebar_background: String,
    pub card_background: String,
    pub accent: String,
    pub text_primary: String,
    pub text_secondary: String,
}

impl Default for ThemePalette {
    fn default() -> Self {
        Self {
            background: BASE_DARK_HEX.to_string(),
            sidebar_background: SIDEBAR_DARK_HEX.to_string(),
            card_background: CARD_DARK_HEX.to_string(),
            accent: DEFAULT_ACCENT_HEX.to_string(),
            text_primary: "#FFFFFF".to_string(),
            text_secondary: "#8C8C8C".to_string(),
        }
    }
}

/// Calculate relative luminance according to WCAG 2.1 definition
pub fn relative_luminance(r: f32, g: f32, b: f32) -> f32 {
    let adjust = |c: f32| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };

    let r_lin = adjust(r);
    let g_lin = adjust(g);
    let b_lin = adjust(b);

    0.2126 * r_lin + 0.7152 * g_lin + 0.0722 * b_lin
}

/// Calculate WCAG 2.1 contrast ratio between two colors (ranges from 1.0 to 21.0)
pub fn contrast_ratio(lum1: f32, lum2: f32) -> f32 {
    let lighter = lum1.max(lum2);
    let darker = lum1.min(lum2);
    (lighter + 0.05) / (darker + 0.05)
}

/// Ensure hex color meets WCAG 2.1 AA contrast ratio (>= 4.5:1) against dark background
pub fn ensure_contrast_wcag(hex: &str, bg_lum: f32) -> String {
    let clean = hex.trim_start_matches('#');
    if let Ok(color) = u32::from_str_radix(clean, 16) {
        let r = ((color >> 16) & 0xFF) as f32 / 255.0;
        let g = ((color >> 8) & 0xFF) as f32 / 255.0;
        let b = (color & 0xFF) as f32 / 255.0;

        let lum = relative_luminance(r, g, b);
        let ratio = contrast_ratio(lum, bg_lum);

        if ratio >= 4.5 {
            return hex.to_string();
        }

        // Brighten color until it satisfies >= 4.5 contrast
        let mut srgb = Srgb::new(r, g, b);
        for _ in 0..15 {
            srgb.red = (srgb.red + 0.08).min(1.0);
            srgb.green = (srgb.green + 0.08).min(1.0);
            srgb.blue = (srgb.blue + 0.08).min(1.0);
            let new_lum = relative_luminance(srgb.red, srgb.green, srgb.blue);
            if contrast_ratio(new_lum, bg_lum) >= 4.5 {
                break;
            }
        }

        let r_u8 = (srgb.red * 255.0).round() as u8;
        let g_u8 = (srgb.green * 255.0).round() as u8;
        let b_u8 = (srgb.blue * 255.0).round() as u8;
        return format!("#{:02X}{:02X}{:02X}", r_u8, g_u8, b_u8);
    }

    DEFAULT_ACCENT_HEX.to_string()
}

#[derive(Clone, Copy)]
struct ColorCluster {
    r: f32,
    g: f32,
    b: f32,
    count: usize,
}

/// Extract dominant and accent colors from image bytes (JPEG / PNG) using k-means clustering
pub fn extract_palette_from_image_bytes(bytes: &[u8]) -> ExtractedPalette {
    let dynamic_img = match image::load_from_memory(bytes) {
        Ok(img) => img,
        Err(_) => return ExtractedPalette::default(),
    };

    extract_palette_from_dynamic_image(&dynamic_img)
}

/// Extract dominant and accent colors from a loaded DynamicImage
pub fn extract_palette_from_dynamic_image(img: &DynamicImage) -> ExtractedPalette {
    // Downscale for fast, robust clustering (64x64 = 4096 pixels)
    let thumb = img.thumbnail(64, 64).to_rgb8();
    let pixels: Vec<[f32; 3]> = thumb
        .pixels()
        .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32])
        .collect();

    if pixels.is_empty() {
        return ExtractedPalette::default();
    }

    const K: usize = 5;
    let mut centroids = [[0.0f32; 3]; K];
    // Initialize centroids evenly spaced across the pixel set
    for (i, c) in centroids.iter_mut().enumerate() {
        let idx = (i * pixels.len()) / K;
        *c = pixels[idx];
    }

    let mut clusters = [ColorCluster { r: 0.0, g: 0.0, b: 0.0, count: 0 }; K];

    // Run 10 iterations of k-means
    for _ in 0..10 {
        let mut sums = [[0.0f32; 3]; K];
        let mut counts = [0usize; K];

        for &px in &pixels {
            let mut min_dist = f32::MAX;
            let mut best_k = 0;
            for (k, c) in centroids.iter().enumerate() {
                let dr = px[0] - c[0];
                let dg = px[1] - c[1];
                let db = px[2] - c[2];
                let dist_sq = dr * dr + dg * dg + db * db;
                if dist_sq < min_dist {
                    min_dist = dist_sq;
                    best_k = k;
                }
            }
            sums[best_k][0] += px[0];
            sums[best_k][1] += px[1];
            sums[best_k][2] += px[2];
            counts[best_k] += 1;
        }

        for k in 0..K {
            if counts[k] > 0 {
                centroids[k][0] = sums[k][0] / counts[k] as f32;
                centroids[k][1] = sums[k][1] / counts[k] as f32;
                centroids[k][2] = sums[k][2] / counts[k] as f32;
                clusters[k] = ColorCluster {
                    r: centroids[k][0],
                    g: centroids[k][1],
                    b: centroids[k][2],
                    count: counts[k],
                };
            }
        }
    }

    // Sort clusters by population (descending)
    let mut sorted_clusters: Vec<ColorCluster> = clusters.into_iter().filter(|c| c.count > 0).collect();
    sorted_clusters.sort_by_key(|a| std::cmp::Reverse(a.count));

    if sorted_clusters.is_empty() {
        return ExtractedPalette::default();
    }

    let dominant = sorted_clusters[0];
    let dominant_hex = rgb_to_hex(dominant.r, dominant.g, dominant.b);

    // Find the most vibrant cluster for the accent color
    let mut best_accent = sorted_clusters[0];
    let mut best_vibrancy = -1.0f32;

    for &cl in &sorted_clusters {
        let max_c = cl.r.max(cl.g).max(cl.b);
        let min_c = cl.r.min(cl.g).min(cl.b);
        let saturation = if max_c > 0.0 { (max_c - min_c) / max_c } else { 0.0 };
        let lum = relative_luminance(cl.r / 255.0, cl.g / 255.0, cl.b / 255.0);

        // Vibrancy prioritizes saturation and avoids pure black/white
        let vibrancy = saturation * (1.0 - (lum - 0.5).abs());
        if vibrancy > best_vibrancy && cl.count > (pixels.len() / 40) {
            best_vibrancy = vibrancy;
            best_accent = cl;
        }
    }

    let raw_accent_hex = rgb_to_hex(best_accent.r, best_accent.g, best_accent.b);
    let bg_lum = relative_luminance(0.07, 0.07, 0.07); // #121212 background luminance
    let accent_hex = ensure_contrast_wcag(&raw_accent_hex, bg_lum);

    // Secondary accent (2nd most populated or 2nd vibrant)
    let secondary = if sorted_clusters.len() > 1 {
        sorted_clusters[1]
    } else {
        best_accent
    };
    let secondary_hex = rgb_to_hex(secondary.r, secondary.g, secondary.b);

    // Background tint: 15% mix of dominant color with base dark #121212
    let tint_r = (dominant.r * 0.15 + 18.0 * 0.85).round();
    let tint_g = (dominant.g * 0.15 + 18.0 * 0.85).round();
    let tint_b = (dominant.b * 0.15 + 18.0 * 0.85).round();
    let background_tint_hex = rgb_to_hex(tint_r, tint_g, tint_b);

    ExtractedPalette {
        dominant_hex,
        accent_hex,
        secondary_hex,
        background_tint_hex,
    }
}

fn rgb_to_hex(r: f32, g: f32, b: f32) -> String {
    let r_u8 = r.clamp(0.0, 255.0).round() as u8;
    let g_u8 = g.clamp(0.0, 255.0).round() as u8;
    let b_u8 = b.clamp(0.0, 255.0).round() as u8;
    format!("#{:02X}{:02X}{:02X}", r_u8, g_u8, b_u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    #[test]
    fn test_yandex_yellow_contrast() {
        let bg_lum = relative_luminance(0.07, 0.07, 0.07);
        let yellow_lum = relative_luminance(1.0, 0.8, 0.0);
        let ratio = contrast_ratio(yellow_lum, bg_lum);
        assert!(ratio >= 4.5);
    }

    #[test]
    fn test_ensure_contrast_brightening() {
        let bg_lum = relative_luminance(0.07, 0.07, 0.07);
        // Dark blue with poor contrast
        let dark_blue = "#102040";
        let adjusted = ensure_contrast_wcag(dark_blue, bg_lum);
        assert_ne!(adjusted, dark_blue);

        let clean = adjusted.trim_start_matches('#');
        let color = u32::from_str_radix(clean, 16).unwrap();
        let r = ((color >> 16) & 0xFF) as f32 / 255.0;
        let g = ((color >> 8) & 0xFF) as f32 / 255.0;
        let b = (color & 0xFF) as f32 / 255.0;
        let ratio = contrast_ratio(relative_luminance(r, g, b), bg_lum);
        assert!(ratio >= 4.5);
    }

    #[test]
    fn test_extract_palette_from_synthetic_image() {
        // Create an image with vibrant cyan dominant color
        let mut img = RgbImage::new(100, 100);
        for pixel in img.pixels_mut() {
            *pixel = Rgb([0, 220, 255]); // Vibrant cyan
        }
        let dynamic = DynamicImage::ImageRgb8(img);
        let palette = extract_palette_from_dynamic_image(&dynamic);

        assert!(palette.dominant_hex.starts_with('#'));
        assert!(palette.accent_hex.starts_with('#'));

        // Check WCAG contrast of extracted accent against dark background
        let bg_lum = relative_luminance(0.07, 0.07, 0.07);
        let clean = palette.accent_hex.trim_start_matches('#');
        let color = u32::from_str_radix(clean, 16).unwrap();
        let r = ((color >> 16) & 0xFF) as f32 / 255.0;
        let g = ((color >> 8) & 0xFF) as f32 / 255.0;
        let b = (color & 0xFF) as f32 / 255.0;
        assert!(contrast_ratio(relative_luminance(r, g, b), bg_lum) >= 4.5);
    }

    #[test]
    fn test_extract_palette_corrupt_bytes_fallback() {
        let corrupt_data = [0xde, 0xad, 0xbe, 0xef];
        let palette = extract_palette_from_image_bytes(&corrupt_data);
        assert_eq!(palette.accent_hex, DEFAULT_ACCENT_HEX);
    }
}

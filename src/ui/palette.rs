use palette::Srgb;
use serde::{Deserialize, Serialize};

pub const DEFAULT_ACCENT_HEX: &str = "#FFCC00"; // Yandex Yellow
pub const BASE_DARK_HEX: &str = "#121212";
pub const SIDEBAR_DARK_HEX: &str = "#181818";
pub const CARD_DARK_HEX: &str = "#222222";

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
        for _ in 0..10 {
            srgb.red = (srgb.red + 0.1).min(1.0);
            srgb.green = (srgb.green + 0.1).min(1.0);
            srgb.blue = (srgb.blue + 0.1).min(1.0);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_yandex_yellow_contrast() {
        // Dark background #121212
        let bg_lum = relative_luminance(0.07, 0.07, 0.07);
        // Yandex yellow #FFCC00
        let yellow_lum = relative_luminance(1.0, 0.8, 0.0);

        let ratio = contrast_ratio(yellow_lum, bg_lum);
        println!("Yellow contrast ratio on dark: {:.2}", ratio);
        assert!(ratio >= 4.5);
    }

    #[test]
    fn test_ensure_contrast_brightening() {
        let bg_lum = relative_luminance(0.07, 0.07, 0.07);
        // Very dark blue that initially fails contrast
        let dark_color = "#111133";
        let adjusted = ensure_contrast_wcag(dark_color, bg_lum);
        assert_ne!(adjusted, dark_color);
    }
}

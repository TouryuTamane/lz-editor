use eframe::egui::{self, FontData, FontDefinitions, FontFamily};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

pub(crate) struct FontCandidate {
    pub(crate) path: String,
    pub(crate) index: u32,
}

/// Query fontconfig via `fc-match` to get the exact file path and TTC face index.
fn query_fc_match(pattern: &str) -> Option<FontCandidate> {
    let output = Command::new("fc-match")
        .args(["-f", "%{file}\n%{index}\n", pattern])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines();
    let path = lines.next()?.trim().to_string();
    let index = lines.next().and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(0);
    if path.is_empty() || !Path::new(&path).exists() {
        return None;
    }
    Some(FontCandidate { path, index })
}

pub(crate) fn japanese_font_candidates() -> Vec<FontCandidate> {
    // Priority 1: Query fontconfig for Japanese monospace fonts (works on Ubuntu, Debian, Raspberry Pi OS).
    // Prefer TrueType Japanese monospace fonts and Noto Sans Mono CJK JP.
    let fc_patterns = [
        "Noto Sans Mono CJK JP",
        "IPAGothic",
        "VL Gothic",
        "TakaoGothic",
        "Noto Sans CJK JP",
        "monospace:lang=ja",
        "sans-serif:lang=ja",
    ];

    let mut candidates: Vec<FontCandidate> = Vec::new();
    for pattern in fc_patterns {
        if let Some(cand) = query_fc_match(pattern) {
            // Skip non-Japanese fallback fonts returned when pattern doesn't match a CJK font.
            let lower = cand.path.to_lowercase();
            let looks_ja = lower.contains("cjk")
                || lower.contains("ipa")
                || lower.contains("vl-")
                || lower.contains("takao")
                || lower.contains("japanese")
                || lower.contains("noto")
                || lower.contains("droid")
                || lower.contains("wqy")
                || lower.contains("unifont")
                || lower.contains("hiragino")
                || lower.contains("mplus")
                || lower.contains("biz")
                || lower.contains("hackgen")
                || lower.contains("udev")
                || lower.contains("plemol");
            if looks_ja {
                candidates.push(cand);
            }
        }
    }

    // Priority 2: Direct filesystem fallbacks for Ubuntu & Raspberry Pi OS.
    let static_fallbacks: &[(&str, u32)] = &[
        ("/usr/share/fonts/opentype/ipafont-gothic/ipag.ttf", 0),
        ("/usr/share/fonts/truetype/vlgothic/VL-Gothic-Regular.ttf", 0),
        ("/usr/share/fonts/truetype/takao-gothic/TakaoGothic.ttf", 0),
        ("/usr/share/fonts/truetype/fonts-japanese-gothic.ttf", 0),
        ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 5),
        ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
        ("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", 5),
        ("/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf", 0),
    ];

    for &(path, index) in static_fallbacks {
        if Path::new(path).exists() {
            candidates.push(FontCandidate {
                path: path.to_string(),
                index,
            });
        }
    }

    candidates
}

/// Load a system Japanese monospace font into egui's font definitions to prevent tofu (missing glyphs).
pub fn configure_japanese_fonts(ctx: &egui::Context) -> Option<String> {
    let mut fonts = FontDefinitions::default();
    let mut loaded_names: Vec<String> = Vec::new();
    let mut seen_keys: Vec<(String, u32)> = Vec::new();
    let mut primary_loaded_path: Option<String> = None;

    for (i, cand) in japanese_font_candidates().into_iter().enumerate() {
        let key = (cand.path.clone(), cand.index);
        if seen_keys.contains(&key) {
            continue;
        }
        if let Ok(bytes) = std::fs::read(&cand.path) {
            seen_keys.push(key);
            let font_name = format!("ja_font_{i}");
            let mut font_data = FontData::from_owned(bytes);
            font_data.index = cand.index;
            fonts.font_data.insert(font_name.clone(), Arc::new(font_data));
            loaded_names.push(font_name);
            if primary_loaded_path.is_none() {
                primary_loaded_path = Some(format!("{} (index {})", cand.path, cand.index));
            }
            // Loading the single best Japanese monospace font keeps startup instant and RAM minimal.
            if !loaded_names.is_empty() {
                break;
            }
        }
    }

    // Insert loaded Japanese fonts at the front of Monospace and Proportional font families.
    for (pos, name) in loaded_names.iter().enumerate() {
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(pos, name.clone());
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(pos, name.clone());
    }

    ctx.set_fonts(fonts);
    primary_loaded_path
}

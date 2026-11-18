// Copyright (C) 2026 CRV Science Network (https://crvscience.com)
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://gnu.org>.

//! Portable asset resolution, `<exe_dir>` writable paths, and cross-platform UI font registration.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Returns the directory containing the running `wf_client` / `2_WildFront.exe` binary.
pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Resolves an existing `assets/` directory from CWD, `<exe_dir>/assets`, or workspace root.
#[allow(dead_code)]
pub fn resolve_asset_dir() -> Option<PathBuf> {
    let mut candidates = vec![PathBuf::from("assets"), exe_dir().join("assets")];
    if let Some(manifest) = option_env!("CARGO_MANIFEST_DIR") {
        candidates.push(Path::new(manifest).join("../../assets"));
    }
    candidates.into_iter().find(|p| p.is_dir())
}

/// Resolves a relative file or subdirectory beneath `assets/` (e.g. `"compiled/wildfront_roster.db"` or `"vox"`).
pub fn resolve_asset_path(rel_under_assets: &str) -> Option<PathBuf> {
    let mut candidates = vec![
        Path::new("assets").join(rel_under_assets),
        exe_dir().join("assets").join(rel_under_assets),
    ];
    if let Some(manifest) = option_env!("CARGO_MANIFEST_DIR") {
        candidates.push(Path::new(manifest).join("../../assets").join(rel_under_assets));
    }
    candidates.into_iter().find(|p| p.exists())
}

/// Returns a writable path anchored to CWD when running inside the source workspace,
/// or beside `<exe_dir>` when running standalone (`2_WildFront.exe`).
pub fn writable_app_path(rel_path: &str) -> PathBuf {
    if Path::new("Cargo.toml").exists() && Path::new("assets").is_dir() {
        PathBuf::from(rel_path)
    } else {
        exe_dir().join(rel_path)
    }
}

/// Ensures the parent directory of `path` exists before writing.
pub fn ensure_parent_dir(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

fn read_first_existing_font(paths: &[&str]) -> Option<Vec<u8>> {
    paths.iter().find_map(|p| fs::read(p).ok())
}

/// Installs OS system UI, monospace, symbol, and emoji fallback fonts into `egui::Context`
/// while preserving `egui` default fallback glyphs when available.
pub fn install_ui_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    let sans_candidates = [
        r"C:\Windows\Fonts\segoeui.ttf",
        r"C:\Windows\Fonts\arial.ttf",
        r"C:\Windows\Fonts\calibri.ttf",
        "/System/Library/Fonts/SFNS.ttf",
        "/System/Library/Fonts/Helvetica.ttc",
        "/Library/Fonts/Arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    ];
    let mono_candidates = [
        r"C:\Windows\Fonts\consola.ttf",
        r"C:\Windows\Fonts\lucon.ttf",
        "/System/Library/Fonts/Menlo.ttc",
        "/System/Library/Fonts/Monaco.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
    ];
    let sym_candidates = [
        r"C:\Windows\Fonts\seguisym.ttf",
        "/System/Library/Fonts/Apple Symbols.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf",
        "/usr/share/fonts/TTF/NotoSansSymbols2-Regular.ttf",
    ];
    let emoji_candidates = [
        r"C:\Windows\Fonts\seguiemj.ttf",
        "/System/Library/Fonts/Apple Color Emoji.ttc",
        "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
    ];

    let mut registered_any = false;

    if let Some(bytes) = read_first_existing_font(&sans_candidates) {
        fonts
            .font_data
            .insert("ui-sans".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
        let prop = fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default();
        if !prop.iter().any(|n| n == "ui-sans") {
            prop.insert(0, "ui-sans".to_owned());
        }
        registered_any = true;
    }

    if let Some(bytes) = read_first_existing_font(&mono_candidates) {
        fonts
            .font_data
            .insert("ui-mono".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
        let mono = fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default();
        if !mono.iter().any(|n| n == "ui-mono") {
            mono.insert(0, "ui-mono".to_owned());
        }
        registered_any = true;
    }

    if fonts.font_data.contains_key("ui-sans") {
        let mono = fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default();
        if !mono.iter().any(|n| n == "ui-sans") {
            let pos = if mono.first().map(|s| s.as_str()) == Some("ui-mono") { 1 } else { 0 };
            mono.insert(pos.min(mono.len()), "ui-sans".to_owned());
        }
    }

    if let Some(bytes) = read_first_existing_font(&sym_candidates) {
        fonts
            .font_data
            .insert("ui-sym".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
        let prop = fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default();
        if !prop.iter().any(|n| n == "ui-sym") {
            // Place ui-sym right after ui-sans (and before built-in incomplete emoji fonts)
            let idx = if prop.first().map(|s| s.as_str()) == Some("ui-sans") { 1 } else { 0 };
            prop.insert(idx.min(prop.len()), "ui-sym".to_owned());
        }
        let mono = fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default();
        if !mono.iter().any(|n| n == "ui-sym") {
            mono.push("ui-sym".to_owned());
        }
        registered_any = true;
    }

    if let Some(bytes) = read_first_existing_font(&emoji_candidates) {
        fonts
            .font_data
            .insert("ui-emoji".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
        let prop = fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default();
        if !prop.iter().any(|n| n == "ui-emoji") {
            prop.push("ui-emoji".to_owned());
        }
        let mono = fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default();
        if !mono.iter().any(|n| n == "ui-emoji") {
            mono.push("ui-emoji".to_owned());
        }
        registered_any = true;
    }

    if registered_any {
        ctx.set_fonts(fonts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_ui_fonts_registers_os_fonts_even_with_default_fonts_enabled() {
        let ctx = egui::Context::default();
        install_ui_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();

        // On Windows/Linux/macOS with system fonts, ui-sans and ui-sym should be active in Proportional
        ctx.fonts(|f| {
            let fam = f.definitions().families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
            if Path::new(r"C:\Windows\Fonts\segoeui.ttf").exists() {
                assert_eq!(fam.first().map(|s| s.as_str()), Some("ui-sans"));
            }
            if Path::new(r"C:\Windows\Fonts\seguisym.ttf").exists() {
                assert!(fam.iter().any(|s| s == "ui-sym"), "ui-sym must be registered in Proportional family");
                assert!(f.definitions().font_data.contains_key("ui-sym"));
            }
            if Path::new(r"C:\Windows\Fonts\seguiemj.ttf").exists() {
                assert!(f.definitions().font_data.contains_key("ui-emoji"));
            }
        });
    }
}

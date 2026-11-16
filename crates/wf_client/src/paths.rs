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

/// Installs OS system UI, monospace, and symbol fallback fonts into `egui::Context`
/// when `egui` compiles with `FontDefinitions::default().font_data` empty (`egui 0.37`).
pub fn install_ui_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    if !fonts.font_data.is_empty() {
        return;
    }

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
        r"C:\Windows\Fonts\seguiemj.ttf",
        "/System/Library/Fonts/Apple Symbols.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf",
    ];

    if let Some(bytes) = read_first_existing_font(&sans_candidates) {
        fonts
            .font_data
            .insert("ui-sans".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .push("ui-sans".to_owned());
    }
    if let Some(bytes) = read_first_existing_font(&mono_candidates) {
        fonts
            .font_data
            .insert("ui-mono".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .push("ui-mono".to_owned());
    }
    if fonts.font_data.contains_key("ui-sans") {
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .push("ui-sans".to_owned());
    }
    if let Some(bytes) = read_first_existing_font(&sym_candidates) {
        fonts
            .font_data
            .insert("ui-sym".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .push("ui-sym".to_owned());
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .push("ui-sym".to_owned());
    }

    if !fonts.font_data.is_empty() {
        ctx.set_fonts(fonts);
    }
}

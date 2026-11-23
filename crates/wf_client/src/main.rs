#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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

//! Native `winit 0.30.13` + `wgpu 30.0` + `egui 0.37.0` *Wildfront Battlegrounds* client.
//!
//! Features:
//! - Interactive 3D Hex Cursor Ray-Picking (terrain plateaus + unit voxel AABBs)
//! - Immediate-Mode `egui` 5-Card Hand Deployment Tray, Unit Command Card & Graveyard Queues
//! - GPU-Raymarched Combat VFX (laser beams, tracers, shockwaves, bio-rings & 3D voxel shrapnel)
//! - Right-drag or Left-drag orbit, Left-click hex pick/deploy/move/attack, Scroll zoom

mod audio;
mod codex_ui;
mod deck_builder_ui;
mod dice_roller_ui;
mod db_manager_ui;
mod map_editor_ui;
mod replay_ui;
mod main_menu_ui;
mod stl_export_ui;
mod net_lockstep;
mod game_session;
mod picking;
mod paths;
mod scene;
mod silhouettes;
mod ui_hud;
mod vfx;
mod voxel_pipeline;
mod world_overlays;

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Instant;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use glam::{Mat4, Vec3};
use hexx::Hex;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use wf_core::voxel::{CompiledModelHandle, GpuLeafBrick, GpuNode64};
use wf_core::{
    DeckList, MultiAssetGpuBuffers, RosterManifest, ScenarioId, WILDFRONT_THEME_2072,
};

const EMBEDDED_MASTER_ROSTER_RON: &str =
    include_str!("../../../assets/compiled/master_roster.ron");
const EMBEDDED_STARTER_DECK_0_RON: &str =
    include_str!("../../../assets/compiled/starter_deck_plant_cnidarian.ron");
const EMBEDDED_STARTER_DECK_1_RON: &str =
    include_str!("../../../assets/compiled/starter_deck_mammal_reptile.ron");
const EMBEDDED_VOX_64TREE_ZLIB: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/precompiled_vox_64tree.bin"));
const EMBEDDED_WINDOW_ICON_RGBA_64: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/wildfront_icon_64.rgba"));
use crate::audio::SpatialAudioEngine;
use crate::codex_ui::CodexOverlay;
use crate::deck_builder_ui::DeckBuilderScreen;
use crate::main_menu_ui::MainMenuScreen;
use crate::stl_export_ui::StlExportStudio;
use crate::dice_roller_ui::DiceRollerStudio;
use crate::db_manager_ui::DatabaseManagerStudio;
use crate::map_editor_ui::MapEditorStudio;
use crate::replay_ui::ReplayTheaterStudio;
use crate::net_lockstep::{NetEvent, NetLockstepManager, NetPacket};
use crate::audio::SfxCue;
use crate::game_session::{ClientMatchState, DEFAULT_QUICKSAVE_PATH};
use crate::world_overlays::draw_world_space_overlays;
use crate::picking::{pick_hex_from_ray, screen_cursor_to_world_ray};
use crate::scene::{axial_to_world_xz, build_live_gpu_scene, HEX_WORLD_RADIUS};
use crate::ui_hud::{draw_end_turn_confirm_modal, draw_tactical_hud};
use crate::vfx::VfxManager;
use crate::voxel_pipeline::{CameraUniforms, VoxelRenderEngine};

struct OrbitalCamera {
    target: Vec3,
    distance: f32,
    yaw: f32,
    pitch: f32,
    fov_y_radians: f32,
    orbit_dragging: bool,
    left_down_pos: Option<(f64, f64)>,
    last_cursor: (f64, f64),
    show_overlays: bool,
    show_unit_badges: bool,
    show_terrain_props: bool,
    atmosphere_mode: u32,
}

impl Default for OrbitalCamera {
    fn default() -> Self {
        Self {
            target: Vec3::new(0.0, 1.6, 0.0),
            distance: 35.0,
            yaw: 0.62,
            pitch: 0.72,
            fov_y_radians: 45.0_f32.to_radians(),
            orbit_dragging: false,
            left_down_pos: None,
            last_cursor: (720.0, 450.0),
            show_overlays: true,
            show_unit_badges: true,
            show_terrain_props: true,
            atmosphere_mode: 0,
        }
    }
}

impl OrbitalCamera {
    fn clamp_target_to_bounds(&mut self, bounds: (f32, f32, f32, f32)) {
        self.target.x = self.target.x.clamp(bounds.0, bounds.1);
        self.target.z = self.target.z.clamp(bounds.2, bounds.3);
    }

    fn reset_view(&mut self) {
        let def = Self::default();
        self.target = def.target;
        self.distance = def.distance;
        self.yaw = def.yaw;
        self.pitch = def.pitch;
    }

    fn view_matrix(&self) -> Mat4 {
        let cp = self.pitch.cos();
        let eye = self.target
            + Vec3::new(
                self.distance * cp * self.yaw.sin(),
                self.distance * self.pitch.sin(),
                self.distance * cp * self.yaw.cos(),
            );
        Mat4::look_at_rh(eye, self.target, Vec3::Y)
    }

    fn view_proj_matrix(&self, width: u32, height: u32) -> Mat4 {
        let aspect = (width.max(1) as f32) / (height.max(1) as f32);
        let view = self.view_matrix();
        let proj = Mat4::perspective_rh(self.fov_y_radians, aspect, 0.2, 300.0);
        proj * view
    }

    fn eye_and_inv_vp(&self, width: u32, height: u32) -> (Vec3, Mat4) {
        let aspect = (width.max(1) as f32) / (height.max(1) as f32);
        let cp = self.pitch.cos();
        let eye = self.target
            + Vec3::new(
                self.distance * cp * self.yaw.sin(),
                self.distance * self.pitch.sin(),
                self.distance * cp * self.yaw.cos(),
            );
        let view = Mat4::look_at_rh(eye, self.target, Vec3::Y);
        let proj = Mat4::perspective_rh(self.fov_y_radians, aspect, 0.2, 300.0);
        (eye, (proj * view).inverse())
    }

    fn build_uniforms(
        &self,
        width: u32,
        height: u32,
        instance_count: u32,
        time_seconds: f32,
        turn_number: u32,
    ) -> CameraUniforms {
        let (eye, inv_vp) = self.eye_and_inv_vp(width, height);
        let pixel_spread =
            (self.fov_y_radians * 0.5).tan() * 2.0 / (height.max(1) as f32);

        CameraUniforms {
            inv_view_proj: inv_vp.to_cols_array_2d(),
            camera_pos_spread: [eye.x, eye.y, eye.z, pixel_spread],
            sun_dir_flags: [
                0.48,
                0.78,
                0.40,
                if self.show_overlays { 1.0 } else { 0.0 },
            ],
            resolution: [width, height],
            instance_count,
            hex_radius: HEX_WORLD_RADIUS,
            vfx_beam_count: 0,
            vfx_debris_count: 0,
            time_seconds,
            atmosphere_mode: if self.atmosphere_mode == 0 && turn_number >= 30 { 2 } else { self.atmosphere_mode },
        }
    }
}

struct WildfrontApp {
    vox_gpu: MultiAssetGpuBuffers,
    handles: BTreeMap<String, CompiledModelHandle>,
    session: ClientMatchState,
    main_menu: MainMenuScreen,
    codex: CodexOverlay,
    stl_studio: StlExportStudio,
    dice_roller: DiceRollerStudio,
    db_manager: DatabaseManagerStudio,
    map_editor: MapEditorStudio,
    replay_theater: ReplayTheaterStudio,
    net: NetLockstepManager,
    deck_builder: DeckBuilderScreen,
    starter_presets: [DeckList; 2],
    audio: SpatialAudioEngine,
    vfx: VfxManager,
    active_seed: u64,
    camera: OrbitalCamera,
    pick_boxes: Vec<(usize, Hex, Vec3, Vec3)>,
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    engine: Option<VoxelRenderEngine>,
    last_frame_instant: Instant,
    show_about: bool,
    end_turn_confirm_open: bool,
}

fn board_pan_bounds(map: &wf_core::BattleHexMap) -> (f32, f32, f32, f32) {
    if map.tiles.is_empty() {
        let r = f32::from(map.radius.clamp(4, 9)) * HEX_WORLD_RADIUS * 1.8;
        return (-r, r, -r, r);
    }
    let (mut min_x, mut max_x, mut min_z, mut max_z) =
        (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for &h in map.tiles.keys() {
        let (wx, wz) = axial_to_world_xz(h);
        min_x = min_x.min(wx);
        max_x = max_x.max(wx);
        min_z = min_z.min(wz);
        max_z = max_z.max(wz);
    }
    let pad = HEX_WORLD_RADIUS * 1.25;
    (min_x - pad, max_x + pad, min_z - pad, max_z + pad)
}

#[derive(Debug, Clone, Copy, Default)]
struct ModalUiSnapshot {
    main_menu_open: bool,
    deck_builder_open: bool,
    codex_open: bool,
    stl_studio_open: bool,
    dice_roller_open: bool,
    db_manager_open: bool,
    map_editor_open: bool,
    replay_theater_open: bool,
    about_open: bool,
    end_turn_confirm_open: bool,
    outcome_modal_open: bool,
    hotseat_handoff_open: bool,
}

impl ModalUiSnapshot {
    fn any_blocking_gameplay_hotkey(self) -> bool {
        self.main_menu_open
            || self.deck_builder_open
            || self.codex_open
            || self.stl_studio_open
            || self.dice_roller_open
            || self.db_manager_open
            || self.map_editor_open
            || self.replay_theater_open
            || self.about_open
            || self.end_turn_confirm_open
            || self.outcome_modal_open
            || self.hotseat_handoff_open
    }

    fn any_blocking_camera_hotkey(self) -> bool {
        self.main_menu_open
            || self.deck_builder_open
            || self.codex_open
            || self.stl_studio_open
            || self.dice_roller_open
            || self.db_manager_open
            || self.replay_theater_open
            || self.about_open
            || self.end_turn_confirm_open
            || self.outcome_modal_open
            || self.hotseat_handoff_open
    }

    fn any_blocking_submodal_toggle(self) -> bool {
        self.about_open
            || self.end_turn_confirm_open
            || self.outcome_modal_open
            || self.hotseat_handoff_open
    }

    fn can_space_trigger_dice_roll(self) -> bool {
        self.dice_roller_open
            && !self.main_menu_open
            && !self.deck_builder_open
            && !self.codex_open
            && !self.stl_studio_open
            && !self.db_manager_open
            && !self.map_editor_open
            && !self.replay_theater_open
            && !self.about_open
            && !self.end_turn_confirm_open
            && !self.outcome_modal_open
            && !self.hotseat_handoff_open
    }

    fn can_space_reveal_handoff(self) -> bool {
        self.hotseat_handoff_open
            && !self.main_menu_open
            && !self.deck_builder_open
            && !self.codex_open
            && !self.stl_studio_open
            && !self.dice_roller_open
            && !self.db_manager_open
            && !self.map_editor_open
            && !self.replay_theater_open
            && !self.about_open
            && !self.end_turn_confirm_open
            && !self.outcome_modal_open
    }

    fn can_space_request_end_turn(self) -> bool {
        !self.any_blocking_gameplay_hotkey()
    }
}

fn load_window_icon() -> Option<winit::window::Icon> {
    winit::window::Icon::from_rgba(EMBEDDED_WINDOW_ICON_RGBA_64.to_vec(), 64, 64).ok()
}

fn draw_about_modal(root_ui: &mut egui::Ui, show_about: &mut bool) {
    if !*show_about {
        return;
    }
    let mut open = *show_about;
    egui::Window::new("ℹ About WildFront BattleGrounds")
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .collapsible(false)
        .resizable(false)
        .order(egui::Order::Foreground)
        .frame(
            egui::Frame::NONE
                .fill(egui::Color32::from_rgba_unmultiplied(12, 20, 28, 250))
                .stroke(egui::Stroke::new(2.0, egui::Color32::from_rgb(76, 212, 236)))
                .corner_radius(10)
                .inner_margin(egui::Margin::same(18)),
        )
        .show(root_ui.ctx(), |ui| {
            ui.set_min_width(420.0);
            ui.set_max_width(460.0);
            ui.label(
                egui::RichText::new("WildFront BattleGrounds")
                    .size(18.0)
                    .strong()
                    .color(egui::Color32::WHITE),
            );
            ui.label(
                egui::RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                    .strong()
                    .color(egui::Color32::from_rgb(112, 232, 248)),
            );
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(
                    "GPU-raymarched Sparse 64-Tree voxel hex tactical strategy client \
                     with 2-ply AI, TCP 1v1 lockstep multiplayer, 3D-print .STL studio, \
                     and live scenario map editor.",
                )
                .small()
                .color(egui::Color32::from_rgb(192, 218, 230)),
            );
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("Copyright CRV Science Network")
                    .strong()
                    .color(egui::Color32::from_rgb(232, 244, 248)),
            );
            ui.label(
                egui::RichText::new("License: GNU General Public License v3.0 or later (GPL-3.0-or-later)")
                    .small()
                    .color(egui::Color32::from_rgb(168, 206, 220)),
            );
            ui.add_space(6.0);
            ui.hyperlink_to("CRV Science Network — https://crvscience.com", "https://crvscience.com");
            ui.hyperlink_to("CRV Science Apps — https://crvscience.com/apps", "https://crvscience.com/apps");
            ui.add_space(10.0);
            if ui.button("Close").clicked() {
                *show_about = false;
            }
        });
    if !open {
        *show_about = false;
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_escape_priority(
    show_about: &mut bool,
    replay_open: &mut bool,
    map_open: &mut bool,
    db_open: &mut bool,
    dice_open: &mut bool,
    stl_open: &mut bool,
    codex_open: &mut bool,
    deck_open: &mut bool,
    mode: &mut crate::game_session::CommandMode,
    main_menu_has_started: bool,
    main_menu_open: &mut bool,
) {
    if *show_about {
        *show_about = false;
    } else if *replay_open {
        *replay_open = false;
    } else if *map_open {
        *map_open = false;
    } else if *db_open {
        *db_open = false;
    } else if *dice_open {
        *dice_open = false;
    } else if *stl_open {
        *stl_open = false;
    } else if *codex_open {
        *codex_open = false;
    } else if *deck_open {
        *deck_open = false;
    } else if *mode != crate::game_session::CommandMode::Inspect {
        *mode = crate::game_session::CommandMode::Inspect;
    } else if main_menu_has_started {
        *main_menu_open = !*main_menu_open;
    }
}

impl WildfrontApp {
    fn modal_snapshot(&self) -> ModalUiSnapshot {
        ModalUiSnapshot {
            main_menu_open: self.main_menu.is_open,
            deck_builder_open: self.deck_builder.is_open,
            codex_open: self.codex.is_open,
            stl_studio_open: self.stl_studio.is_open,
            dice_roller_open: self.dice_roller.is_open,
            db_manager_open: self.db_manager.is_open,
            map_editor_open: self.map_editor.is_open,
            replay_theater_open: self.replay_theater.is_open,
            about_open: self.show_about,
            end_turn_confirm_open: self.end_turn_confirm_open,
            outcome_modal_open: self.session.match_outcome.is_some()
                && !self.session.outcome_modal_dismissed,
            hotseat_handoff_open: self.session.hotseat_mode
                && self.session.hotseat_handoff_pending
                && self.session.match_outcome.is_none(),
        }
    }

    fn request_end_turn(&mut self) -> bool {
        if self.session.match_outcome.is_some() {
            return false;
        }
        if self.session.hotseat_handoff_pending {
            self.session.end_active_turn(&mut self.vfx, &mut self.audio);
            return true;
        }
        if !self.session.can_local_player_act() {
            return false;
        }
        let viewer = self.session.viewing_team();
        let act_rem = self
            .session
            .economy
            .actions_remaining
            .get(&viewer)
            .copied()
            .unwrap_or(0);
        if act_rem > 0 {
            self.end_turn_confirm_open = true;
            false
        } else {
            self.session.end_active_turn(&mut self.vfx, &mut self.audio);
            true
        }
    }

    fn update_hover_ray(&mut self) {
        let Some(eng) = self.engine.as_ref() else {
            return;
        };
        let (eye, inv_vp) = self
            .camera
            .eye_and_inv_vp(eng.config.width, eng.config.height);
        let (ro, rd) = screen_cursor_to_world_ray(
            self.camera.last_cursor,
            (eng.config.width, eng.config.height),
            eye,
            inv_vp,
        );
        self.session.hovered_hex = pick_hex_from_ray(ro, rd, &self.session.scenario.map, &self.pick_boxes)
            .map(|res| res.hex);
    }
}

impl ApplicationHandler for WildfrontApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.engine.is_some() {
            return;
        }
        let (hex_tiles, instances, pick_boxes) =
            build_live_gpu_scene(&self.session, &self.vfx, &self.handles, self.camera.show_terrain_props);
        self.pick_boxes = pick_boxes;

        let attrs = Window::default_attributes()
            .with_title(format!(
                "Wildfront Battlegrounds — {} [Seed 0x{:08X}]",
                self.session.scenario.title, self.active_seed
            ))
            .with_inner_size(PhysicalSize::new(1480, 920))
            .with_window_icon(load_window_icon());
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("Failed to create Wildfront window"),
        );

        let engine = pollster::block_on(VoxelRenderEngine::new(
            window.clone(),
            &self.vox_gpu,
            &hex_tiles,
            &instances,
        ));
        let max_tex = engine.device.limits().max_texture_dimension_2d as usize;
        let egui_state = egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &*window,
            Some(window.scale_factor() as f32),
            None,
            Some(max_tex),
        );

        self.last_frame_instant = Instant::now();
        if let Some(w) = engine.window.as_ref() { w.request_redraw(); }
        self.egui_state = Some(egui_state);
        self.engine = Some(engine);
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Keep continuous 60 FPS redraws running whenever combat VFX or flying units are active
        if self.main_menu.is_open
            || self.net.status != crate::net_lockstep::NetStatus::Offline
            || self.vfx.has_active_effects()
            || self.session.status_toast.is_some()
            || self.session.entities.iter().any(|e| e.card.can_fly || matches!(e.vox_slug.as_str(), "fish_laser_shark" | "cnidarian_manowar"))
        {
            if let Some(eng) = self.engine.as_ref() {
                if let Some(w) = eng.window.as_ref() { w.request_redraw(); }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // Forward input to egui first so HUD buttons/cards consume clicks cleanly
        if let (Some(egui_state), Some(eng)) = (self.egui_state.as_mut(), self.engine.as_ref()) {
            if let Some(window) = eng.window.as_ref() {
                let resp = egui_state.on_window_event(window, &event);
                if resp.repaint {
                    window.request_redraw();
                }
                if resp.consumed && !matches!(event, WindowEvent::RedrawRequested | WindowEvent::Resized(_)) {
                    return;
                }
            }
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(sz) => {
                if let Some(eng) = self.engine.as_mut() {
                    eng.resize(sz.width, sz.height);
                    if let Some(w) = eng.window.as_ref() { w.request_redraw(); }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => match button {
                MouseButton::Right => {
                    self.camera.orbit_dragging = state == ElementState::Pressed;
                }
                MouseButton::Left => {
                    if state == ElementState::Pressed {
                        self.camera.left_down_pos = Some(self.camera.last_cursor);
                        self.camera.orbit_dragging = true;
                    } else {
                        self.camera.orbit_dragging = false;
                        if let Some((sx, sy)) = self.camera.left_down_pos.take() {
                            let dist = ((self.camera.last_cursor.0 - sx).powi(2)
                                + (self.camera.last_cursor.1 - sy).powi(2))
                            .sqrt();
                            // Crisp click (< 6 px drag) triggers 3D hex pick / deploy / move / attack!
                            if dist < 6.0 {
                                self.update_hover_ray();
                                if let Some(clicked_hex) = self.session.hovered_hex {
                                    if self.map_editor.is_open && self.map_editor.brush_paint_enabled {
                                        if self.map_editor.paint_hex(&mut self.session, clicked_hex) {
                                            let (ht, inst, pb) = build_live_gpu_scene(&self.session, &self.vfx, &self.handles, self.camera.show_terrain_props);
                                            self.pick_boxes = pb;
                                            if let Some(eng) = self.engine.as_mut() {
                                                let (beams, beam_cnt, debris, debris_cnt) = self.vfx.pack_gpu_buffers();
                                                eng.upload_live_scene_and_vfx(&ht, &inst, &beams, beam_cnt, &debris, debris_cnt);
                                            }
                                        }
                                    } else if !self.modal_snapshot().any_blocking_camera_hotkey()
                                        && !self.map_editor.is_open
                                    {
                                        self.session.handle_hex_click(clicked_hex, &mut self.vfx, &mut self.audio);
                                    }
                                }
                            }
                        }
                    }
                    if let Some(eng) = self.engine.as_ref() {
                        if let Some(w) = eng.window.as_ref() { w.request_redraw(); }
                    }
                }
                _ => {}
            },
            WindowEvent::CursorMoved { position, .. } => {
                let prev = self.camera.last_cursor;
                self.camera.last_cursor = (position.x, position.y);

                if self.camera.orbit_dragging {
                    let dx = (position.x - prev.0) as f32;
                    let dy = (position.y - prev.1) as f32;
                    if dx.abs() + dy.abs() > 0.5 {
                        self.camera.yaw -= dx * 0.006;
                        self.camera.pitch = (self.camera.pitch + dy * 0.006).clamp(0.18, 1.45);
                    }
                }
                self.update_hover_ray();
                if let Some(eng) = self.engine.as_ref() {
                    if let Some(w) = eng.window.as_ref() { w.request_redraw(); }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let scroll = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => (p.y as f32) * 0.02,
                };
                self.camera.distance = (self.camera.distance * (1.0 - scroll * 0.10)).clamp(3.2, 68.0);
                self.update_hover_ray();
                if let Some(eng) = self.engine.as_ref() {
                    if let Some(w) = eng.window.as_ref() { w.request_redraw(); }
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let modals = self.modal_snapshot();
                let forward = Vec3::new(-self.camera.yaw.sin(), 0.0, -self.camera.yaw.cos());
                let right = Vec3::new(self.camera.yaw.cos(), 0.0, -self.camera.yaw.sin());
                let pan_step = 1.25;
                let mut camera_changed = false;
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::Escape) => {
                        if self.end_turn_confirm_open {
                            self.end_turn_confirm_open = false;
                        } else {
                            handle_escape_priority(
                                &mut self.show_about,
                                &mut self.replay_theater.is_open,
                                &mut self.map_editor.is_open,
                                &mut self.db_manager.is_open,
                                &mut self.dice_roller.is_open,
                                &mut self.stl_studio.is_open,
                                &mut self.codex.is_open,
                                &mut self.deck_builder.is_open,
                                &mut self.session.mode,
                                self.main_menu.has_started_match,
                                &mut self.main_menu.is_open,
                            );
                        }
                    }
                    PhysicalKey::Code(KeyCode::Enter) | PhysicalKey::Code(KeyCode::NumpadEnter) => {
                        if self.end_turn_confirm_open
                            && !self.main_menu.is_open
                            && !self.show_about
                            && !self.deck_builder.is_open
                            && !self.codex.is_open
                            && !self.db_manager.is_open
                            && !self.stl_studio.is_open
                            && !self.dice_roller.is_open
                            && !self.map_editor.is_open
                            && !self.replay_theater.is_open
                        {
                            self.end_turn_confirm_open = false;
                            self.session.end_active_turn(&mut self.vfx, &mut self.audio);
                        }
                    }
                    PhysicalKey::Code(KeyCode::F1) => {
                        let other_open = modals.any_blocking_submodal_toggle()
                            || self.deck_builder.is_open
                            || self.db_manager.is_open
                            || self.stl_studio.is_open
                            || self.dice_roller.is_open
                            || self.map_editor.is_open
                            || self.replay_theater.is_open;
                        if self.codex.is_open || !other_open {
                            self.codex.is_open = !self.codex.is_open;
                        }
                    }
                    PhysicalKey::Code(KeyCode::F3) => {
                        let other_open = modals.any_blocking_submodal_toggle()
                            || self.deck_builder.is_open
                            || self.codex.is_open
                            || self.db_manager.is_open
                            || self.stl_studio.is_open
                            || self.dice_roller.is_open
                            || self.replay_theater.is_open;
                        if self.map_editor.is_open || !other_open {
                            self.map_editor.is_open = !self.map_editor.is_open;
                        }
                    }
                    PhysicalKey::Code(KeyCode::F4) => {
                        let other_open = modals.any_blocking_submodal_toggle()
                            || self.deck_builder.is_open
                            || self.codex.is_open
                            || self.db_manager.is_open
                            || self.stl_studio.is_open
                            || self.dice_roller.is_open
                            || self.map_editor.is_open;
                        if self.replay_theater.is_open || !other_open {
                            self.replay_theater.is_open = !self.replay_theater.is_open;
                            if self.replay_theater.is_open {
                                self.replay_theater.sync_to_latest(&self.session);
                            }
                        }
                    }
                    PhysicalKey::Code(KeyCode::KeyL) => {
                        if !modals.any_blocking_gameplay_hotkey() {
                            self.camera.atmosphere_mode = (self.camera.atmosphere_mode + 1) % 4;
                            let sky_lbl = match self.camera.atmosphere_mode {
                                0 => "🌤 Sky: Clear Daylight (Auto Storm R30+)",
                                1 => "🌅 Sky: Dusk Tactical",
                                2 => "☄ Sky: Orbital Fallout Storm",
                                _ => "🌙 Sky: Night Bio-Luminescence",
                            };
                            self.session.push_toast(sky_lbl);
                        }
                    }
                    PhysicalKey::Code(KeyCode::F2) => {
                        let other_open = modals.any_blocking_submodal_toggle()
                            || self.deck_builder.is_open
                            || self.codex.is_open
                            || self.stl_studio.is_open
                            || self.dice_roller.is_open
                            || self.map_editor.is_open
                            || self.replay_theater.is_open;
                        if self.db_manager.is_open {
                            self.db_manager.is_open = false;
                        } else if !other_open {
                            if let Some(sel_h) = self.session.selected_hex {
                                if let Some(idx) = self.session.entity_at(sel_h) {
                                    let ck = self.session.entities[idx].card.card_key.clone();
                                    self.db_manager.open_for_unit(&ck);
                                } else {
                                    self.db_manager.is_open = true;
                                }
                            } else {
                                self.db_manager.is_open = true;
                            }
                        }
                    }
                    PhysicalKey::Code(KeyCode::KeyR) => {
                        let other_open = modals.any_blocking_submodal_toggle()
                            || self.deck_builder.is_open
                            || self.codex.is_open
                            || self.db_manager.is_open
                            || self.stl_studio.is_open
                            || self.map_editor.is_open
                            || self.replay_theater.is_open;
                        if self.dice_roller.is_open {
                            self.dice_roller.is_open = false;
                        } else if !other_open {
                            self.dice_roller.open_with_selection(&self.session);
                        }
                    }
                    PhysicalKey::Code(KeyCode::F8) => {
                        let other_open = modals.any_blocking_submodal_toggle()
                            || self.deck_builder.is_open
                            || self.codex.is_open
                            || self.db_manager.is_open
                            || self.dice_roller.is_open
                            || self.map_editor.is_open
                            || self.replay_theater.is_open;
                        if self.stl_studio.is_open {
                            self.stl_studio.is_open = false;
                        } else if !other_open {
                            if let Some(sel_h) = self.session.selected_hex {
                                if let Some(idx) = self.session.entity_at(sel_h) {
                                    let ck = self.session.entities[idx].card.card_key.clone();
                                    self.stl_studio.focus_card(&ck);
                                } else {
                                    self.stl_studio.is_open = true;
                                }
                            } else {
                                self.stl_studio.is_open = true;
                            }
                        }
                    }
                    PhysicalKey::Code(KeyCode::KeyB) => {
                        let other_open = modals.any_blocking_submodal_toggle()
                            || self.codex.is_open
                            || self.db_manager.is_open
                            || self.stl_studio.is_open
                            || self.dice_roller.is_open
                            || self.map_editor.is_open
                            || self.replay_theater.is_open;
                        if self.deck_builder.is_open || !other_open {
                            self.deck_builder.is_open = !self.deck_builder.is_open;
                        }
                    }
                    PhysicalKey::Code(KeyCode::Tab) => {
                        if !modals.any_blocking_gameplay_hotkey() {
                            self.camera.show_overlays = !self.camera.show_overlays;
                        }
                    }
                    PhysicalKey::Code(KeyCode::KeyH) => {
                        if !modals.any_blocking_gameplay_hotkey() {
                            self.camera.show_unit_badges = !self.camera.show_unit_badges;
                        }
                    }
                    PhysicalKey::Code(KeyCode::KeyP) => {
                        if !modals.any_blocking_gameplay_hotkey() {
                            self.camera.show_terrain_props = !self.camera.show_terrain_props;
                        }
                    }
                    PhysicalKey::Code(KeyCode::KeyF) => {
                        if !modals.any_blocking_gameplay_hotkey() {
                            self.session.fog_enabled = !self.session.fog_enabled;
                            if !self.session.fog_enabled {
                                self.session.hotseat_handoff_pending = false;
                            }
                        }
                    }
                    PhysicalKey::Code(KeyCode::KeyM) => {
                        if !modals.any_blocking_gameplay_hotkey() {
                            self.session.hotseat_mode = !self.session.hotseat_mode;
                            self.session.hotseat_handoff_pending = false;
                            self.session.mode = crate::game_session::CommandMode::Inspect;
                            let mode_lbl = if self.session.hotseat_mode {
                                "👥 Switched to 1v1 Local Hotseat Mode!"
                            } else {
                                "🤖 Switched to Solo vs AI Mode!"
                            };
                            self.session.push_toast(mode_lbl);
                        }
                    }
                    PhysicalKey::Code(KeyCode::F5) => {
                        if !modals.any_blocking_gameplay_hotkey() {
                            let save_p = crate::paths::writable_app_path(DEFAULT_QUICKSAVE_PATH);
                            match self.session.save_quicksave(&save_p, self.active_seed) {
                                Ok(_) => self.audio.play_ui(SfxCue::BioHealChime),
                                Err(err) => self.session.push_toast(format!("⚠ Save failed: {err}")),
                            }
                        }
                    }
                    PhysicalKey::Code(KeyCode::F9) => {
                        if !modals.any_blocking_gameplay_hotkey() {
                            let load_p = crate::paths::writable_app_path(DEFAULT_QUICKSAVE_PATH);
                            match self.session.load_quicksave(&load_p) {
                                Ok(restored_seed) => {
                                    self.active_seed = restored_seed;
                                    self.end_turn_confirm_open = false;
                                    self.audio.play_ui(SfxCue::UiCardTurn);
                                }
                                Err(err) => self.session.push_toast(format!("⚠ {err}")),
                            }
                        }
                    }
                    PhysicalKey::Code(KeyCode::KeyW) | PhysicalKey::Code(KeyCode::ArrowUp) => {
                        if !modals.any_blocking_camera_hotkey() {
                            self.camera.target += forward * pan_step;
                            camera_changed = true;
                        }
                    }
                    PhysicalKey::Code(KeyCode::KeyS) | PhysicalKey::Code(KeyCode::ArrowDown) => {
                        if !modals.any_blocking_camera_hotkey() {
                            self.camera.target -= forward * pan_step;
                            camera_changed = true;
                        }
                    }
                    PhysicalKey::Code(KeyCode::KeyA) | PhysicalKey::Code(KeyCode::ArrowLeft) => {
                        if !modals.any_blocking_camera_hotkey() {
                            self.camera.target -= right * pan_step;
                            camera_changed = true;
                        }
                    }
                    PhysicalKey::Code(KeyCode::KeyD) | PhysicalKey::Code(KeyCode::ArrowRight) => {
                        if !modals.any_blocking_camera_hotkey() {
                            self.camera.target += right * pan_step;
                            camera_changed = true;
                        }
                    }
                    PhysicalKey::Code(KeyCode::Home) | PhysicalKey::Code(KeyCode::KeyC) => {
                        if !modals.any_blocking_camera_hotkey() {
                            self.camera.reset_view();
                            camera_changed = true;
                            self.session.push_toast("🎯 Camera reset to tactical overview");
                        }
                    }
                    PhysicalKey::Code(KeyCode::Digit1) => {
                        if !modals.any_blocking_gameplay_hotkey() {
                            self.end_turn_confirm_open = false;
                            self.session.load_scenario(ScenarioId::EmergenceSkirmish, self.active_seed);
                        }
                    }
                    PhysicalKey::Code(KeyCode::Digit2) => {
                        if !modals.any_blocking_gameplay_hotkey() {
                            self.end_turn_confirm_open = false;
                            self.session.load_scenario(ScenarioId::BunkerBreakout, self.active_seed);
                        }
                    }
                    PhysicalKey::Code(KeyCode::Digit3) => {
                        if !modals.any_blocking_gameplay_hotkey() {
                            self.end_turn_confirm_open = false;
                            self.session.load_scenario(ScenarioId::BioRiverConvoy, self.active_seed);
                        }
                    }
                    PhysicalKey::Code(KeyCode::Digit4) => {
                        if !modals.any_blocking_gameplay_hotkey() {
                            self.end_turn_confirm_open = false;
                            self.session.load_scenario(ScenarioId::PelagicAtollAssault, self.active_seed);
                        }
                    }
                    PhysicalKey::Code(KeyCode::Digit5) => {
                        if !modals.any_blocking_gameplay_hotkey() {
                            self.end_turn_confirm_open = false;
                            self.session.load_scenario(ScenarioId::KingOfTheControlSpire, self.active_seed);
                        }
                    }
                    PhysicalKey::Code(KeyCode::KeyN) => {
                        if !modals.any_blocking_gameplay_hotkey() {
                            self.end_turn_confirm_open = false;
                            self.active_seed = self.active_seed.wrapping_add(0x9E37_79B9);
                            let sid = self.session.scenario.scenario_id;
                            self.session.load_scenario(sid, self.active_seed);
                        }
                    }
                    PhysicalKey::Code(KeyCode::Space) => {
                        if modals.can_space_trigger_dice_roll() {
                            self.dice_roller.trigger_roll(&mut self.audio);
                        } else if modals.can_space_reveal_handoff() || modals.can_space_request_end_turn() {
                            self.request_end_turn();
                        }
                    }
                    _ => {}
                }
                if camera_changed {
                    let bounds = board_pan_bounds(&self.session.scenario.map);
                    self.camera.clamp_target_to_bounds(bounds);
                    self.update_hover_ray();
                }
                if let Some(eng) = self.engine.as_ref() {
                    if let Some(w) = eng.window.as_ref() { w.request_redraw(); }
                }
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - self.last_frame_instant).as_secs_f32();
                self.last_frame_instant = now;
                self.vfx.tick(dt);
                self.session.tick_toast(dt);
                self.dice_roller.tick(dt, &mut self.audio);
                if self.main_menu.is_open {
                    self.camera.yaw += dt * 0.11;
                }
                self.audio.tick(dt, self.camera.view_matrix());

                // Flush any local lockstep commands to remote TCP peer
                for out_cmd in self.session.outbound_net_cmds.drain(..) {
                    if self.net.is_online() {
                        self.net.send(NetPacket::Command {
                            cmd: out_cmd.cmd,
                            post_state_hash: out_cmd.post_state_hash,
                        });
                    }
                }
                // Poll inbound TCP lockstep events
                for net_ev in self.net.poll_events() {
                    match net_ev {
                        NetEvent::PeerConnected(peer) => {
                            if matches!(self.net.status, crate::net_lockstep::NetStatus::ConnectedHost(_)) {
                                self.net.send(NetPacket::MatchInit {
                                    scenario_id: self.session.scenario.scenario_id,
                                    seed: self.active_seed,
                                    host_deck: self.session.decks[0].clone(),
                                    fog_enabled: self.session.fog_enabled,
                                });
                                self.session.push_toast(format!("🌐 Player 2 connected from {peer}!"));
                            }
                        }
                        NetEvent::PacketReceived(NetPacket::MatchInit {
                            scenario_id,
                            seed,
                            host_deck,
                            fog_enabled,
                        }) => {
                            self.active_seed = seed;
                            let my_p2_deck = self.session.decks[1].clone();
                            self.session.decks[0] = host_deck;
                            self.session.online_local_team = Some(2);
                            self.session.hotseat_mode = false;
                            self.session.fog_enabled = fog_enabled;
                            self.session.load_scenario(scenario_id, seed);
                            self.net.send(NetPacket::ClientJoinDeck { client_deck: my_p2_deck });
                            self.main_menu.has_started_match = true;
                            self.main_menu.is_open = false;
                            self.session.push_toast("🌐 Synced TCP Lockstep Match as Player 2!");
                        }
                        NetEvent::PacketReceived(NetPacket::ClientJoinDeck { client_deck }) => {
                            self.session.decks[1] = client_deck;
                            self.session.online_local_team = Some(1);
                            self.session.hotseat_mode = false;
                            let sid = self.session.scenario.scenario_id;
                            self.session.load_scenario(sid, self.active_seed);
                            self.main_menu.has_started_match = true;
                            self.main_menu.is_open = false;
                            self.session.push_toast("🌐 Synced TCP Lockstep Match as Player 1 (Host)!");
                        }
                        NetEvent::PacketReceived(NetPacket::Command {
                            cmd: remote_cmd,
                            post_state_hash,
                        }) => {
                            if let Err(err) = self.session.apply_remote_lockstep_command(
                                remote_cmd,
                                post_state_hash,
                                &mut self.vfx,
                                &mut self.audio,
                            ) {
                                self.session.push_toast(format!("⚠ {err}"));
                            }
                        }
                        NetEvent::PacketReceived(NetPacket::DisconnectNotice)
                        | NetEvent::ErrorOrClosed(_) => {
                            self.session.online_local_team = None;
                            self.session.push_toast(self.net.status_line.clone());
                        }
                    }
                }

                // Build live GPU scene & AABB pick-boxes BEFORE UI so 3D unit badges align to exact frame positions!
                let (hex_tiles, instances, pick_boxes) =
                    build_live_gpu_scene(&self.session, &self.vfx, &self.handles, self.camera.show_terrain_props);
                self.pick_boxes = pick_boxes;

                let prev_view_sig = (self.session.fog_enabled, self.session.hotseat_mode, self.session.hotseat_handoff_pending, self.session.viewing_team(), self.session.mode);
                let mut hud_actions = Default::default();
                let mut menu_actions = Default::default();
                let mut launched_custom_deck: Option<(usize, DeckList)> = None;
                let mut confirm_end_turn_now = false;
                let mut paint_jobs = Vec::new();
                let mut textures_delta = egui::TexturesDelta::default();

                if let (Some(egui_state), Some(eng)) = (self.egui_state.as_mut(), self.engine.as_ref()) {
                    let window = eng.window.as_ref().expect("windowed mode");
                    let raw_input = egui_state.take_egui_input(window);
                    let show_ov = self.camera.show_overlays;
                    let show_badges = self.camera.show_unit_badges && !self.deck_builder.is_open && !self.main_menu.is_open && !self.session.hotseat_handoff_pending;
                    let vp_mat = self.camera.view_proj_matrix(eng.config.width, eng.config.height);
                    let cam_dist = self.camera.distance;

                    let p1_dname = self.session.decks[0].deck_name.clone();
                    let p2_dname = self.session.decks[1].deck_name.clone();
                    let cur_sid = self.session.scenario.scenario_id;
                    let cur_seed = self.active_seed;

                    let full_output = self.egui_ctx.run_ui(raw_input, |root_ui| {
                        if !self.main_menu.is_open {
                            hud_actions = draw_tactical_hud(
                                root_ui,
                                &mut self.session,
                                &mut self.vfx,
                                &mut self.audio,
                                show_ov,
                                self.camera.show_unit_badges,
                                self.camera.show_terrain_props,
                            );
                        }
                        let viewport_rect = root_ui.available_rect_before_wrap();
                        draw_world_space_overlays(
                            root_ui,
                            viewport_rect,
                            vp_mat,
                            cam_dist,
                            &self.session,
                            &self.vfx,
                            &self.pick_boxes,
                            show_badges,
                        );
                        if !self.deck_builder.is_open {
                            menu_actions = self.main_menu.show(
                                root_ui,
                                cur_sid,
                                cur_seed,
                                &p1_dname,
                                &p2_dname,
                                &mut self.net,
                                self.session.ai_difficulty,
                            );
                        }
                        self.codex.show(root_ui);
                        let p1_deck_snap = self.session.decks[0].clone();
                        self.stl_studio.show(
                            root_ui,
                            &self.session.roster,
                            &p1_deck_snap,
                            &mut self.audio,
                        );
                        self.dice_roller.show(
                            root_ui,
                            &self.session,
                            &mut self.audio,
                        );
                        self.db_manager.show(
                            root_ui,
                            &mut self.session.roster,
                            &mut self.deck_builder.working_deck,
                            &mut self.audio,
                        );
                        if self.map_editor.show(root_ui, &mut self.session, &mut self.audio) {
                            hud_actions.toggle_overlays = false; // triggers redraw below
                        }
                        if self.replay_theater.show(
                            root_ui,
                            &mut self.session,
                            &mut self.active_seed,
                            &mut self.audio,
                        ) {
                            hud_actions.toggle_overlays = false;
                        }
                        if let Some(new_deck) = self.deck_builder.show(
                            root_ui,
                            &self.session.roster,
                            &self.starter_presets,
                            &mut self.audio,
                        ) {
                            launched_custom_deck = Some(new_deck);
                        }
                        draw_about_modal(root_ui, &mut self.show_about);
                        if self.end_turn_confirm_open
                            && (self.main_menu.is_open || !self.session.can_local_player_act())
                        {
                            self.end_turn_confirm_open = false;
                        }
                        let viewer = self.session.viewing_team();
                        let act_rem = self
                            .session
                            .economy
                            .actions_remaining
                            .get(&viewer)
                            .copied()
                            .unwrap_or(0);
                        if draw_end_turn_confirm_modal(root_ui, &mut self.end_turn_confirm_open, act_rem) {
                            confirm_end_turn_now = true;
                        }
                    });
                    let window = eng.window.as_ref().expect("windowed mode");
                    egui_state.handle_platform_output(window, full_output.platform_output);
                    paint_jobs = self
                        .egui_ctx
                        .tessellate(full_output.shapes, full_output.pixels_per_point);
                    textures_delta = full_output.textures_delta;
                }

                // Apply any HUD toolbar & modal actions
                let curr_view_sig = (self.session.fog_enabled, self.session.hotseat_mode, self.session.hotseat_handoff_pending, self.session.viewing_team(), self.session.mode);
                let mut scene_mutated = prev_view_sig != curr_view_sig || self.map_editor.is_open || self.replay_theater.is_open;
                if menu_actions.quit_app {
                    self.net.disconnect();
                    event_loop.exit();
                    return;
                }
                if hud_actions.toggle_main_menu {
                    self.main_menu.is_open = !self.main_menu.is_open;
                }
                if hud_actions.toggle_codex || menu_actions.open_codex {
                    self.codex.is_open = !self.codex.is_open;
                }
                if hud_actions.open_about || menu_actions.open_about {
                    self.show_about = !self.show_about;
                }
                if hud_actions.toggle_map_editor || menu_actions.open_map_editor {
                    self.map_editor.is_open = !self.map_editor.is_open;
                    self.main_menu.is_open = false;
                    scene_mutated = true;
                }
                if hud_actions.toggle_replay_theater || menu_actions.open_replay_theater {
                    self.replay_theater.is_open = !self.replay_theater.is_open;
                    if self.replay_theater.is_open {
                        self.replay_theater.sync_to_latest(&self.session);
                    }
                    self.main_menu.is_open = false;
                    scene_mutated = true;
                }
                if hud_actions.cycle_atmosphere {
                    self.camera.atmosphere_mode = (self.camera.atmosphere_mode + 1) % 4;
                }
                if let Some(new_diff) = menu_actions.select_ai_difficulty {
                    self.session.ai_difficulty = new_diff;
                }
                if let Some(req_card) = self.deck_builder.requested_db_card.take() {
                    self.db_manager.open_for_unit(&req_card);
                }
                if hud_actions.toggle_db_manager || menu_actions.open_db_manager {
                    if !self.db_manager.is_open {
                        if let Some(sel_h) = self.session.selected_hex {
                            if let Some(idx) = self.session.entity_at(sel_h) {
                                let ck = self.session.entities[idx].card.card_key.clone();
                                self.db_manager.open_for_unit(&ck);
                            } else {
                                self.db_manager.is_open = true;
                            }
                        } else {
                            self.db_manager.is_open = true;
                        }
                    } else {
                        self.db_manager.is_open = false;
                    }
                }
                if hud_actions.toggle_dice_roller || menu_actions.open_dice_roller {
                    if !self.dice_roller.is_open {
                        self.dice_roller.open_with_selection(&self.session);
                    } else {
                        self.dice_roller.is_open = false;
                    }
                }
                if hud_actions.toggle_stl_studio || menu_actions.open_stl_studio {
                    if let Some(sel_h) = self.session.selected_hex {
                        if let Some(idx) = self.session.entity_at(sel_h) {
                            let ck = self.session.entities[idx].card.card_key.clone();
                            self.stl_studio.focus_card(&ck);
                        } else {
                            self.stl_studio.is_open = !self.stl_studio.is_open;
                        }
                    } else {
                        self.stl_studio.is_open = !self.stl_studio.is_open;
                    }
                }
                if menu_actions.open_deck_builder {
                    self.deck_builder.is_open = true;
                }
                if menu_actions.quickload {
                    hud_actions.quickload = true;
                    self.main_menu.has_started_match = true;
                    self.main_menu.is_open = false;
                }
                if let Some(sid) = menu_actions.select_scenario {
                    hud_actions.switch_scenario = Some(sid);
                }
                if menu_actions.roll_seed {
                    hud_actions.roll_new_seed = true;
                }
                if menu_actions.start_solo_ai {
                    self.net.disconnect();
                    self.session.online_local_team = None;
                    self.session.hotseat_mode = false;
                    let sid = self.session.scenario.scenario_id;
                    self.session.load_scenario(sid, self.active_seed);
                    self.main_menu.has_started_match = true;
                    self.main_menu.is_open = false;
                    scene_mutated = true;
                }
                if menu_actions.start_hotseat {
                    self.net.disconnect();
                    self.session.online_local_team = None;
                    self.session.hotseat_mode = true;
                    let sid = self.session.scenario.scenario_id;
                    self.session.load_scenario(sid, self.active_seed);
                    self.main_menu.has_started_match = true;
                    self.main_menu.is_open = false;
                    scene_mutated = true;
                }
                if menu_actions.host_tcp {
                    if let Err(e) = self.net.start_hosting() {
                        self.net.status_line = format!("⚠ {e}");
                    }
                }
                if menu_actions.cancel_tcp {
                    self.net.disconnect();
                }
                if menu_actions.join_tcp {
                    if let Err(e) = self.net.connect_to_host() {
                        self.net.status_line = format!("⚠ {e}");
                    }
                }
                // Also flush any outbound net cmds generated by UI buttons (e.g., Rest or End Turn)
                for out_cmd in self.session.outbound_net_cmds.drain(..) {
                    if self.net.is_online() {
                        self.net.send(NetPacket::Command {
                            cmd: out_cmd.cmd,
                            post_state_hash: out_cmd.post_state_hash,
                        });
                    }
                }
                if hud_actions.toggle_overlays {
                    self.camera.show_overlays = !self.camera.show_overlays;
                    scene_mutated = true;
                }
                if hud_actions.toggle_unit_badges {
                    self.camera.show_unit_badges = !self.camera.show_unit_badges;
                }
                if hud_actions.toggle_terrain_props {
                    self.camera.show_terrain_props = !self.camera.show_terrain_props;
                    scene_mutated = true;
                }
                if hud_actions.toggle_deck_builder {
                    self.deck_builder.is_open = !self.deck_builder.is_open;
                }
                if hud_actions.quicksave {
                    let save_p = crate::paths::writable_app_path(DEFAULT_QUICKSAVE_PATH);
                    match self.session.save_quicksave(&save_p, self.active_seed) {
                        Ok(_) => self.audio.play_ui(SfxCue::BioHealChime),
                        Err(err) => self.session.push_toast(format!("⚠ Save failed: {err}")),
                    }
                }
                if hud_actions.quickload {
                    let load_p = crate::paths::writable_app_path(DEFAULT_QUICKSAVE_PATH);
                    match self.session.load_quicksave(&load_p) {
                        Ok(restored_seed) => {
                            self.active_seed = restored_seed;
                            self.audio.play_ui(SfxCue::UiCardTurn);
                            scene_mutated = true;
                        }
                        Err(err) => self.session.push_toast(format!("⚠ {err}")),
                    }
                }
                if let Some((slot_idx, custom_deck)) = launched_custom_deck {
                    self.session.apply_custom_deck(slot_idx, custom_deck);
                    scene_mutated = true;
                }
                if hud_actions.rematch_current {
                    let sid = self.session.scenario.scenario_id;
                    self.session.load_scenario(sid, self.active_seed);
                    scene_mutated = true;
                }
                if let Some(sid) = hud_actions.switch_scenario {
                    self.session.load_scenario(sid, self.active_seed);
                    scene_mutated = true;
                }
                if hud_actions.roll_new_seed {
                    self.active_seed = self.active_seed.wrapping_add(0x9E37_79B9);
                    let sid = self.session.scenario.scenario_id;
                    self.session.load_scenario(sid, self.active_seed);
                    scene_mutated = true;
                }
                if let Some(preset) = hud_actions.switch_deck {
                    self.session.switch_deck_preset(preset);
                    scene_mutated = true;
                }
                if confirm_end_turn_now {
                    self.end_turn_confirm_open = false;
                    self.session.end_active_turn(&mut self.vfx, &mut self.audio);
                    scene_mutated = true;
                } else if hud_actions.end_turn && self.request_end_turn() {
                    scene_mutated = true;
                }

                let (hex_tiles, instances) = if scene_mutated {
                    let (ht, inst, pb) = build_live_gpu_scene(&self.session, &self.vfx, &self.handles, self.camera.show_terrain_props);
                    self.pick_boxes = pb;
                    if let Some(eng) = self.engine.as_ref() {
                        if let Some(window) = eng.window.as_ref() {
                            window.set_title(&format!(
                                "Wildfront Battlegrounds — {} [Seed 0x{:08X}]",
                                self.session.scenario.title, self.active_seed
                            ));
                        }
                    }
                    (ht, inst)
                } else {
                    (hex_tiles, instances)
                };
                let (beams, beam_cnt, debris, debris_cnt) = self.vfx.pack_gpu_buffers();

                if let Some(eng) = self.engine.as_mut() {
                    eng.upload_live_scene_and_vfx(
                        &hex_tiles,
                        &instances,
                        &beams,
                        beam_cnt,
                        &debris,
                        debris_cnt,
                    );
                    let uniforms = self.camera.build_uniforms(
                        eng.config.width,
                        eng.config.height,
                        eng.instance_count,
                        self.vfx.clock_seconds,
                        self.session.turn_number,
                    );
                    eng.render_frame(uniforms, &paint_jobs, &textures_delta);
                }
            }
            _ => {}
        }
    }
}

fn read_u32_le(cursor: &mut &[u8]) -> u32 {
    let (head, rest) = cursor.split_at(4);
    *cursor = rest;
    u32::from_le_bytes([head[0], head[1], head[2], head[3]])
}

fn read_f32_le(cursor: &mut &[u8]) -> f32 {
    let (head, rest) = cursor.split_at(4);
    *cursor = rest;
    f32::from_le_bytes([head[0], head[1], head[2], head[3]])
}

fn load_precompiled_vox_64tree() -> MultiAssetGpuBuffers {
    let mut dec = ZlibDecoder::new(EMBEDDED_VOX_64TREE_ZLIB);
    let mut raw = Vec::new();
    dec.read_to_end(&mut raw)
        .expect("Failed decompressing embedded precompiled_vox_64tree.bin");
    let mut cur: &[u8] = &raw;
    assert!(
        cur.len() >= 8 && &cur[..8] == b"WF64TR01",
        "Corrupt embedded precompiled_vox_64tree magic"
    );
    cur = &cur[8..];

    let mut palette_linear_rgba = [[0.0_f32; 4]; 256];
    for slot in &mut palette_linear_rgba {
        for ch in slot {
            *ch = read_f32_le(&mut cur);
        }
    }

    let node_count = read_u32_le(&mut cur) as usize;
    let mut nodes = Vec::with_capacity(node_count);
    for _ in 0..node_count {
        nodes.push(GpuNode64 {
            mask_lo: read_u32_le(&mut cur),
            mask_hi: read_u32_le(&mut cur),
            child_base_ptr: read_u32_le(&mut cur),
            flags_and_lod_rgba: read_u32_le(&mut cur),
        });
    }

    let brick_count = read_u32_le(&mut cur) as usize;
    let mut bricks = Vec::with_capacity(brick_count);
    for _ in 0..brick_count {
        let mask_lo = read_u32_le(&mut cur);
        let mask_hi = read_u32_le(&mut cur);
        let lod_rgba = read_u32_le(&mut cur);
        let _pad = read_u32_le(&mut cur);
        let mut packed_mats = [0u32; 16];
        for word in &mut packed_mats {
            *word = read_u32_le(&mut cur);
        }
        bricks.push(GpuLeafBrick {
            mask_lo,
            mask_hi,
            lod_rgba,
            _pad,
            packed_mats,
        });
    }

    let handle_count = read_u32_le(&mut cur) as usize;
    let mut handles = BTreeMap::new();
    for _ in 0..handle_count {
        let slug_len = read_u32_le(&mut cur) as usize;
        let (slug_bytes, rest) = cur.split_at(slug_len);
        cur = rest;
        let slug = String::from_utf8(slug_bytes.to_vec()).expect("UTF-8 handle slug");
        let root_node_index = read_u32_le(&mut cur);
        let size_voxels = [
            read_u32_le(&mut cur),
            read_u32_le(&mut cur),
            read_u32_le(&mut cur),
        ];
        handles.insert(
            slug,
            CompiledModelHandle {
                root_node_index,
                size_voxels,
            },
        );
    }

    MultiAssetGpuBuffers {
        nodes,
        bricks,
        palette_linear_rgba,
        handles,
    }
}

fn load_roster_and_decks() -> (RosterManifest, [DeckList; 2]) {
    let roster: RosterManifest = if let Some(db_path) = crate::paths::resolve_asset_path("compiled/wildfront_roster.db") {
        match wf_core::sqlite_db::load_roster_from_sqlite(&db_path) {
            Ok(r) if !r.units.is_empty() => {
                println!("[wf_client] Loaded {} units from SQLite DB ({}).", r.units.len(), db_path.display());
                r
            }
            _ => {
                let raw_roster = crate::paths::resolve_asset_path("compiled/master_roster.ron")
                    .and_then(|p| fs::read_to_string(p).ok())
                    .unwrap_or_else(|| EMBEDDED_MASTER_ROSTER_RON.to_string());
                ron::from_str(&raw_roster).expect("Invalid master_roster.ron")
            }
        }
    } else {
        let raw_roster = crate::paths::resolve_asset_path("compiled/master_roster.ron")
            .and_then(|p| fs::read_to_string(p).ok())
            .unwrap_or_else(|| EMBEDDED_MASTER_ROSTER_RON.to_string());
        ron::from_str(&raw_roster).expect("Invalid master_roster.ron")
    };

    let raw_d0 = crate::paths::resolve_asset_path("compiled/starter_deck_plant_cnidarian.ron")
        .and_then(|p| fs::read_to_string(p).ok())
        .unwrap_or_else(|| EMBEDDED_STARTER_DECK_0_RON.to_string());
    let d0: DeckList = ron::from_str(&raw_d0).expect("Invalid starter_deck_plant_cnidarian.ron");

    let raw_d1 = crate::paths::resolve_asset_path("compiled/starter_deck_mammal_reptile.ron")
        .and_then(|p| fs::read_to_string(p).ok())
        .unwrap_or_else(|| EMBEDDED_STARTER_DECK_1_RON.to_string());
    let d1: DeckList = ron::from_str(&raw_d1).expect("Invalid starter_deck_mammal_reptile.ron");

    (roster, [d0, d1])
}

fn write_rgba_png(
    path: &std::path::Path,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> std::io::Result<()> {
    let row_bytes = width as usize * 4;
    if rgba.len() != row_bytes * height as usize {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("Unexpected RGBA buffer length {}", rgba.len()),
        ));
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    // PNG Sub scanline filter (type 1) + DEFLATE zlib compression via flate2
    let mut filtered = Vec::with_capacity((row_bytes + 1) * height as usize);
    for row in rgba.chunks_exact(row_bytes) {
        filtered.push(1);
        for (i, &byte) in row.iter().enumerate() {
            let left = if i >= 4 { row[i - 4] } else { 0 };
            filtered.push(byte.wrapping_sub(left));
        }
    }

    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&filtered)?;
    let zlib = enc.finish()?;

    let push_chunk = |out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(tag);
        out.extend_from_slice(data);
        let mut crc = 0xFFFF_FFFFu32;
        for &b in tag.iter().chain(data.iter()) {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        out.extend_from_slice(&(crc ^ 0xFFFF_FFFF).to_be_bytes());
    };
    let mut png = Vec::with_capacity(zlib.len() + 64);
    png.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = 6;
    push_chunk(&mut png, b"IHDR", &ihdr);
    push_chunk(&mut png, b"IDAT", &zlib);
    push_chunk(&mut png, b"IEND", &[]);
    std::fs::write(path, png)
}

fn render_headless_screenshot(app: &mut WildfrontApp, out_path: &std::path::Path, view: u8) {
    let width: u32 = 1600;
    let height: u32 = 1000;

    if view == 2 {
        app.main_menu.has_started_match = true;
        app.main_menu.is_open = false;
        app.deck_builder.is_open = true;
        app.map_editor.is_open = false;
        app.codex.is_open = false;
        app.session.fog_enabled = false;
        app.session.update_explored_fog();
    } else if view == 3 {
        app.session
            .load_scenario(ScenarioId::BunkerBreakout, app.active_seed);
        app.main_menu.has_started_match = true;
        app.main_menu.is_open = false;
        app.deck_builder.is_open = false;
        app.map_editor.is_open = true;
        app.map_editor.brush_paint_enabled = true;
        app.map_editor.symmetry_brush = crate::map_editor_ui::SymmetryBrush::Rotational180;
        app.map_editor.selected_biome = wf_core::Biome::RuinedBunkerCaldera;
        app.map_editor.selected_morphology = wf_core::Morphology::BunkerRuins;
        app.map_editor.selected_elevation = 2;
        for target_hex in [Hex::new(0, 1), Hex::new(1, -1)] {
            if !app.session.entities.iter().any(|e| e.hex == target_hex) {
                app.map_editor.paint_hex(&mut app.session, target_hex);
            }
        }
        app.map_editor.selected_morphology = wf_core::Morphology::ShallowFord;
        app.map_editor.selected_elevation = 0;
        if !app.session.entities.iter().any(|e| e.hex == Hex::new(0, 0)) {
            app.map_editor.paint_hex(&mut app.session, Hex::new(0, 0));
        }
        // Leave brush selector set to BunkerRuins @ Elev 2 for Map Studio clarity
        app.map_editor.selected_morphology = wf_core::Morphology::BunkerRuins;
        app.map_editor.selected_elevation = 2;

        app.session.fog_enabled = false;
        app.session.update_explored_fog();
        app.camera.show_unit_badges = false;
        if let Some(unit) = app
            .session
            .entities
            .iter()
            .find(|e| e.team_id == 1 && !e.card.combat_class.is_building())
            .or_else(|| app.session.entities.iter().find(|e| e.team_id == 1))
        {
            app.session.selected_hex = Some(unit.hex);
        }
        app.camera.yaw = 0.56;
        app.camera.pitch = 0.64;
        app.camera.distance = 43.0;
        app.camera.target = Vec3::new(3.0, -0.3, -1.8);
    } else {
        app.main_menu.has_started_match = true;
        app.main_menu.is_open = false;
        app.deck_builder.is_open = false;
        app.map_editor.is_open = false;
        app.session.fog_enabled = false;
        app.session.update_explored_fog();

        // Deploy 2 distinct Team 1 units from hand onto forward hexes toward the center arena,
        // then run 1 AI opening turn so opposing Team 1 / Team 2 3D voxel units showcase on the sunlit board.
        for _ in 0..2 {
            let p1_mun = app
                .session
                .economy
                .home_base_munitions
                .get(&1)
                .copied()
                .unwrap_or(0);
            let p1_act = app
                .session
                .economy
                .actions_remaining
                .get(&1)
                .copied()
                .unwrap_or(0);
            if p1_act == 0 {
                break;
            }
            let mut chosen: Option<(usize, hexx::Hex)> = None;
            for (idx, card_key) in app.session.hand.iter().enumerate() {
                let Some(card) = app.session.card_def(card_key).cloned() else {
                    continue;
                };
                if card.combat_class == wf_core::CombatClass::TacticalDeployment
                    || card.combat_class == wf_core::CombatClass::Artifact
                    || card.combat_class.is_building()
                {
                    continue;
                }
                if u16::from(card.encumbrance) > p1_mun {
                    continue;
                }
                let valid = app.session.valid_deploy_hexes_for_team(1, card_key);
                if let Some(best_hex) = valid
                    .into_iter()
                    .min_by_key(|h| h.unsigned_distance_to(hexx::Hex::ZERO))
                {
                    chosen = Some((idx, best_hex));
                    break;
                }
            }
            if let Some((hand_idx, dst_hex)) = chosen {
                app.session.mode = crate::game_session::CommandMode::DeployHandCard(hand_idx);
                app.session
                    .handle_hex_click(dst_hex, &mut app.vfx, &mut app.audio);
            } else {
                break;
            }
        }
        app.session.end_active_turn(&mut app.vfx, &mut app.audio);
        app.session.fog_enabled = false;
        app.vfx.tick(0.35);

        if let Some(vanguard) = app
            .session
            .entities
            .iter()
            .filter(|e| e.team_id == 1 && e.unit.current_str_hp > 0 && !e.card.combat_class.is_building())
            .min_by_key(|e| e.hex.unsigned_distance_to(hexx::Hex::ZERO))
        {
            app.session.selected_hex = Some(vanguard.hex);
        } else if let Some(first_p1) = app.session.entities.iter().find(|e| e.team_id == 1) {
            app.session.selected_hex = Some(first_p1.hex);
        }

        // Center the arena inside the open viewport to the right of the 305px inspector sidebar
        // and above the bottom hand tray so both teams, bridges, and the center canyon frame cleanly.
        app.camera.yaw = 0.56;
        app.camera.pitch = 0.66;
        app.camera.distance = 39.5;
        app.camera.target = Vec3::new(-4.2, -0.4, 3.2);
    }

    let (hex_tiles, instances, pick_boxes) = build_live_gpu_scene(
        &app.session,
        &app.vfx,
        &app.handles,
        app.camera.show_terrain_props,
    );
    app.pick_boxes = pick_boxes;

    let mut engine = pollster::block_on(VoxelRenderEngine::new_offscreen(
        width,
        height,
        &app.vox_gpu,
        &hex_tiles,
        &instances,
    ));

    let target_tex = engine.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("wildfront-offscreen-target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target_tex.create_view(&wgpu::TextureViewDescriptor::default());
    let bytes_per_row = width * 4;
    let staging = engine.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("wildfront-offscreen-staging"),
        size: (bytes_per_row * height) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    app.egui_ctx.set_pixels_per_point(1.0);

    for frame_idx in 0..4u32 {
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width as f32, height as f32),
            )),
            viewports: std::iter::once((
                egui::ViewportId::ROOT,
                egui::ViewportInfo {
                    native_pixels_per_point: Some(1.0),
                    ..Default::default()
                },
            ))
            .collect(),
            time: Some(frame_idx as f64 / 60.0),
            predicted_dt: 1.0 / 60.0,
            ..Default::default()
        };
        let show_ov = app.camera.show_overlays;
        let show_badges = app.camera.show_unit_badges
            && !app.deck_builder.is_open
            && !app.main_menu.is_open
            && !app.session.hotseat_handoff_pending;
        let vp_mat = app.camera.view_proj_matrix(width, height);
        let cam_dist = app.camera.distance;

        let full_output = app.egui_ctx.run_ui(raw_input, |root_ui| {
            if !app.main_menu.is_open && !app.deck_builder.is_open {
                let _ = draw_tactical_hud(
                    root_ui,
                    &mut app.session,
                    &mut app.vfx,
                    &mut app.audio,
                    show_ov,
                    app.camera.show_unit_badges,
                    app.camera.show_terrain_props,
                );
            }
            let viewport_rect = root_ui.available_rect_before_wrap();
            draw_world_space_overlays(
                root_ui,
                viewport_rect,
                vp_mat,
                cam_dist,
                &app.session,
                &app.vfx,
                &app.pick_boxes,
                show_badges,
            );
            let _ = app.map_editor.show(root_ui, &mut app.session, &mut app.audio);
            let _ = app.deck_builder.show(
                root_ui,
                &app.session.roster,
                &app.starter_presets,
                &mut app.audio,
            );
        });
        let paint_jobs = app
            .egui_ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);
        let (beams, beam_cnt, debris, debris_cnt) = app.vfx.pack_gpu_buffers();
        engine.upload_live_scene_and_vfx(
            &hex_tiles,
            &instances,
            &beams,
            beam_cnt,
            &debris,
            debris_cnt,
        );
        let uniforms = app.camera.build_uniforms(
            width,
            height,
            engine.instance_count,
            app.vfx.clock_seconds,
            app.session.turn_number,
        );
        engine.render_offscreen_frame(
            uniforms,
            &paint_jobs,
            &full_output.textures_delta,
            &target_tex,
            &target_view,
            if frame_idx == 3 { Some(&staging) } else { None },
        );
    }

    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        let _ = tx.send(res);
    });
    let _ = engine.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv()
        .expect("GPU readback channel closed")
        .expect("GPU staging buffer map failed");
    let mapped = slice
        .get_mapped_range()
        .expect("Failed to map staging buffer range");
    write_rgba_png(out_path, width, height, &mapped)
        .expect("Failed to write offscreen screenshot PNG");
    drop(mapped);
    staging.unmap();
}

#[cfg(windows)]
fn attach_parent_console() {
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(dw_process_id: u32) -> i32;
    }
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliAction {
    RunGui,
    Help,
    Version,
    Screenshot {
        out_path: std::path::PathBuf,
        view: u8,
    },
}

fn parse_cli_args_from<I>(args: I) -> CliAction
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    let mut out_path: Option<std::path::PathBuf> = None;
    let mut view: u8 = 1;
    let mut iter = args.into_iter().skip(1);
    while let Some(arg) = iter.next() {
        if arg == "--help" || arg == "-h" {
            return CliAction::Help;
        }
        if arg == "--version" || arg == "-V" {
            return CliAction::Version;
        }
        if arg == "--screenshot" {
            let val = iter
                .next()
                .expect("--screenshot requires an output file path argument");
            out_path = Some(std::path::PathBuf::from(val));
            continue;
        }
        if arg == "--screenshot-view" {
            let val = iter
                .next()
                .expect("--screenshot-view requires a numeric view index (1..=3)");
            if let Some(s) = val.to_str() {
                if let Ok(v) = s.trim().parse::<u8>() {
                    view = v;
                }
            }
            continue;
        }
        if let Some(s) = arg.to_str() {
            if let Some(rest) = s.strip_prefix("--screenshot=") {
                out_path = Some(std::path::PathBuf::from(rest));
            } else if let Some(rest) = s.strip_prefix("--screenshot-view=") {
                if let Ok(v) = rest.trim().parse::<u8>() {
                    view = v;
                }
            }
        }
    }
    if let Some(out_path) = out_path {
        CliAction::Screenshot { out_path, view }
    } else {
        CliAction::RunGui
    }
}

fn print_cli_help() {
    println!("WildFront BattleGrounds (wf_client v{})", env!("CARGO_PKG_VERSION"));
    println!("Copyright CRV Science Network (https://crvscience.com) — GPL-3.0-or-later");
    println!();
    println!("USAGE:");
    println!("  wf_client [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("  -h, --help                     Print this help message and exit");
    println!("  -V, --version                  Print version information and exit");
    println!("  --screenshot <PATH>            Render headless 1600x1000 wgpu PNG and exit");
    println!("  --screenshot-view <1|2|3>      Select view (1: Arena, 2: Deck Builder, 3: Map Studio)");
}

fn main() {
    #[cfg(windows)]
    if std::env::args_os().len() > 1 {
        attach_parent_console();
    }

    let cli_action = parse_cli_args_from(std::env::args_os());
    match &cli_action {
        CliAction::Help => {
            print_cli_help();
            return;
        }
        CliAction::Version => {
            println!(
                "wf_client {} (WildFront BattleGrounds — CRV Science Network, GPL-3.0-or-later)",
                env!("CARGO_PKG_VERSION")
            );
            return;
        }
        _ => {}
    }

    println!("=================================================================");
    println!("  WILDFRONT BATTLEGROUNDS — PLAYABLE VOXEL & EGUI CLIENT (v{} — Tile-Culled 64-Tree Raymarcher, 2-Ply AI, F3 Map Studio & F4 Replay Theater)", env!("CARGO_PKG_VERSION"));
    println!("=================================================================");
    println!("{}\n", WILDFRONT_THEME_2072);

    let (roster, decks) = load_roster_and_decks();
    let vox_gpu = load_precompiled_vox_64tree();
    let handles = vox_gpu.handles.clone();
    println!(
        "[wf_client] Loaded {} Sparse 64-Tree rigs ({} nodes, {} bricks) + {} balanced cards.",
        handles.len(),
        vox_gpu.nodes.len(),
        vox_gpu.bricks.len(),
        roster.units.len()
    );

    let seed = 0x2072_0814;
    let starter_presets = decks.clone();
    let initial_builder_deck = decks[0].clone();
    let session = ClientMatchState::new(roster, decks, ScenarioId::EmergenceSkirmish, seed, 0);

    let mut app = WildfrontApp {
        vox_gpu,
        handles,
        session,
        main_menu: MainMenuScreen::default(),
        codex: CodexOverlay::default(),
        stl_studio: StlExportStudio::default(),
        dice_roller: DiceRollerStudio::default(),
        db_manager: DatabaseManagerStudio::default(),
        map_editor: MapEditorStudio::default(),
        replay_theater: ReplayTheaterStudio::default(),
        net: NetLockstepManager::default(),
        deck_builder: DeckBuilderScreen::new(initial_builder_deck),
        starter_presets,
        audio: SpatialAudioEngine::new(),
        vfx: VfxManager::default(),
        active_seed: seed,
        camera: OrbitalCamera::default(),
        pick_boxes: Vec::new(),
        egui_ctx: egui::Context::default(),
        egui_state: None,
        engine: None,
        last_frame_instant: Instant::now(),
        show_about: false,
        end_turn_confirm_open: false,
    };
    crate::paths::install_ui_fonts(&app.egui_ctx);
    if let CliAction::Screenshot { out_path, view } = cli_action {
        render_headless_screenshot(&mut app, &out_path, view);
        return;
    }

    let event_loop = EventLoop::new().expect("Failed to initialize winit EventLoop");
    event_loop.run_app(&mut app).expect("EventLoop terminated with error");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn cli_parser_supports_help_version_and_screenshot_views() {
        assert_eq!(
            parse_cli_args_from([OsString::from("wf_client"), OsString::from("--help")]),
            CliAction::Help
        );
        assert_eq!(
            parse_cli_args_from([OsString::from("wf_client"), OsString::from("--version")]),
            CliAction::Version
        );
        assert_eq!(
            parse_cli_args_from([
                OsString::from("wf_client"),
                OsString::from("--screenshot"),
                OsString::from("out.png"),
                OsString::from("--screenshot-view"),
                OsString::from("3"),
            ]),
            CliAction::Screenshot {
                out_path: std::path::PathBuf::from("out.png"),
                view: 3,
            }
        );
    }

    #[test]
    fn write_rgba_png_outputs_deflate_compressed_valid_png() {
        let w = 256u32;
        let h = 128u32;
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                rgba.push(((x * 2) % 256) as u8);
                rgba.push(((y * 2) % 256) as u8);
                rgba.push(42);
                rgba.push(255);
            }
        }
        let tmp = std::env::temp_dir().join("wf_deflate_png_test.png");
        write_rgba_png(&tmp, w, h, &rgba).expect("write_rgba_png failed");
        let bytes = std::fs::read(&tmp).expect("read png failed");
        let _ = std::fs::remove_file(&tmp);

        assert_eq!(&bytes[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        assert!(
            bytes.len() < rgba.len() / 4,
            "Expected DEFLATE PNG ({} bytes) to be much smaller than raw RGBA ({} bytes)",
            bytes.len(),
            rgba.len()
        );
    }

    #[test]
    fn modal_escape_priority_dismisses_about_before_menu_or_command_mode() {
        let mut show_about = true;
        let mut replay_open = false;
        let mut map_open = false;
        let mut db_open = false;
        let mut dice_open = false;
        let mut stl_open = false;
        let mut codex_open = false;
        let mut deck_open = false;
        let mut mode = crate::game_session::CommandMode::DeployHandCard(0);
        let mut main_menu_open = true;

        handle_escape_priority(
            &mut show_about,
            &mut replay_open,
            &mut map_open,
            &mut db_open,
            &mut dice_open,
            &mut stl_open,
            &mut codex_open,
            &mut deck_open,
            &mut mode,
            true,
            &mut main_menu_open,
        );
        assert!(!show_about, "Escape must dismiss About modal first");
        assert!(main_menu_open, "Main menu state must stay preserved while dismissing About");
        assert_eq!(
            mode,
            crate::game_session::CommandMode::DeployHandCard(0),
            "CommandMode must stay preserved while dismissing About"
        );

        handle_escape_priority(
            &mut show_about,
            &mut replay_open,
            &mut map_open,
            &mut db_open,
            &mut dice_open,
            &mut stl_open,
            &mut codex_open,
            &mut deck_open,
            &mut mode,
            true,
            &mut main_menu_open,
        );
        assert_eq!(mode, crate::game_session::CommandMode::Inspect);
        assert!(main_menu_open);

        handle_escape_priority(
            &mut show_about,
            &mut replay_open,
            &mut map_open,
            &mut db_open,
            &mut dice_open,
            &mut stl_open,
            &mut codex_open,
            &mut deck_open,
            &mut mode,
            true,
            &mut main_menu_open,
        );
        assert!(!main_menu_open);
    }

    #[test]
    fn gameplay_hotkeys_and_space_are_blocked_by_any_open_modal_or_curtain() {
        let base = ModalUiSnapshot::default();
        assert!(!base.any_blocking_gameplay_hotkey());
        assert!(base.can_space_request_end_turn());

        let modal_cases = [
            ModalUiSnapshot { main_menu_open: true, ..base },
            ModalUiSnapshot { deck_builder_open: true, ..base },
            ModalUiSnapshot { codex_open: true, ..base },
            ModalUiSnapshot { stl_studio_open: true, ..base },
            ModalUiSnapshot { dice_roller_open: true, ..base },
            ModalUiSnapshot { db_manager_open: true, ..base },
            ModalUiSnapshot { map_editor_open: true, ..base },
            ModalUiSnapshot { replay_theater_open: true, ..base },
            ModalUiSnapshot { about_open: true, ..base },
            ModalUiSnapshot { end_turn_confirm_open: true, ..base },
            ModalUiSnapshot { outcome_modal_open: true, ..base },
            ModalUiSnapshot { hotseat_handoff_open: true, ..base },
        ];
        for snap in modal_cases {
            assert!(
                snap.any_blocking_gameplay_hotkey(),
                "1-5 / N / gameplay hotkeys must be blocked when modal is open: {snap:?}"
            );
            assert!(
                !snap.can_space_request_end_turn(),
                "Space must never end turn when modal is open: {snap:?}"
            );
        }

        // Handoff curtain allows Space reveal ONLY when no other modal is stacked above it
        let handoff_only = ModalUiSnapshot { hotseat_handoff_open: true, ..base };
        assert!(handoff_only.can_space_reveal_handoff());
        assert!(handoff_only.any_blocking_gameplay_hotkey());
        let handoff_under_codex = ModalUiSnapshot {
            hotseat_handoff_open: true,
            codex_open: true,
            ..base
        };
        assert!(!handoff_under_codex.can_space_reveal_handoff());
    }

    #[test]
    fn camera_pan_clamps_to_board_bounds_and_reset_view_restores_defaults() {
        let scen = wf_core::generate_scenario(ScenarioId::EmergenceSkirmish, 0x2072_0814);
        let bounds = board_pan_bounds(&scen.map);
        let mut cam = OrbitalCamera::default();
        cam.target = Vec3::new(9_999.0, 1.6, -9_999.0);
        cam.distance = 62.0;
        cam.yaw = 2.1;
        cam.pitch = 1.2;

        cam.clamp_target_to_bounds(bounds);
        assert!((cam.target.x - bounds.1).abs() < 1e-4);
        assert!((cam.target.z - bounds.2).abs() < 1e-4);
        assert!(bounds.0 < 0.0 && bounds.1 > 0.0);
        assert!(bounds.2 < 0.0 && bounds.3 > 0.0);

        cam.reset_view();
        let def = OrbitalCamera::default();
        assert_eq!(cam.target, def.target);
        assert_eq!(cam.distance, def.distance);
        assert_eq!(cam.yaw, def.yaw);
        assert_eq!(cam.pitch, def.pitch);
    }
}

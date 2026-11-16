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

//! `kira 0.12.6` 3D Spatial Combat Audio Engine for *Wildfront Battlegrounds*.
//!
//! Loads the 8 synthesized `.wav` combat & tactical SFX from `assets/sfx/*.wav`,
//! projects world-space hex coordinates into camera-view space (`Mat4::look_at_rh`),
//! and plays attenuated stereo-spatialized cues via `SpatialTrackBuilder`.
//! Gracefully falls back to silent mode on headless/no-audio systems.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use glam::{Mat4, Vec3};
use kira::listener::ListenerHandle;
use kira::sound::static_sound::StaticSoundData;
use kira::track::{SpatialTrackBuilder, SpatialTrackHandle};
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SfxCue {
    LaserPulse,
    BallisticVolley,
    MeleeClash,
    VoxelExplosion,
    OrbitalDeploy,
    BioHealChime,
    VenomSnare,
    UiCardTurn,
}

impl SfxCue {
    pub const ALL: [(Self, &'static str); 8] = [
        (Self::LaserPulse, "assets/sfx/laser_pulse.wav"),
        (Self::BallisticVolley, "assets/sfx/ballistic_volley.wav"),
        (Self::MeleeClash, "assets/sfx/melee_clash.wav"),
        (Self::VoxelExplosion, "assets/sfx/voxel_explosion.wav"),
        (Self::OrbitalDeploy, "assets/sfx/orbital_deploy.wav"),
        (Self::BioHealChime, "assets/sfx/bio_heal_chime.wav"),
        (Self::VenomSnare, "assets/sfx/venom_snare.wav"),
        (Self::UiCardTurn, "assets/sfx/ui_card_turn.wav"),
    ];
}

struct LiveAudioBackend {
    manager: AudioManager<DefaultBackend>,
    listener: ListenerHandle,
    clips: HashMap<SfxCue, StaticSoundData>,
    active_spatial_tracks: VecDeque<(f32, SpatialTrackHandle)>,
}

pub struct SpatialAudioEngine {
    backend: Option<LiveAudioBackend>,
    pub muted: bool,
    pub master_gain_db: f32,
    pub current_view_mat: Mat4,
}

impl Default for SpatialAudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SpatialAudioEngine {
    pub fn new() -> Self {
        let backend = Self::try_init_kira();
        Self {
            backend,
            muted: false,
            master_gain_db: -2.0,
            current_view_mat: Mat4::IDENTITY,
        }
    }

    fn try_init_kira() -> Option<LiveAudioBackend> {
        let mut manager = match AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()) {
            Ok(m) => m,
            Err(err) => {
                eprintln!("[wf_client::audio] Audio device unavailable ({}); running in silent mode.", err);
                return None;
            }
        };

        // Listener anchored at camera view-space origin (0, 0, 0) with identity orientation
        let listener = manager
            .add_listener(
                mint::Vector3 { x: 0.0, y: 0.0, z: 0.0 },
                mint::Quaternion {
                    v: mint::Vector3 { x: 0.0, y: 0.0, z: 0.0 },
                    s: 1.0,
                },
            )
            .ok()?;

        let mut clips = HashMap::new();
        for (cue, rel_path) in SfxCue::ALL {
            if Path::new(rel_path).exists() {
                if let Ok(data) = StaticSoundData::from_file(rel_path) {
                    clips.insert(cue, data);
                }
            }
        }
        println!(
            "[wf_client::audio] Initialized kira 0.12 spatial mixer ({}/8 WAV cues loaded).",
            clips.len()
        );

        Some(LiveAudioBackend {
            manager,
            listener,
            clips,
            active_spatial_tracks: VecDeque::new(),
        })
    }

    pub fn is_hardware_online(&self) -> bool {
        self.backend.is_some()
    }

    pub fn tick(&mut self, dt: f32, camera_view_mat: Mat4) {
        self.current_view_mat = camera_view_mat;
        let Some(b) = self.backend.as_mut() else {
            return;
        };
        let step = dt.clamp(0.0, 0.1);
        for (ttl, _) in &mut b.active_spatial_tracks {
            *ttl -= step;
        }
        while b.active_spatial_tracks.front().map_or(false, |(ttl, _)| *ttl <= 0.0) {
            b.active_spatial_tracks.pop_front();
        }
    }

    /// Plays a non-spatial UI/turn cue directly on the main mixer track.
    pub fn play_ui(&mut self, cue: SfxCue) {
        if self.muted {
            return;
        }
        let Some(b) = self.backend.as_mut() else {
            return;
        };
        let Some(clip) = b.clips.get(&cue).cloned() else {
            return;
        };
        let _ = b.manager.play(clip.volume(Decibels(self.master_gain_db)));
    }

    /// Plays a 3D spatialized combat/deployment cue positioned at `world_pos` relative to the orbital camera.
    pub fn play_spatial(&mut self, cue: SfxCue, world_pos: Vec3) {
        if self.muted {
            return;
        }
        let view_pos = self.current_view_mat.transform_point3(world_pos) * 0.14;
        let Some(b) = self.backend.as_mut() else {
            return;
        };
        let Some(clip) = b.clips.get(&cue).cloned() else {
            return;
        };

        // Cap simultaneous spatial sub-tracks to 16 so rapid volleys never exhaust mixer slots
        if b.active_spatial_tracks.len() >= 16 {
            b.active_spatial_tracks.pop_front();
        }

        let mint_pos = mint::Vector3 {
            x: view_pos.x,
            y: view_pos.y,
            z: view_pos.z,
        };
        if let Ok(mut track) = b.manager.add_spatial_sub_track(
            &b.listener,
            mint_pos,
            SpatialTrackBuilder::new(),
        ) {
            let _ = track.play(clip.volume(Decibels(self.master_gain_db)));
            b.active_spatial_tracks.push_back((1.15, track));
        }
    }
}

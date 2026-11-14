//! GPU-raymarched volumetric combat VFX (`GpuVfxBeam`) and physically simulated
//! 3D micro-voxel shrapnel debris (`GpuVfxDebris`) + unit recoil/lunge animations.

use std::collections::HashMap;
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use wf_core::mapgen::SplitMix64;

pub const MAX_GPU_VFX_BEAMS: usize = 32;
pub const MAX_GPU_VFX_DEBRIS: usize = 192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum VfxKind {
    LaserBeam = 1,
    BallisticTracer = 2,
    ExplosionSphere = 3,
    BioAbilityRing = 4,
    OrbitalDeployColumn = 5,
}

/// 48-byte (`3 x vec4<f32>`) GPU primitive evaluated volumetrically in `raymarch_64tree.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, Pod, Zeroable)]
pub struct GpuVfxBeam {
    /// `xyz` = world start/center, `w` = radius or current expanding ring radius.
    pub start_and_radius: [f32; 4],
    /// `xyz` = world end/axis, `w` = normalized progress `0.0 .. 1.0`.
    pub end_and_progress: [f32; 4],
    /// `xyz` = linear RGB emissive color, `w` = `VfxKind as u32 as f32` (`0.0` = inactive).
    pub color_and_kind: [f32; 4],
}

/// 32-byte (`2 x vec4<f32>`) 3D voxel debris cube rendered in `raymarch_64tree.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, Pod, Zeroable)]
pub struct GpuVfxDebris {
    /// `xyz` = world center, `w` = cube half-extent in meters (`0.0` = inactive).
    pub pos_and_scale: [f32; 4],
    /// `xyz` = linear RGB albedo/emissive tint, `w` = emissive boost.
    pub color_and_emit: [f32; 4],
}

#[derive(Debug, Clone)]
struct ActiveBeam {
    kind: VfxKind,
    start: Vec3,
    end: Vec3,
    base_radius: f32,
    color_lin: [f32; 3],
    elapsed: f32,
    duration: f32,
}

#[derive(Debug, Clone)]
struct ActiveDebris {
    pos: Vec3,
    vel: Vec3,
    ground_y: f32,
    half_extent: f32,
    color_lin: [f32; 3],
    emissive: f32,
    elapsed: f32,
    lifetime: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct UnitKineticAnim {
    /// World XZ offset vector for melee lunge or ranged recoil kick.
    pub impulse_dir_xz: Vec3,
    pub amplitude_m: f32,
    pub elapsed: f32,
    pub duration: f32,
}


/// Volumetric 3D tumbling D6 physically simulated in world space.
#[derive(Debug, Clone)]
pub struct TumblingDie3D {
    pub pos: Vec3,
    pub vel: Vec3,
    pub ground_y: f32,
    pub half_extent: f32,
    pub color_body: [f32; 3],
    pub color_pip: [f32; 3],
    pub face_value: u8,
    pub ang_rot: Vec3,
    pub ang_vel: Vec3,
    pub elapsed: f32,
    pub settle_time: f32,
    pub lifetime: f32,
}

/// Floating world-space D6 combat roll or tactical event popup rendered in `world_overlays.rs`.
#[derive(Debug, Clone)]
pub struct FloatingCombatPopup {
    pub id: u64,
    pub world_pos: Vec3,
    pub headline: String,
    pub subline: String,
    pub accent_rgb: [u8; 3],
    pub elapsed: f32,
    pub lifetime: f32,
}

pub struct VfxManager {
    beams: Vec<ActiveBeam>,
    debris: Vec<ActiveDebris>,
    pub popups: Vec<FloatingCombatPopup>,
    pub tumbling_dice: Vec<TumblingDie3D>,
    next_popup_id: u64,
    pub unit_anims: HashMap<u64, UnitKineticAnim>,
    pub clock_seconds: f32,
    rng: SplitMix64,
}

impl Default for VfxManager {
    fn default() -> Self {
        Self {
            beams: Vec::new(),
            debris: Vec::new(),
            popups: Vec::new(),
            tumbling_dice: Vec::new(),
            next_popup_id: 1,
            unit_anims: HashMap::new(),
            clock_seconds: 0.0,
            rng: SplitMix64::new(0x5646_585F_2072),
        }
    }
}

impl VfxManager {
    pub fn has_active_effects(&self) -> bool {
        !self.beams.is_empty() || !self.debris.is_empty() || !self.popups.is_empty() || !self.unit_anims.is_empty() || !self.tumbling_dice.is_empty()
    }

    pub fn spawn_popup(
        &mut self,
        world_pos: Vec3,
        headline: impl Into<String>,
        subline: impl Into<String>,
        accent_rgb: [u8; 3],
    ) {
        let id = self.next_popup_id;
        self.next_popup_id += 1;
        self.popups.push(FloatingCombatPopup {
            id,
            world_pos,
            headline: headline.into(),
            subline: subline.into(),
            accent_rgb,
            elapsed: 0.0,
            lifetime: 2.55,
        });
    }

    pub fn trigger_unit_lunge(&mut self, unit_uid: u64, from: Vec3, to: Vec3, amplitude_m: f32) {
        let dir = Vec3::new(to.x - from.x, 0.0, to.z - from.z).normalize_or_zero();
        self.unit_anims.insert(
            unit_uid,
            UnitKineticAnim {
                impulse_dir_xz: dir,
                amplitude_m,
                elapsed: 0.0,
                duration: 0.36,
            },
        );
    }

    pub fn spawn_orbital_deploy(&mut self, hex_anchor: Vec3, faction_rgb: [f32; 3]) {
        self.beams.push(ActiveBeam {
            kind: VfxKind::OrbitalDeployColumn,
            start: hex_anchor,
            end: hex_anchor + Vec3::new(0.0, 9.5, 0.0),
            base_radius: 1.35,
            color_lin: faction_rgb,
            elapsed: 0.0,
            duration: 0.65,
        });
        self.spawn_voxel_shrapnel(hex_anchor + Vec3::Y * 0.3, hex_anchor.y, 14, faction_rgb, 0.7);
    }

    pub fn spawn_laser_attack(&mut self, attacker_uid: u64, from: Vec3, to: Vec3, hit: bool, destroyed: bool) {
        self.trigger_unit_lunge(attacker_uid, to, from, 0.28); // crisp backward recoil
        self.beams.push(ActiveBeam {
            kind: VfxKind::LaserBeam,
            start: from,
            end: to,
            base_radius: 0.16,
            color_lin: [1.0, 0.14, 0.22],
            elapsed: 0.0,
            duration: 0.42,
        });
        if hit {
            self.beams.push(ActiveBeam {
                kind: VfxKind::ExplosionSphere,
                start: to,
                end: to,
                base_radius: if destroyed { 1.85 } else { 1.05 },
                color_lin: [1.0, 0.42, 0.08],
                elapsed: 0.0,
                duration: if destroyed { 0.70 } else { 0.45 },
            });
            let count = if destroyed { 36 } else { 18 };
            self.spawn_voxel_shrapnel(to, to.y - 0.8, count, [1.0, 0.38, 0.10], 0.85);
        }
    }


    pub fn spawn_tumbling_combat_dice(
        &mut self,
        from_pos: Vec3,
        to_pos: Vec3,
        hit_d6: u8,
        dmg_d6: u8,
        hit_succeeded: bool,
    ) {
        let dir = (to_pos - from_pos).normalize_or_zero();
        let ground_y = to_pos.y;
        let right = Vec3::new(-dir.z, 0.0, dir.x).normalize_or_zero();

        // 1. Hit D6 (Cyan / Ice-Blue body with high-contrast White/Cyan pips)
        let hit_origin = from_pos + Vec3::new(0.0, 1.2, 0.0) - right * 0.45;
        let hit_target = to_pos + Vec3::new(0.0, 0.28, 0.0) - right * 0.65;
        let hit_dir = (hit_target - hit_origin).normalize_or_zero();
        let hit_dist = hit_origin.distance(hit_target);
        self.tumbling_dice.push(TumblingDie3D {
            pos: hit_origin,
            vel: hit_dir * (hit_dist * 1.55) + Vec3::new(0.0, 4.4, 0.0),
            ground_y,
            half_extent: 0.22,
            color_body: if hit_succeeded { [0.08, 0.68, 0.88] } else { [0.22, 0.35, 0.46] },
            color_pip: [0.95, 0.98, 1.0],
            face_value: hit_d6,
            ang_rot: Vec3::new(self.rng.next_f32() * 6.28, self.rng.next_f32() * 6.28, 0.0),
            ang_vel: Vec3::new(12.5 + self.rng.next_f32() * 6.0, 15.0 + self.rng.next_f32() * 8.0, 8.0),
            elapsed: 0.0,
            settle_time: 0.68,
            lifetime: 2.85,
        });

        // 2. Damage D6 (Crimson / Ember body with Gold/Amber pips)
        let dmg_origin = from_pos + Vec3::new(0.0, 1.35, 0.0) + right * 0.45;
        let dmg_target = to_pos + Vec3::new(0.0, 0.28, 0.0) + right * 0.65;
        let dmg_dir = (dmg_target - dmg_origin).normalize_or_zero();
        let dmg_dist = dmg_origin.distance(dmg_target);
        self.tumbling_dice.push(TumblingDie3D {
            pos: dmg_origin,
            vel: dmg_dir * (dmg_dist * 1.62) + Vec3::new(0.0, 4.8, 0.0),
            ground_y,
            half_extent: 0.22,
            color_body: [0.88, 0.18, 0.12],
            color_pip: [1.0, 0.84, 0.22],
            face_value: dmg_d6,
            ang_rot: Vec3::new(self.rng.next_f32() * 6.28, self.rng.next_f32() * 6.28, 0.0),
            ang_vel: Vec3::new(-14.0 - self.rng.next_f32() * 6.0, 16.0 + self.rng.next_f32() * 8.0, -9.0),
            elapsed: 0.0,
            settle_time: 0.68,
            lifetime: 2.85,
        });
    }

    pub fn spawn_ballistic_or_melee(
        &mut self,
        attacker_uid: u64,
        from: Vec3,
        to: Vec3,
        is_melee: bool,
        hit: bool,
        destroyed: bool,
    ) {
        if is_melee {
            self.trigger_unit_lunge(attacker_uid, from, to, 0.95);
        } else {
            self.trigger_unit_lunge(attacker_uid, to, from, 0.32);
            self.beams.push(ActiveBeam {
                kind: VfxKind::BallisticTracer,
                start: from,
                end: to,
                base_radius: 0.20,
                color_lin: [1.0, 0.68, 0.12],
                elapsed: 0.0,
                duration: 0.34,
            });
        }
        if hit {
            self.beams.push(ActiveBeam {
                kind: VfxKind::ExplosionSphere,
                start: to,
                end: to,
                base_radius: if destroyed { 1.95 } else { 1.15 },
                color_lin: if destroyed { [1.0, 0.32, 0.06] } else { [0.98, 0.62, 0.16] },
                elapsed: 0.0,
                duration: if destroyed { 0.72 } else { 0.46 },
            });
            let count = if destroyed { 40 } else { 20 };
            self.spawn_voxel_shrapnel(to, to.y - 0.8, count, [0.85, 0.52, 0.18], 0.55);
        }
    }

    pub fn spawn_bio_ability_ring(&mut self, center: Vec3, color_lin: [f32; 3]) {
        self.beams.push(ActiveBeam {
            kind: VfxKind::BioAbilityRing,
            start: center,
            end: center + Vec3::Y * 1.4,
            base_radius: 2.1,
            color_lin,
            elapsed: 0.0,
            duration: 0.62,
        });
        self.spawn_voxel_shrapnel(center + Vec3::Y * 0.4, center.y, 12, color_lin, 0.75);
    }

    pub fn spawn_voxel_shrapnel(
        &mut self,
        origin: Vec3,
        ground_y: f32,
        count: usize,
        tint_lin: [f32; 3],
        emissive: f32,
    ) {
        for _ in 0..count {
            if self.debris.len() >= MAX_GPU_VFX_DEBRIS {
                break;
            }
            let ang = self.rng.next_f32() * std::f32::consts::TAU;
            let spd = 2.2 + self.rng.next_f32() * 5.8;
            let vy = 3.2 + self.rng.next_f32() * 5.6;
            let jitter = 0.75 + self.rng.next_f32() * 0.45;
            self.debris.push(ActiveDebris {
                pos: origin + Vec3::new((self.rng.next_f32() - 0.5) * 0.5, 0.15, (self.rng.next_f32() - 0.5) * 0.5),
                vel: Vec3::new(ang.cos() * spd, vy, ang.sin() * spd),
                ground_y,
                half_extent: 0.065 + self.rng.next_f32() * 0.065,
                color_lin: [
                    (tint_lin[0] * jitter).min(1.0),
                    (tint_lin[1] * jitter).min(1.0),
                    (tint_lin[2] * jitter).min(1.0),
                ],
                emissive,
                elapsed: 0.0,
                lifetime: 0.65 + self.rng.next_f32() * 0.55,
            });
        }
    }

    pub fn tick(&mut self, dt: f32) {
        let clamped_dt = dt.clamp(0.0, 0.05);
        self.clock_seconds += clamped_dt;

        for b in &mut self.beams {
            b.elapsed += clamped_dt;
        }
        self.beams.retain(|b| b.elapsed < b.duration);


        for d in &mut self.tumbling_dice {
            d.elapsed += clamped_dt;
            if d.elapsed < d.settle_time {
                d.vel.y -= 15.0 * clamped_dt;
                d.pos += d.vel * clamped_dt;
                d.ang_rot += d.ang_vel * clamped_dt;
                d.ang_vel *= 0.94; // Air resistance damping
                if d.pos.y - d.half_extent < d.ground_y {
                    d.pos.y = d.ground_y + d.half_extent;
                    d.vel.y = -d.vel.y * 0.45; // Elastic bounce
                    d.vel.x *= 0.72;
                    d.vel.z *= 0.72;
                }
            } else {
                // Settle firmly flat on the ground plane
                d.pos.y = d.ground_y + d.half_extent;
                d.vel = Vec3::ZERO;
                d.ang_rot = Vec3::ZERO;
            }
        }
        self.tumbling_dice.retain(|d| d.elapsed < d.lifetime);

        for d in &mut self.debris {
            d.elapsed += clamped_dt;
            d.vel.y -= 13.5 * clamped_dt;
            d.pos += d.vel * clamped_dt;
            if d.pos.y - d.half_extent < d.ground_y {
                d.pos.y = d.ground_y + d.half_extent;
                d.vel.y = -d.vel.y * 0.42;
                d.vel.x *= 0.72;
                d.vel.z *= 0.72;
            }
        }
        self.debris.retain(|d| d.elapsed < d.lifetime);

        for anim in self.unit_anims.values_mut() {
            anim.elapsed += clamped_dt;
        }
        self.unit_anims.retain(|_, a| a.elapsed < a.duration);
    }

    pub fn unit_impulse_offset(&self, unit_uid: u64) -> Vec3 {
        let Some(anim) = self.unit_anims.get(&unit_uid) else {
            return Vec3::ZERO;
        };
        let t = (anim.elapsed / anim.duration).clamp(0.0, 1.0);
        let wave = (t * std::f32::consts::PI).sin();
        anim.impulse_dir_xz * (anim.amplitude_m * wave)
    }

    pub fn pack_gpu_buffers(&self) -> (Vec<GpuVfxBeam>, u32, Vec<GpuVfxDebris>, u32) {
        let mut out_beams = vec![GpuVfxBeam::default(); MAX_GPU_VFX_BEAMS];
        let mut b_cnt = 0usize;
        for b in self.beams.iter().take(MAX_GPU_VFX_BEAMS) {
            let prog = (b.elapsed / b.duration).clamp(0.0, 1.0);
            let fade = 1.0 - prog;
            let radius = match b.kind {
                VfxKind::ExplosionSphere | VfxKind::BioAbilityRing => b.base_radius * (0.25 + 0.85 * prog),
                VfxKind::OrbitalDeployColumn => b.base_radius * (1.0 - 0.65 * prog),
                _ => b.base_radius * (0.6 + 0.4 * fade),
            };
            out_beams[b_cnt] = GpuVfxBeam {
                start_and_radius: [b.start.x, b.start.y, b.start.z, radius],
                end_and_progress: [b.end.x, b.end.y, b.end.z, prog],
                color_and_kind: [
                    b.color_lin[0] * fade,
                    b.color_lin[1] * fade,
                    b.color_lin[2] * fade,
                    (b.kind as u32) as f32,
                ],
            };
            b_cnt += 1;
        }

        let mut out_debris = vec![GpuVfxDebris::default(); MAX_GPU_VFX_DEBRIS];
        let mut d_cnt = 0usize;

        // Render 3D Tumbling Dice cubes directly in the GPU voxel raymarcher
        for die in self.tumbling_dice.iter() {
            if d_cnt >= MAX_GPU_VFX_DEBRIS.saturating_sub(10) {
                break;
            }
            let fade = ((die.lifetime - die.elapsed) / 0.40).clamp(0.0, 1.0);
            let he = die.half_extent * fade;
            if he <= 0.01 { continue; }

            // Main Die Body Cube
            out_debris[d_cnt] = GpuVfxDebris {
                pos_and_scale: [die.pos.x, die.pos.y, die.pos.z, he],
                color_and_emit: [die.color_body[0], die.color_body[1], die.color_body[2], 0.25 * fade],
            };
            d_cnt += 1;

            // Render top pip face dots
            let pip_col = die.color_pip;
            let pip_r = he * 0.22;
            let py = die.pos.y + he + pip_r * 0.5;
            let ox = he * 0.52;
            let oz = he * 0.52;

            let mut add_pip = |dx: f32, dz: f32| {
                if d_cnt < MAX_GPU_VFX_DEBRIS {
                    out_debris[d_cnt] = GpuVfxDebris {
                        pos_and_scale: [die.pos.x + dx, py, die.pos.z + dz, pip_r],
                        color_and_emit: [pip_col[0], pip_col[1], pip_col[2], 0.85 * fade],
                    };
                    d_cnt += 1;
                }
            };

            match die.face_value {
                1 => add_pip(0.0, 0.0),
                2 => { add_pip(-ox, -oz); add_pip(ox, oz); }
                3 => { add_pip(-ox, -oz); add_pip(0.0, 0.0); add_pip(ox, oz); }
                4 => { add_pip(-ox, -oz); add_pip(ox, -oz); add_pip(-ox, oz); add_pip(ox, oz); }
                5 => { add_pip(-ox, -oz); add_pip(ox, -oz); add_pip(0.0, 0.0); add_pip(-ox, oz); add_pip(ox, oz); }
                6 => { add_pip(-ox, -oz); add_pip(ox, -oz); add_pip(-ox, 0.0); add_pip(ox, 0.0); add_pip(-ox, oz); add_pip(ox, oz); }
                _ => add_pip(0.0, 0.0),
            }
        }

        for d in self.debris.iter().take(MAX_GPU_VFX_DEBRIS) {
            let life_left = (1.0 - d.elapsed / d.lifetime).clamp(0.0, 1.0);
            out_debris[d_cnt] = GpuVfxDebris {
                pos_and_scale: [d.pos.x, d.pos.y, d.pos.z, d.half_extent * life_left.sqrt()],
                color_and_emit: [
                    d.color_lin[0],
                    d.color_lin[1],
                    d.color_lin[2],
                    d.emissive * life_left,
                ],
            };
            d_cnt += 1;
        }

        (out_beams, b_cnt as u32, out_debris, d_cnt as u32)
    }
}

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

//! `wgpu 30.0` + `egui-wgpu 0.37.0` Sparse 64-Tree Compute Raymarcher, Volumetric Combat VFX,
//! and Combined Swapchain Blit + Immediate-Mode Tactical HUD Pipeline.

use std::sync::Arc;
use bytemuck::{Pod, Zeroable};
use egui::ClippedPrimitive;
use egui_wgpu::{Renderer as EguiRenderer, RendererOptions, ScreenDescriptor};
use wgpu::util::DeviceExt;
use winit::window::Window;
use wf_core::MultiAssetGpuBuffers;
use crate::scene::{GpuHexOverlay, GpuUnitInstance, HEX_WORLD_RADIUS};
use crate::vfx::{GpuVfxBeam, GpuVfxDebris, MAX_GPU_VFX_BEAMS, MAX_GPU_VFX_DEBRIS};

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct CameraUniforms {
    pub inv_view_proj: [[f32; 4]; 4],
    pub camera_pos_spread: [f32; 4],
    pub sun_dir_flags: [f32; 4],
    pub resolution: [u32; 2],
    pub instance_count: u32,
    pub hex_radius: f32,
    pub vfx_beam_count: u32,
    pub vfx_debris_count: u32,
    pub time_seconds: f32,
    pub atmosphere_mode: u32,
}

pub struct VoxelRenderEngine {
    pub window: Option<Arc<Window>>,
    pub surface: Option<wgpu::Surface<'static>>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub cam_buffer: wgpu::Buffer,
    pub tree_node_buffer: wgpu::Buffer,
    pub leaf_brick_buffer: wgpu::Buffer,
    pub palette_buffer: wgpu::Buffer,
    pub instance_buffer: wgpu::Buffer,
    pub hex_buffer: wgpu::Buffer,
    pub vfx_beam_buffer: wgpu::Buffer,
    pub vfx_debris_buffer: wgpu::Buffer,
    pub storage_tex_view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub compute_pipeline: wgpu::ComputePipeline,
    pub compute_bind_group: wgpu::BindGroup,
    pub blit_pipeline: wgpu::RenderPipeline,
    pub blit_bind_group: wgpu::BindGroup,
    pub egui_renderer: EguiRenderer,
    pub instance_count: u32,
    pub vfx_beam_count: u32,
    pub vfx_debris_count: u32,
}

const MAX_SCENARIO_INSTANCES: usize = 256;

impl VoxelRenderEngine {
    pub async fn new(
        window: Arc<Window>,
        vox_gpu: &MultiAssetGpuBuffers,
        hex_tiles: &[GpuHexOverlay],
        instances: &[GpuUnitInstance],
    ) -> Self {
        let size = window.inner_size();
        Self::init_with_surface(
            Some(window),
            size.width.max(64),
            size.height.max(64),
            vox_gpu,
            hex_tiles,
            instances,
        )
        .await
    }

    pub async fn new_offscreen(
        width: u32,
        height: u32,
        vox_gpu: &MultiAssetGpuBuffers,
        hex_tiles: &[GpuHexOverlay],
        instances: &[GpuUnitInstance],
    ) -> Self {
        Self::init_with_surface(
            None,
            width.max(64),
            height.max(64),
            vox_gpu,
            hex_tiles,
            instances,
        )
        .await
    }

    async fn init_with_surface(
        window: Option<Arc<Window>>,
        width: u32,
        height: u32,
        vox_gpu: &MultiAssetGpuBuffers,
        hex_tiles: &[GpuHexOverlay],
        instances: &[GpuUnitInstance],
    ) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = window.as_ref().map(|w| {
            instance
                .create_surface(w.clone())
                .expect("Failed to create wgpu window surface")
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: surface.as_ref(),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .expect("No suitable GPU adapter found");

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Wildfront Voxel + egui Device"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                memory_hints: Default::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                trace: wgpu::Trace::Off,
            })
            .await
            .expect("Failed to request wgpu device");

        let (format, alpha_mode) = if let Some(ref s) = surface {
            let caps = s.get_capabilities(&adapter);
            let fmt = caps
                .formats
                .iter()
                .copied()
                .find(|f| f.is_srgb())
                .unwrap_or(caps.formats[0]);
            (fmt, caps.alpha_modes[0])
        } else {
            (
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::CompositeAlphaMode::Opaque,
            )
        };

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        if let Some(ref s) = surface {
            s.configure(&device, &config);
        }

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("raymarch_64tree.wgsl"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("shaders/raymarch_64tree.wgsl").into(),
            ),
        });

        let cam_init = CameraUniforms {
            inv_view_proj: glam::Mat4::IDENTITY.to_cols_array_2d(),
            camera_pos_spread: [0.0, 18.0, 26.0, 0.0015],
            sun_dir_flags: [0.48, 0.78, 0.40, 1.0],
            resolution: [width, height],
            instance_count: instances.len() as u32,
            hex_radius: HEX_WORLD_RADIUS,
            vfx_beam_count: 0,
            vfx_debris_count: 0,
            time_seconds: 0.0,
            atmosphere_mode: 0,
        };
        let cam_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Camera Uniforms"),
            contents: bytemuck::bytes_of(&cam_init),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let tree_node_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("64-Tree Nodes Storage"),
            contents: bytemuck::cast_slice(&vox_gpu.nodes),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let leaf_brick_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("64-Tree Leaf Bricks Storage"),
            contents: bytemuck::cast_slice(&vox_gpu.bricks),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let palette_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Linear RGBA Palette"),
            contents: bytemuck::cast_slice(&vox_gpu.palette_linear_rgba),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let mut padded_instances = vec![GpuUnitInstance::default(); MAX_SCENARIO_INSTANCES];
        for (dst, src) in padded_instances.iter_mut().zip(instances.iter()) {
            *dst = *src;
        }
        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Voxel Unit Instances"),
            contents: bytemuck::cast_slice(&padded_instances),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let hex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Procedural Hex Grid Storage"),
            contents: bytemuck::cast_slice(hex_tiles),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let init_beams = vec![GpuVfxBeam::default(); MAX_GPU_VFX_BEAMS];
        let vfx_beam_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Combat VFX Beams Storage"),
            contents: bytemuck::cast_slice(&init_beams),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let init_debris = vec![GpuVfxDebris::default(); MAX_GPU_VFX_DEBRIS];
        let vfx_debris_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Combat VFX Debris Storage"),
            contents: bytemuck::cast_slice(&init_debris),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let storage_tex_view = Self::create_raymarch_texture(&device, width, height);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Blit Linear Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Sparse 64-Tree + Combat VFX Compute Pipeline"),
            layout: None,
            module: &shader,
            entry_point: Some("cs_main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Swapchain Blit Pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_blit"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(if format.is_srgb() { "fs_blit" } else { "fs_blit_unorm" }),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let compute_bind_group = Self::build_compute_bind_group(
            &device,
            &compute_pipeline,
            &cam_buffer,
            &tree_node_buffer,
            &leaf_brick_buffer,
            &palette_buffer,
            &instance_buffer,
            &hex_buffer,
            &storage_tex_view,
            &vfx_beam_buffer,
            &vfx_debris_buffer,
        );
        let blit_bind_group =
            Self::build_blit_bind_group(&device, &blit_pipeline, &storage_tex_view, &sampler);

        let egui_renderer = EguiRenderer::new(&device, format, RendererOptions::default());

        Self {
            window,
            surface,
            device,
            queue,
            config,
            cam_buffer,
            tree_node_buffer,
            leaf_brick_buffer,
            palette_buffer,
            instance_buffer,
            hex_buffer,
            vfx_beam_buffer,
            vfx_debris_buffer,
            storage_tex_view,
            sampler,
            compute_pipeline,
            compute_bind_group,
            blit_pipeline,
            blit_bind_group,
            egui_renderer,
            instance_count: instances.len().min(MAX_SCENARIO_INSTANCES) as u32,
            vfx_beam_count: 0,
            vfx_debris_count: 0,
        }
    }

    pub fn upload_live_scene_and_vfx(
        &mut self,
        hex_tiles: &[GpuHexOverlay],
        instances: &[GpuUnitInstance],
        beams: &[GpuVfxBeam],
        beam_count: u32,
        debris: &[GpuVfxDebris],
        debris_count: u32,
    ) {
        self.queue
            .write_buffer(&self.hex_buffer, 0, bytemuck::cast_slice(hex_tiles));
        let mut padded = vec![GpuUnitInstance::default(); MAX_SCENARIO_INSTANCES];
        for (dst, src) in padded.iter_mut().zip(instances.iter()) {
            *dst = *src;
        }
        self.queue
            .write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&padded));
        self.instance_count = instances.len().min(MAX_SCENARIO_INSTANCES) as u32;

        self.queue
            .write_buffer(&self.vfx_beam_buffer, 0, bytemuck::cast_slice(beams));
        self.queue
            .write_buffer(&self.vfx_debris_buffer, 0, bytemuck::cast_slice(debris));
        self.vfx_beam_count = beam_count;
        self.vfx_debris_count = debris_count;
    }

    fn create_raymarch_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Raymarch Output RGBA8"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        tex.create_view(&wgpu::TextureViewDescriptor::default())
    }

    #[allow(clippy::too_many_arguments)]
    fn build_compute_bind_group(
        device: &wgpu::Device,
        pipeline: &wgpu::ComputePipeline,
        cam_buffer: &wgpu::Buffer,
        tree_node_buffer: &wgpu::Buffer,
        leaf_brick_buffer: &wgpu::Buffer,
        palette_buffer: &wgpu::Buffer,
        instance_buffer: &wgpu::Buffer,
        hex_buffer: &wgpu::Buffer,
        storage_tex_view: &wgpu::TextureView,
        vfx_beam_buffer: &wgpu::Buffer,
        vfx_debris_buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Compute Raymarch + VFX BindGroup"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: cam_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: tree_node_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: leaf_brick_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: palette_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: instance_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: hex_buffer.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(storage_tex_view),
                },
                wgpu::BindGroupEntry { binding: 7, resource: vfx_beam_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: vfx_debris_buffer.as_entire_binding() },
            ],
        })
    }

    fn build_blit_bind_group(
        device: &wgpu::Device,
        pipeline: &wgpu::RenderPipeline,
        storage_tex_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Swapchain Blit BindGroup"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(storage_tex_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        if let Some(surface) = self.surface.as_ref() {
            surface.configure(&self.device, &self.config);
        }
        self.storage_tex_view = Self::create_raymarch_texture(&self.device, width, height);
        self.compute_bind_group = Self::build_compute_bind_group(
            &self.device,
            &self.compute_pipeline,
            &self.cam_buffer,
            &self.tree_node_buffer,
            &self.leaf_brick_buffer,
            &self.palette_buffer,
            &self.instance_buffer,
            &self.hex_buffer,
            &self.storage_tex_view,
            &self.vfx_beam_buffer,
            &self.vfx_debris_buffer,
        );
        self.blit_bind_group = Self::build_blit_bind_group(
            &self.device,
            &self.blit_pipeline,
            &self.storage_tex_view,
            &self.sampler,
        );
    }

    pub fn render_frame(
        &mut self,
        cam: CameraUniforms,
        paint_jobs: &[ClippedPrimitive],
        textures_delta: &egui::TexturesDelta,
    ) {
        let Some(surface) = self.surface.as_ref() else {
            return;
        };
        let output = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => return,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                surface.configure(&self.device, &self.config);
                return;
            }
        };

        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let pixels_per_point = self
            .window
            .as_ref()
            .map(|w| w.scale_factor() as f32)
            .unwrap_or(1.0);
        self.render_to_view(
            cam,
            paint_jobs,
            textures_delta,
            &view,
            pixels_per_point,
            None,
        );
        if let Some(window) = self.window.as_ref() {
            window.pre_present_notify();
        }
        self.queue.present(output);
    }

    pub fn render_offscreen_frame(
        &mut self,
        cam: CameraUniforms,
        paint_jobs: &[ClippedPrimitive],
        textures_delta: &egui::TexturesDelta,
        target_tex: &wgpu::Texture,
        target_view: &wgpu::TextureView,
        staging_copy: Option<&wgpu::Buffer>,
    ) {
        self.render_to_view(
            cam,
            paint_jobs,
            textures_delta,
            target_view,
            1.0,
            staging_copy.map(|buf| (target_tex, buf)),
        );
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }

    fn render_to_view(
        &mut self,
        mut cam: CameraUniforms,
        paint_jobs: &[ClippedPrimitive],
        textures_delta: &egui::TexturesDelta,
        view: &wgpu::TextureView,
        pixels_per_point: f32,
        staging_copy: Option<(&wgpu::Texture, &wgpu::Buffer)>,
    ) {
        cam.resolution = [self.config.width, self.config.height];
        cam.instance_count = self.instance_count;
        cam.vfx_beam_count = self.vfx_beam_count;
        cam.vfx_debris_count = self.vfx_debris_count;
        self.queue
            .write_buffer(&self.cam_buffer, 0, bytemuck::bytes_of(&cam));

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Raymarch + Blit + egui Encoder"),
            });

        let screen_descriptor = ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point,
        };

        for (id, image_deltas) in &textures_delta.set {
            for image_delta in image_deltas {
                self.egui_renderer
                    .update_texture(&self.device, &self.queue, *id, image_delta);
            }
        }
        let mut user_cmd_bufs = self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            paint_jobs,
            &screen_descriptor,
        );

        // 1. Compute pass: raymarch procedural hex terrain + Sparse 64-Tree rigs + volumetric VFX
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Sparse 64-Tree + VFX Compute Pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.compute_pipeline);
            cpass.set_bind_group(0, &self.compute_bind_group, &[]);
            let gx = self.config.width.div_ceil(8);
            let gy = self.config.height.div_ceil(8);
            cpass.dispatch_workgroups(gx, gy, 1);
        }

        // 2. Combined Render pass: fullscreen raymarch blit + immediate-mode egui HUD overlay
        {
            let rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Fullscreen Blit + egui HUD Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let mut rpass_static = rpass.forget_lifetime();
            rpass_static.set_pipeline(&self.blit_pipeline);
            rpass_static.set_bind_group(0, &self.blit_bind_group, &[]);
            rpass_static.draw(0..3, 0..1);

            self.egui_renderer
                .render(&mut rpass_static, paint_jobs, &screen_descriptor);
        }

        for id in &textures_delta.free {
            self.egui_renderer.free_texture(id);
        }

        if let Some((src_tex, dst_buf)) = staging_copy {
            let bytes_per_row = self.config.width * 4;
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: src_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: dst_buf,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(bytes_per_row),
                        rows_per_image: Some(self.config.height),
                    },
                },
                wgpu::Extent3d {
                    width: self.config.width,
                    height: self.config.height,
                    depth_or_array_layers: 1,
                },
            );
        }

        user_cmd_bufs.push(encoder.finish());
        self.queue.submit(user_cmd_bufs);
    }
}

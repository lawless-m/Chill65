//! The window: a wgpu surface, the tube renderer, and a keyboard.
//!
//! Deliberately plain. `tube-shell` has the parameter panel, the debug views
//! and the shader hot-reload; this has a picture and the controls, because it
//! is a way to play a game rather than a way to tune a tube. The rendering
//! path is the same one `tube-shell` uses — surface, [`Field`], present blit —
//! and is mirrored from `app.rs` rather than invented afresh.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use tube_renderer::{DepositMode, Field, FieldShaders, TubeParams, TubeProfile, View};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::live::LiveMachine;
use crate::machine::{apply_key, Game};

/// Matches `tube-shell`'s own display height, so the deposit buffer and the
/// tube profile behave identically here.
const DISPLAY_HEIGHT: u32 = 512;

/// The tube these games were played on: a colour XY monitor, landscape 4:3.
///
/// `TubeParams::default()` is `TubeProfile::VECTREX`, which is **portrait** 3:4
/// — the right tube for the machine Trexy was written against and the wrong one
/// for either of ours. Both boards drive a landscape face: Space Duel's full
/// scale is 256 by 192 (`vg::FULL_SCALE_X`) and Tempest's screen-boundary
/// diagnostic paints a box on a 4:3 tube (`te::FULL_SCALE_X`). Nothing else in
/// the shipped profile is claimed to fit — a Wells-Gardner 6100 profile does
/// not exist yet — but the aspect is not a matter of taste and is fixed here.
fn tube_params() -> TubeParams {
    TubeParams {
        profile: TubeProfile {
            aspect_w: 4.0,
            aspect_h: 3.0,
        },
        ..TubeParams::default()
    }
}

/// The shaders the renderer needs, plus the blit that puts it on screen.
const SHADERS: [&str; 11] = [
    "deposit.wgsl",
    "deposit_splat.wgsl",
    "deposit_resolve.wgsl",
    "phosphor.wgsl",
    "deposit_total.wgsl",
    "readout.wgsl",
    "blur.wgsl",
    "tonemap.wgsl",
    "view.wgsl",
    "sample_points.wgsl",
    "present.wgsl",
];

/// Where the renderer's WGSL lives, relative to this crate.
///
/// The same sibling checkout the path dependency points at: if the renderer
/// compiled, its shaders are there too.
pub fn shader_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../Trexy/crates/tube-renderer/shaders")
}

/// Read every shader, naming the first one that is missing.
pub fn load_shaders() -> Result<Vec<(&'static str, String)>, String> {
    let dir = shader_dir();
    SHADERS
        .iter()
        .map(|name| {
            std::fs::read_to_string(dir.join(name))
                .map(|src| (*name, src))
                .map_err(|e| format!("{}: {e}", dir.join(name).display()))
        })
        .collect()
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PresentUniform {
    fit: [f32; 2],
    exposure: f32,
    _pad: f32,
}

/// Report the adapter and the shaders without opening a window.
///
/// This is the unattended stand-in for launching the thing: everything the
/// window needs, proved present, on a machine nobody is sitting at.
pub fn self_check() -> Result<(), String> {
    let instance = instance();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        ..Default::default()
    }))
    .map_err(|e| format!("no Vulkan adapter: {e}"))?;

    let info = adapter.get_info();
    println!(
        "adapter: {} ({:?}, {:?})\ndriver:  {} {}",
        info.name, info.backend, info.device_type, info.driver, info.driver_info
    );
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("self-check"),
        ..Default::default()
    }))
    .map_err(|e| format!("adapter found but no device: {e}"))?;
    println!("device:  ok");

    let shaders = load_shaders()?;
    println!(
        "shaders: {} of {} found in {}",
        shaders.len(),
        SHADERS.len(),
        shader_dir().display()
    );
    Ok(())
}

/// Vulkan first, as the renderer's own notes require; `WGPU_BACKEND` overrides.
fn instance() -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::from_env().unwrap_or(wgpu::Backends::VULKAN);
    wgpu::Instance::new(descriptor)
}

/// Open the window and play whichever game the corpus holds.
///
/// `view` is the renderer's readout, and it is selectable — with `Tab` while
/// running — because the default one currently cannot be played on. Trexy's
/// **beauty** pass loses a vector picture: isolated strokes disappear and only
/// the places where strokes pile up survive, so Tempest's web vanishes and its
/// text comes through as blobs. The energy readout shows the same field
/// faithfully and decays as the phosphor does, so it is playable. This is not
/// our end of the wire — the deposit the renderer is given is correct, which
/// `--capture` and `tube-shell --view deposit` demonstrate together — so the
/// switch is a way round it and not a fix.
pub fn run(corpus: PathBuf, view: View) -> Result<(), String> {
    let shaders = load_shaders()?;
    let game = Game::detect(&corpus)?;
    println!("{}. Keys:\n{}", game.title(), game.bindings());
    println!("  Tab          readout, now {}\n", view.name());

    let event_loop = EventLoop::new().map_err(|e| format!("no event loop: {e}"))?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        live: LiveMachine::spawn(game, corpus),
        shaders,
        state: None,
        view,
    };
    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("the event loop stopped: {e}"))
}

/// Everything that needs a surface, so it can be built once the window exists.
struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    field: Field,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    present_buffer: wgpu::Buffer,
}

struct App {
    live: LiveMachine,
    shaders: Vec<(&'static str, String)>,
    state: Option<State>,
    view: View,
}

impl App {
    fn source(&self, name: &str) -> &str {
        &self
            .shaders
            .iter()
            .find(|(n, _)| *n == name)
            .expect("every shader was loaded before the window opened")
            .1
    }

    fn build(&mut self, event_loop: &ActiveEventLoop) -> Result<State, String> {
        let attrs = Window::default_attributes()
            .with_title(format!("Chill65 — {}", self.live.game().title()))
            .with_inner_size(winit::dpi::LogicalSize::new(900.0, 1000.0));
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .map_err(|e| format!("no window: {e}"))?,
        );

        let instance = instance();
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("no surface: {e}"))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            ..Default::default()
        }))
        .map_err(|e| format!("no adapter: {e}"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("chill65-window"),
            ..Default::default()
        }))
        .map_err(|e| format!("no device: {e}"))?;

        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        // The tonemap emits linear display values, so the surface encodes.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .map(|c| wgpu::SurfaceConfiguration { format, ..c })
            .ok_or("surface is not supported by this adapter")?;
        surface.configure(&device, &config);

        let field = Field::new(
            &device,
            &queue,
            DISPLAY_HEIGHT,
            tube_params(),
            FieldShaders {
                deposit: self.source("deposit.wgsl"),
                splat: self.source("deposit_splat.wgsl"),
                resolve: self.source("deposit_resolve.wgsl"),
                phosphor: self.source("phosphor.wgsl"),
                deposit_total: self.source("deposit_total.wgsl"),
                readout: self.source("readout.wgsl"),
                blur: self.source("blur.wgsl"),
                tonemap: self.source("tonemap.wgsl"),
                view: self.source("view.wgsl"),
                sample_points: self.source("sample_points.wgsl"),
            },
            self.live.elapsed(),
        );

        let (pipeline, bind_group, present_buffer) =
            present_pipeline(&device, &config, self.source("present.wgsl"), &field);

        Ok(State {
            window,
            surface,
            device,
            queue,
            config,
            field,
            pipeline,
            bind_group,
            present_buffer,
        })
    }
}

/// The blit that letterboxes the tube face into the window.
fn present_pipeline(
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
    source: &str,
    field: &Field,
) -> (wgpu::RenderPipeline, wgpu::BindGroup, wgpu::Buffer) {
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("present"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("present"),
        size: size_of::<PresentUniform>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("present"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("present.wgsl"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("present"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("present"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(config.format.into())],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("present"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(field.output_view()),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buffer.as_entire_binding(),
            },
        ],
    });
    (pipeline, bind_group, buffer)
}

/// Fit the tube's aspect into the window without distorting it.
fn fit(field: &Field, config: &wgpu::SurfaceConfiguration) -> [f32; 2] {
    let tube = field.output_width() as f32 / field.output_height() as f32;
    let window = config.width as f32 / config.height as f32;
    if window > tube {
        [tube / window, 1.0]
    } else {
        [1.0, window / tube]
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match self.build(event_loop) {
            Ok(state) => self.state = Some(state),
            Err(e) => {
                eprintln!("chill65-window: {e}");
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(state) = &mut self.state {
                    state.config.width = size.width.max(1);
                    state.config.height = size.height.max(1);
                    state.surface.configure(&state.device, &state.config);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                if code == KeyCode::Escape {
                    event_loop.exit();
                    return;
                }
                if code == KeyCode::Tab && event.state == ElementState::Pressed && !event.repeat {
                    let next = View::ALL
                        .iter()
                        .position(|v| *v == self.view)
                        .map_or(0, |i| (i + 1) % View::ALL.len());
                    self.view = View::ALL[next];
                    println!("readout: {}", self.view.name());
                    return;
                }
                // winit repeats a held key; a repeat must not queue a second
                // coin, and the switch is already in the state the repeat
                // would set.
                if event.repeat {
                    return;
                }
                let pressed = event.state == ElementState::Pressed;
                let mut controls = self.live.controls();
                if apply_key(&mut controls, code, pressed) {
                    self.live.set_controls(controls);
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = &self.state {
            state.window.request_redraw();
        }
    }
}

impl App {
    fn redraw(&mut self) {
        let now = self.live.elapsed();
        let Some(state) = &mut self.state else {
            return;
        };

        use wgpu::CurrentSurfaceTexture as Acquired;
        let frame = match state.surface.get_current_texture() {
            Acquired::Success(frame) => frame,
            Acquired::Suboptimal(frame) => {
                state.surface.configure(&state.device, &state.config);
                frame
            }
            Acquired::Outdated | Acquired::Lost => {
                state.surface.configure(&state.device, &state.config);
                return;
            }
            other => {
                eprintln!("dropped a frame: {other:?}");
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let uniform = PresentUniform {
            fit: fit(&state.field, &state.config),
            // The chain has already tonemapped; this only places it.
            exposure: 1.0,
            _pad: 0.0,
        };
        state
            .queue
            .write_buffer(&state.present_buffer, 0, bytemuck::bytes_of(&uniform));

        // Everything the field has not caught up with yet.
        let window = self.live.window(state.field.simulated() as f32, now as f32);
        state.field.advance(
            &state.device,
            &state.queue,
            &window,
            now,
            DepositMode::Analytic,
        );
        state
            .field
            .render(&state.device, &state.queue, self.view, &window);

        let mut encoder = state
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("present"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("present"),
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
            pass.set_pipeline(&state.pipeline);
            pass.set_bind_group(0, &state.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        state.queue.submit([encoder.finish()]);
        state.queue.present(frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The renderer's shaders must be where the path dependency says they are.
    /// No GPU needed, and it fails with a name rather than a panic.
    #[test]
    fn every_shader_the_window_needs_is_present() {
        let shaders = load_shaders().expect("the renderer's shaders");
        assert_eq!(shaders.len(), SHADERS.len());
        for (name, src) in &shaders {
            assert!(!src.trim().is_empty(), "{name} is empty");
        }
    }
}

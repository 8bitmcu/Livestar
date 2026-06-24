//! Native renderer for the Livestar live wallpaper.
//!
//! The Java side captures the wallpaper `Surface` and hands it across the JNI
//! boundary. Here we turn it into an `ANativeWindow`, build a raw-window-handle,
//! and let wgpu spin up its Vulkan backend on top of it.
//!
//! The scene is a starfield: on every launch we randomize a fresh set of stars
//! scattered across the screen. Each star is 1-4 pixels, with its own base
//! brightness; a subset pulse their intensity over time (twinkle) at varied
//! speeds, depths, and dimming waveforms so no two look quite alike. The
//! Java engine drives a per-frame loop (via Choreographer) while the wallpaper
//! is visible so the twinkle stays animated.

use std::ffi::c_void;
use std::ptr::NonNull;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use jni::objects::{JClass, JObject};
use jni::sys::{jboolean, jfloat, jint, jlong};
use jni::JNIEnv;
use ndk::native_window::NativeWindow;
use raw_window_handle::{
    AndroidDisplayHandle, AndroidNdkWindowHandle, RawDisplayHandle, RawWindowHandle,
};

/// Tiny self-contained PRNG (xorshift64*). Avoids pulling in an extra crate and
/// lets us seed from the wall clock so the starfield differs each launch.
struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        // Avoid the all-zero state, which xorshift cannot escape.
        Self {
            state: seed | 0x9E3779B97F4A7C15,
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    /// Uniform float in [0, 1).
    fn next_f32(&mut self) -> f32 {
        // Top 24 bits give a full-precision f32 mantissa.
        ((self.next_u64() >> 40) as f32) / ((1u32 << 24) as f32)
    }

    /// Uniform float in [lo, hi).
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next_f32()
    }
}

/// One star, laid out for direct upload as an instance attribute buffer.
/// Fields are packed tightly (28 bytes) and read by the vertex shader.
#[repr(C)]
#[derive(Clone, Copy)]
struct Star {
    /// Center in normalized device coordinates, x/y each in [-1, 1].
    center: [f32; 2],
    /// Size in pixels (1.0 - 4.0).
    size: f32,
    /// Base brightness in [0, 1].
    brightness: f32,
    /// Twinkle angular speed (rad/s); varies widely so some pulse fast, some slow.
    speed: f32,
    /// Twinkle phase offset so pulsing stars aren't synchronized.
    phase: f32,
    /// Twinkle depth in [0, 1]. 1.0 dims fully to black, ~0 barely flickers,
    /// 0 for non-twinkling stars.
    amp: f32,
    /// Dimming waveform selector (0=sine, 1=triangle, 2=flicker, 3=eased).
    curve: f32,
}

const STAR_FLOATS: usize = 8;
const STAR_BYTES: usize = STAR_FLOATS * 4;

/// User-tunable starfield parameters, forwarded from the Java settings UI.
#[derive(Clone, Copy)]
struct Config {
    /// Star count factor in [0, 1] applied to the area-derived base count.
    density: f32,
    /// Minimum star size in pixels.
    size_min: f32,
    /// Maximum star size in pixels.
    size_max: f32,
    /// Brightness multiplier in [0, 1].
    brightness: f32,
    /// Fraction of stars that twinkle, in [0, 1].
    twinkle: f32,
    /// Request a low-power GPU adapter to save battery.
    battery_saving: bool,
}

/// Unit-quad corners (two triangles) expanded around each star center.
const QUAD: [[f32; 2]; 6] = [
    [-1.0, -1.0],
    [1.0, -1.0],
    [1.0, 1.0],
    [-1.0, -1.0],
    [1.0, 1.0],
    [-1.0, 1.0],
];

const SHADER: &str = r#"
struct Globals {
    time: f32,
    _pad: f32,
    resolution: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) is_bg: f32,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vs_main(
    @location(0) corner: vec2<f32>,
    @location(1) center: vec2<f32>,
    @location(2) size: f32,
    @location(3) brightness: f32,
    @location(4) speed: f32,
    @location(5) phase: f32,
    @location(6) amp: f32,
    @location(7) curve: f32,
) -> VsOut {
    var out: VsOut;
    if (size < 0.0) {
        // Background mode: cover full screen using the unit quad corners.
        out.pos = vec4<f32>(corner, 0.0, 1.0);
        out.is_bg = 1.0;
        out.uv = corner * 0.5 + 0.5; // Map [-1, 1] to [0, 1]
        out.color = vec3<f32>(0.0);
    } else {
        // Star mode: expand a quad around the center point.
        let half = vec2<f32>(size / globals.resolution.x, size / globals.resolution.y);
        let p = center + corner * half;

        // Twinkle: oscillate brightness using one of several waveforms so
        // different stars dim with different pattern curves.
        let angle = globals.time * speed + phase;
        let tau = 6.2831853;
        let t = fract(angle / tau); // normalized cycle position in [0, 1)
        var s = 0.5 + 0.5 * sin(angle); // curve 0: smooth sine
        if (curve > 2.5) {
            // curve 3: eased sine - lingers longer at full bright/full dim.
            s = smoothstep(0.0, 1.0, s);
        } else if (curve > 1.5) {
            // curve 2: flicker - mostly dim with brief, sharp bright flashes.
            s = pow(s, 4.0);
        } else if (curve > 0.5) {
            // curve 1: triangle - linear, even fade in and out.
            s = 1.0 - abs(2.0 * t - 1.0);
        }
        let factor = (1.0 - amp) + amp * s;
        let intensity = brightness * factor;

        out.pos = vec4<f32>(p, 0.0, 1.0);
        out.is_bg = 0.0;
        // Carry the quad corner ([-1, 1] on each axis) so the fragment stage can
        // shape the star with a radial profile instead of a flat square.
        out.uv = corner;
        // Vary star color slightly: white to pale yellow.
        out.color = vec3<f32>(1.0, 0.95 + 0.05 * phase, 0.8 + 0.2 * brightness) * intensity;
    }
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (in.is_bg > 0.5) {
        // Vertical gradient: 1.0 at top, 0.0 at bottom.
        let v = in.uv.y;

        // Slow cycle between Sunset (0.0) and Sunrise (1.0).
        let cycle = 0.5 + 0.5 * sin(globals.time * 0.05);

        // Sunset palette
        let sunset_top = vec3<f32>(0.02, 0.05, 0.15);
        let sunset_mid = vec3<f32>(0.9, 0.4, 0.1);
        let sunset_bot = vec3<f32>(0.4, 0.1, 0.0);

        // Sunrise palette
        let sunrise_top = vec3<f32>(0.2, 0.5, 0.8);
        let sunrise_mid = vec3<f32>(1.0, 0.7, 0.7);
        let sunrise_bot = vec3<f32>(1.0, 0.9, 0.6);

        let top = mix(sunset_top, sunrise_top, cycle);
        let mid = mix(sunset_mid, sunrise_mid, cycle);
        let bot = mix(sunset_bot, sunrise_bot, cycle);

        var sky: vec3<f32>;
        if (v > 0.5) {
            sky = mix(mid, top, (v - 0.5) * 2.0);
        } else {
            sky = mix(bot, mid, v * 2.0);
        }

        return vec4<f32>(sky, 1.0);
    } else {
        // Shape the star as a round point rather than a hard square. `uv` is the
        // quad corner in [-1, 1], so its length is the normalized distance from
        // the star center (0 at the middle, 1 at the quad edge).
        let d = length(in.uv);
        // A solid-ish bright core with a softly anti-aliased rim...
        let core = smoothstep(0.55, 0.30, d);
        // ...wrapped in a faint glow that fades to nothing by the quad edge. This
        // is what makes the larger (e.g. 10px) stars read as luminous points
        // instead of blocks.
        let glow = smoothstep(1.0, 0.0, d) * 0.55;
        let alpha = clamp(core + glow, 0.0, 1.0);
        if (alpha <= 0.0) {
            discard;
        }
        return vec4<f32>(in.color, alpha);
    }
}
"#;

/// Everything the renderer needs to keep alive between frames. Boxed and handed
/// back to Java as an opaque `jlong` handle.
struct Renderer {
    // Keep the window alive for as long as the surface borrows it.
    _window: NativeWindow,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    globals: wgpu::Buffer,
    quad: wgpu::Buffer,
    instances: wgpu::Buffer,
    star_count: u32,
    start: Instant,
    visible: bool,
}

fn push_f32(buf: &mut Vec<u8>, v: f32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

/// Builds a randomized starfield sized to the screen area, tuned by `cfg`.
fn generate_stars(width: u32, height: u32, rng: &mut Rng, cfg: &Config) -> Vec<Star> {
    let area = (width as u64) * (height as u64);
    let base = (area / 4000).clamp(120, 1500) as f32;
    let count = (base * cfg.density.clamp(0.0, 1.0)).round() as usize;

    // Guard against an inverted or degenerate size range.
    let lo = cfg.size_min.min(cfg.size_max).max(0.0);
    let hi = cfg.size_max.max(cfg.size_min).max(lo + f32::EPSILON);

    let mut stars = Vec::with_capacity(count);

    for _ in 0..count {
        let center = [rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)];
        let size = rng.range(lo, hi);
        let brightness = rng.range(0.5, 1.0) * cfg.brightness;

        let twinkles = rng.next_f32() < cfg.twinkle;
        let (speed, phase, amp, curve) = if twinkles {
            // Wide speed spread so some pulse quickly and others crawl.
            // Squaring biases toward slower stars while still allowing fast ones.
            let s = rng.next_f32();
            let speed = 0.15 + 3.85 * s * s;
            // Amp spans nearly-imperceptible to fully dimming to black.
            let amp = rng.range(0.1, 1.0);
            // Pick one of four dimming waveforms at random.
            let curve = (rng.next_f32() * 4.0).floor().min(3.0);
            (speed, rng.range(0.0, std::f32::consts::TAU), amp, curve)
        } else {
            (0.0, 0.0, 0.0, 0.0)
        };

        stars.push(Star {
            center,
            size,
            brightness,
            speed,
            phase,
            amp,
            curve,
        });
    }
    stars
}

impl Renderer {
    fn new(window: NativeWindow, cfg: Config) -> Option<Self> {
        let width = window.width().max(1) as u32;
        let height = window.height().max(1) as u32;

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::GL,
            flags: wgpu::InstanceFlags::default() | wgpu::InstanceFlags::ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER,
            backend_options: wgpu::BackendOptions::default(),
            display: None,
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        });
        log::info!("Instance created");

        let window_handle = RawWindowHandle::AndroidNdk(AndroidNdkWindowHandle::new(
            window.ptr().cast::<c_void>(),
        ));
        let display_handle = RawDisplayHandle::Android(AndroidDisplayHandle::new());

        // SAFETY: the NativeWindow (and thus the ANativeWindow it points at) is
        // owned by this Renderer and outlives the surface.
        let surface = unsafe {
            instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(display_handle),
                    raw_window_handle: window_handle,
                })
                .map_err(|e| log::error!("create_surface failed: {e}"))
                .ok()?
        };
        log::info!("Surface created");

        let power_preference = if cfg.battery_saving {
            wgpu::PowerPreference::LowPower
        } else {
            wgpu::PowerPreference::HighPerformance
        };
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .map_err(|e| {
            log::error!("request_adapter failed: {e}");
            e
        })
        .ok()?;

        log::info!("Adapter: {:?}", adapter.get_info());

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("livestar-device"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                ..Default::default()
            },
        ))
        .map_err(|e| log::error!("request_device failed: {e}"))
        .ok()?;
        log::info!("Device created");

        let caps = surface.get_capabilities(&adapter);
        log::info!("Caps: {:?}", caps);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: caps.present_modes[0],
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        log::info!("Configuring surface: {:?}", config);
        surface.configure(&device, &config);
        log::info!("Surface configured");

        // Seed the PRNG from the wall clock so each launch is a new starfield.
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x1234_5678_9ABC_DEF0);
        let mut rng = Rng::new(seed);
        let stars = generate_stars(width, height, &mut rng, &cfg);
        let star_count = stars.len() as u32;

        // Pack star instances into a byte buffer.
        let mut instance_bytes = Vec::with_capacity(stars.len() * STAR_BYTES);
        for s in &stars {
            push_f32(&mut instance_bytes, s.center[0]);
            push_f32(&mut instance_bytes, s.center[1]);
            push_f32(&mut instance_bytes, s.size);
            push_f32(&mut instance_bytes, s.brightness);
            push_f32(&mut instance_bytes, s.speed);
            push_f32(&mut instance_bytes, s.phase);
            push_f32(&mut instance_bytes, s.amp);
            push_f32(&mut instance_bytes, s.curve);
        }

        let mut quad_bytes = Vec::with_capacity(QUAD.len() * 8);
        for c in &QUAD {
            push_f32(&mut quad_bytes, c[0]);
            push_f32(&mut quad_bytes, c[1]);
        }

        let quad = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("star-quad"),
            size: quad_bytes.len() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&quad, 0, &quad_bytes);

        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("star-instances"),
            size: instance_bytes.len().max(STAR_BYTES) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&instances, 0, &instance_bytes);

        // Globals: time (f32), resolution (vec2<f32>), pad (f32) = 16 bytes.
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("globals-layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals-bind-group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("starfield-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("starfield-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let quad_attrs = [wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 0,
            shader_location: 0,
        }];
        let instance_attrs = [
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 0,
                shader_location: 1,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32,
                offset: 8,
                shader_location: 2,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32,
                offset: 12,
                shader_location: 3,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32,
                offset: 16,
                shader_location: 4,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32,
                offset: 20,
                shader_location: 5,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32,
                offset: 24,
                shader_location: 6,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32,
                offset: 28,
                shader_location: 7,
            },
        ];

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("starfield-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: 8,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &quad_attrs,
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: STAR_BYTES as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &instance_attrs,
                    },
                ],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    // Alpha-blend so each star's soft radial rim/glow composites
                    // over the background and its neighbors instead of writing an
                    // opaque square.
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let mut renderer = Self {
            _window: window,
            surface,
            device,
            queue,
            config,
            pipeline,
            bind_group,
            globals,
            quad,
            instances,
            star_count,
            start: Instant::now(),
            visible: true,
        };
        renderer.render();
        Some(renderer)
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
        self.render();
    }

    fn render(&mut self) {
        log::debug!("render called");
        if self.config.width == 0 || self.config.height == 0 {
            log::warn!("render called with zero resolution: {}x{}", self.config.width, self.config.height);
            return;
        }
        // Update per-frame globals: elapsed time and current resolution.
        // WGSL struct Globals { time, pad, resolution } = 16 bytes.
        let time = self.start.elapsed().as_secs_f32();
        let mut globals_bytes = Vec::with_capacity(16);
        push_f32(&mut globals_bytes, time);
        push_f32(&mut globals_bytes, 0.0); // pad
        push_f32(&mut globals_bytes, self.config.width as f32);
        push_f32(&mut globals_bytes, self.config.height as f32);
        self.queue.write_buffer(&self.globals, 0, &globals_bytes);

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => texture,
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            err => {
                log::warn!("dropped frame: {:?}", err);
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame-encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("starfield-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if self.star_count > 0 {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.quad.slice(..));
                pass.set_vertex_buffer(1, self.instances.slice(..));
                pass.draw(0..QUAD.len() as u32, 0..self.star_count);
            }
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
    }
}

/// Reconstructs a `&mut Renderer` from the opaque handle handed to Java.
///
/// SAFETY: `handle` must be a non-zero pointer returned by `onSurfaceCreated`
/// and not yet freed by `onSurfaceDestroyed`.
unsafe fn renderer_from_handle<'a>(handle: jlong) -> Option<&'a mut Renderer> {
    NonNull::new(handle as *mut Renderer).map(|mut p| p.as_mut())
}

#[no_mangle]
pub extern "system" fn Java_com_livestar_NativeBridge_onSurfaceCreated(
    env: JNIEnv,
    _class: JClass,
    surface: JObject,
    density: jfloat,
    star_size_min: jfloat,
    star_size_max: jfloat,
    brightness: jfloat,
    twinkle: jfloat,
    battery_saving: jboolean,
) -> jlong {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Debug)
            .with_tag("livestar"),
    );
    log::debug!("JNI onSurfaceCreated");

    // SAFETY: `surface` is a live android.view.Surface passed from the wallpaper
    // engine; from_surface acquires its own reference to the ANativeWindow.
    let window = unsafe {
        NativeWindow::from_surface(env.get_native_interface(), surface.as_raw())
    };
    let Some(window) = window else {
        log::error!("failed to obtain ANativeWindow from Surface");
        return 0;
    };

    let cfg = Config {
        density,
        size_min: star_size_min,
        size_max: star_size_max,
        brightness,
        twinkle,
        battery_saving: battery_saving != 0,
    };

    match Renderer::new(window, cfg) {
        Some(renderer) => Box::into_raw(Box::new(renderer)) as jlong,
        None => {
            log::error!("renderer initialization failed");
            0
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_livestar_NativeBridge_onSurfaceChanged(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    width: jint,
    height: jint,
) {
    // SAFETY: handle validity is guaranteed by the Java caller's lifecycle.
    if let Some(renderer) = unsafe { renderer_from_handle(handle) } {
        renderer.resize(width.max(0) as u32, height.max(0) as u32);
    }
}

#[no_mangle]
pub extern "system" fn Java_com_livestar_NativeBridge_onFrame(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    // log::debug!("onFrame handle: {}", handle);
    // SAFETY: handle validity is guaranteed by the Java caller's lifecycle.
    if let Some(renderer) = unsafe { renderer_from_handle(handle) } {
        if renderer.visible {
            renderer.render();
        }
    } else {
        log::warn!("onFrame called with invalid handle: {}", handle);
    }
}

#[no_mangle]
pub extern "system" fn Java_com_livestar_NativeBridge_onVisibilityChanged(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    visible: jboolean,
) {
    // SAFETY: handle validity is guaranteed by the Java caller's lifecycle.
    if let Some(renderer) = unsafe { renderer_from_handle(handle) } {
        renderer.visible = visible != 0;
        if renderer.visible {
            renderer.render();
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_livestar_NativeBridge_onSurfaceDestroyed(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    if let Some(ptr) = NonNull::new(handle as *mut Renderer) {
        // SAFETY: reclaim the Box created in onSurfaceCreated and drop it.
        unsafe { drop(Box::from_raw(ptr.as_ptr())) };
    }
}

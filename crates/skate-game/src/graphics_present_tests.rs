//! The presentation shader (`graphics_present.wgsl`). Naga validation always runs. With
//! `--ignored`, a real render through wgpu's Vulkan backend (any adapter; lavapipe is enough)
//! checks the supersampling resolve against a CPU box filter in linear light, and the
//! edge-adaptive upscale against bilinear on an edge-rich image.
//!
//! ```text
//! cargo test -p skate-game --bin skate3rust graphics_quality -- --ignored
//! ```
use naga_oil::compose::{ComposableModuleDescriptor, Composer, NagaModuleDescriptor};
use std::borrow::Cow;

/// bevy_ui_render 0.18.1's `ui_vertex_output.wgsl`: what the UI material pipeline feeds the shader.
const UI_VERTEX_OUTPUT: &str = "#define_import_path bevy_ui::ui_vertex_output
struct UiVertexOutput {
    @location(0) uv: vec2<f32>,
    @location(1) border_widths: vec4<f32>,
    @location(2) border_radius: vec4<f32>,
    @location(3) @interpolate(flat) size: vec2<f32>,
    @builtin(position) position: vec4<f32>,
};";

/// Stands in for the UI vertex stage: one triangle over the target, uv (0,0) at the top-left.
const FULLSCREEN: &str = "#import bevy_ui::ui_vertex_output::UiVertexOutput
@vertex
fn vertex(@builtin(vertex_index) index: u32) -> UiVertexOutput {
    let uv = vec2(f32((index << 1u) & 2u), f32(index & 2u));
    var out: UiVertexOutput;
    out.uv = uv;
    out.position = vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.border_widths = vec4(0.0);
    out.border_radius = vec4(0.0);
    out.size = vec2(0.0);
    return out;
}";

/// Composes against the UI interface and validates with no optional capabilities, as WebGPU has.
fn compose(source: &str, path: &str) -> naga::Module {
    let mut composer = Composer::default().with_capabilities(naga::valid::Capabilities::default());
    if let Err(error) = composer.add_composable_module(ComposableModuleDescriptor {
        source: UI_VERTEX_OUTPUT,
        file_path: "ui_vertex_output",
        ..Default::default()
    }) {
        panic!("{}", error.emit_to_string(&composer));
    }
    let module = composer
        .make_naga_module(NagaModuleDescriptor { source, file_path: path, ..Default::default() })
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&composer)));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::default())
        .validate(&module)
        .unwrap();
    module
}

#[test]
fn present_shader_validates_without_optional_capabilities() {
    compose(include_str!("graphics_present.wgsl"), "graphics_present");
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

fn gpu() -> Gpu {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor { backends: wgpu::Backends::VULKAN, ..Default::default() });
    let adapter = bevy::tasks::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        .expect("a Vulkan adapter (Mesa's lavapipe is enough)");
    let (device, queue) = bevy::tasks::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    Gpu { device, queue }
}

/// Runs the shader in `mode` over an sRGB source, into an sRGB target as the UI pass does.
fn render(gpu: &Gpu, mode: u32, source: &[u8], (sw, sh): (u32, u32), (ow, oh): (u32, u32)) -> Vec<u8> {
    let device = &gpu.device;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let size = |width, height| wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: size(sw, sh),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    gpu.queue.write_texture(
        texture.as_image_copy(),
        source,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * sw), rows_per_image: Some(sh) },
        size(sw, sh),
    );
    // Bevy's `ImageSampler::linear()`: linear filtering, clamp to edge.
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    // `Params`: two vec2s and a u32, padded to the WGSL struct size of 24 bytes.
    let mut params: Vec<u8> = [sw as f32, sh as f32, ow as f32, oh as f32].iter().flat_map(|v| v.to_le_bytes()).collect();
    params.extend(mode.to_le_bytes());
    params.extend([0; 4]);
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: params.len() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    gpu.queue.write_buffer(&uniform, 0, &params);
    let module = |source, path| {
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Naga(Cow::Owned(compose(source, path))),
        })
    };
    let vertex = module(FULLSCREEN, "fullscreen");
    let fragment = module(include_str!("graphics_present.wgsl"), "graphics_present");
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: None,
        vertex: wgpu::VertexState { module: &vertex, entry_point: Some("vertex"), compilation_options: Default::default(), buffers: &[] },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &fragment,
            entry_point: Some("fragment"),
            compilation_options: Default::default(),
            targets: &[Some(format.into())],
        }),
        multiview: None,
        cache: None,
    });
    let view = texture.create_view(&Default::default());
    let view_group = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &pipeline.get_bind_group_layout(0), entries: &[] });
    let material = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(1),
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
        ],
    });
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: size(ow, oh),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let row = (4 * ow).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row * oh),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let target_view = target.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &view_group, &[]);
        pass.set_bind_group(1, &material, &[]);
        pass.draw(0..3, 0..1);
    }
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(oh) },
        },
        size(ow, oh),
    );
    gpu.queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |result| result.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let mapped = readback.slice(..).get_mapped_range();
    mapped.chunks(row as usize).flat_map(|line| line[..4 * ow as usize].to_vec()).collect()
}

fn to_linear(c: u8) -> f32 {
    let c = f32::from(c) / 255.;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn to_srgb(l: f32) -> u8 {
    let s = if l <= 0.003_130_8 { l * 12.92 } else { 1.055 * l.powf(1. / 2.4) - 0.055 };
    (s * 255.).round().clamp(0., 255.) as u8
}

/// The largest per-channel difference over the colour channels.
fn worst(a: &[u8], b: &[u8]) -> u8 {
    a.chunks(4).zip(b.chunks(4)).flat_map(|(p, q)| (0..3).map(move |c| p[c].abs_diff(q[c]))).max().unwrap()
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let (sum, count) = a.chunks(4).zip(b.chunks(4)).fold((0., 0.), |(s, n), (p, q)| {
        (s + (0..3).map(|c| (f64::from(p[c]) - f64::from(q[c])).powi(2)).sum::<f64>(), n + 3.)
    });
    10. * (255f64.powi(2) / (sum / count)).log10()
}

/// Box filter by an integer factor, in linear light.
fn downsample(image: &[u8], (w, h): (u32, u32), factor: u32) -> Vec<u8> {
    let (ow, oh) = (w / factor, h / factor);
    let mut out = Vec::with_capacity((4 * ow * oh) as usize);
    for y in 0..oh {
        for x in 0..ow {
            for c in 0..4 {
                let mut sum = 0.;
                for dy in 0..factor {
                    for dx in 0..factor {
                        sum += to_linear(image[(4 * ((y * factor + dy) * w + x * factor + dx) + c) as usize]);
                    }
                }
                let mean = sum / (factor * factor) as f32;
                out.push(if c == 3 { (mean * 255.).round() as u8 } else { to_srgb(mean) });
            }
        }
    }
    out
}

/// What the stock `ImageNode` shows: hardware bilinear over an sRGB texture, in linear light.
fn bilinear(image: &[u8], (w, h): (u32, u32), (ow, oh): (u32, u32)) -> Vec<u8> {
    let at = |x: i64, y: i64, c: usize| to_linear(image[4 * (y.clamp(0, h as i64 - 1) * w as i64 + x.clamp(0, w as i64 - 1)) as usize + c]);
    let mut out = Vec::with_capacity((4 * ow * oh) as usize);
    for y in 0..oh {
        for x in 0..ow {
            let px = (x as f32 + 0.5) * w as f32 / ow as f32 - 0.5;
            let py = (y as f32 + 0.5) * h as f32 / oh as f32 - 0.5;
            let (fx, fy) = (px.floor(), py.floor());
            let (tx, ty) = (px - fx, py - fy);
            let (ix, iy) = (fx as i64, fy as i64);
            for c in 0..3 {
                let top = at(ix, iy, c) * (1. - tx) + at(ix + 1, iy, c) * tx;
                let bottom = at(ix, iy + 1, c) * (1. - tx) + at(ix + 1, iy + 1, c) * tx;
                out.push(to_srgb(top * (1. - ty) + bottom * ty));
            }
            out.push(255);
        }
    }
    out
}

/// Edge-rich ground truth: a gradient, a rotated square, a disc and thin stripes, each pixel
/// supersampled 4x4 so its edges are properly anti-aliased.
fn scene((w, h): (u32, u32)) -> Vec<u8> {
    let shade = |x: f32, y: f32| -> [f32; 3] {
        let (sin, cos) = 0.35f32.sin_cos();
        let (rx, ry) = ((x - 0.35) * cos - (y - 0.4) * sin, (x - 0.35) * sin + (y - 0.4) * cos);
        if rx.abs() < 0.18 && ry.abs() < 0.18 {
            return [0.02, 0.03, 0.05];
        }
        if (x - 0.72).powi(2) + (y - 0.62).powi(2) < 0.16f32.powi(2) {
            return [0.9, 0.6, 0.1];
        }
        if y > 0.8 && ((x + y) * 24.).fract() < 0.35 {
            return [0.85, 0.85, 0.85];
        }
        [0.15 + 0.4 * x, 0.2, 0.25 + 0.3 * y]
    };
    let mut out = Vec::with_capacity((4 * w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0.; 3];
            for sy in 0..4 {
                for sx in 0..4 {
                    let s = shade((x as f32 + (sx as f32 + 0.5) / 4.) / w as f32, (y as f32 + (sy as f32 + 0.5) / 4.) / h as f32);
                    for c in 0..3 {
                        sum[c] += s[c] / 16.;
                    }
                }
            }
            out.extend(sum.map(to_srgb));
            out.push(255);
        }
    }
    out
}

fn noise((w, h): (u32, u32), mut seed: u32) -> Vec<u8> {
    (0..4 * w * h)
        .map(|i| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            if i % 4 == 3 { 255 } else { (seed >> 24) as u8 }
        })
        .collect()
}

#[test]
#[ignore = "needs a Vulkan adapter (Mesa's lavapipe is enough)"]
fn supersampling_at_200_percent_resolves_to_the_2x2_box_in_linear_light() {
    let source = noise((64, 48), 7);
    let shown = render(&gpu(), 2, &source, (64, 48), (32, 24));
    assert!(worst(&shown, &downsample(&source, (64, 48), 2)) <= 1);
}

#[test]
#[ignore = "needs a Vulkan adapter (Mesa's lavapipe is enough)"]
fn supersampling_at_150_percent_stays_close_to_the_true_downscale() {
    // A 1.5x ratio has no exact box; the four-tap resolve must still track the scene closely.
    let truth = scene((192, 192));
    let shown = render(&gpu(), 2, &scene((288, 288)), (288, 288), (192, 192));
    let db = psnr(&shown, &truth);
    eprintln!("150% resolve: {db:.2} dB");
    assert!(db > 30., "{db:.2} dB");
}

#[test]
#[ignore = "needs a Vulkan adapter (Mesa's lavapipe is enough)"]
fn edge_upscale_keeps_flat_colour_at_any_ratio() {
    let gpu = gpu();
    let colour = [90u8, 140, 200, 255];
    for (w, h) in [(80, 60), (60, 45), (97, 71)] {
        let shown = render(&gpu, 1, &colour.repeat(40 * 30), (40, 30), (w, h));
        assert!(worst(&shown, &colour.repeat((w * h) as usize)) <= 1, "{w}x{h}");
    }
}

#[test]
#[ignore = "needs a Vulkan adapter (Mesa's lavapipe is enough)"]
fn edge_upscale_beats_bilinear_on_edges() {
    let truth = scene((256, 256));
    let source = downsample(&truth, (256, 256), 2);
    let easu = render(&gpu(), 1, &source, (128, 128), (256, 256));
    let stock = bilinear(&source, (128, 128), (256, 256));
    let (easu_db, stock_db) = (psnr(&easu, &truth), psnr(&stock, &truth));
    // `SKATE_PRESENT_DUMP=DIR` keeps the three images for a look.
    if let Some(dir) = std::env::var_os("SKATE_PRESENT_DUMP") {
        for (name, pixels) in [("truth", &truth), ("bilinear", &stock), ("edge-adaptive", &easu)] {
            image::save_buffer(std::path::Path::new(&dir).join(format!("{name}.png")), pixels, 256, 256, image::ColorType::Rgba8).unwrap();
        }
    }
    eprintln!("2x upscale: edge-adaptive {easu_db:.2} dB, bilinear {stock_db:.2} dB");
    assert!(easu_db > stock_db + 2., "edge-adaptive {easu_db:.2} dB, bilinear {stock_db:.2} dB");
}

#[derive(bevy::prelude::Resource, Default)]
struct ReadBack(std::collections::HashMap<bevy::prelude::Entity, Vec<u8>>);

/// The Bevy side, end to end on a real device: the retail tone node renders with SMAA, then FXAA,
/// and CAS attached and no validation error; the ordering edges exist; and a flat image presented
/// through `PresentMaterial` (the real UI material pipeline and bind group) reads back flat in
/// both filtering modes.
#[test]
#[ignore = "needs a Vulkan adapter (Mesa's lavapipe is enough)"]
fn bevy_runs_the_post_process_chain_and_presents_through_the_material() {
    use super::{PresentMaterial, PresentParams, Present};
    use bevy::{
        anti_alias::{contrast_adaptive_sharpening::ContrastAdaptiveSharpening, fxaa::Fxaa, smaa::{Smaa, SmaaPreset}},
        asset::RenderAssetUsages,
        camera::RenderTarget,
        core_pipeline::{core_3d::graph::{Core3d, Node3d}, tonemapping::Tonemapping},
        prelude::*,
        render::{
            RenderApp, RenderPlugin,
            gpu_readback::{Readback, ReadbackComplete},
            render_graph::{RenderGraph, RenderLabel},
            render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
            settings::{Backends, RenderCreation, WgpuSettings},
            view::Hdr,
        },
        ui_render::prelude::MaterialNode,
        window::ExitCondition,
    };
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, close_when_requested: false, ..default() })
            .set(RenderPlugin {
                render_creation: RenderCreation::Automatic(WgpuSettings { backends: Some(Backends::VULKAN), ..default() }),
                synchronous_pipeline_compilation: true,
                ..default()
            })
            .build()
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::log::LogPlugin>()
            .disable::<bevy::gilrs::GilrsPlugin>()
            .disable::<bevy::audio::AudioPlugin>(),
    )
    .insert_resource(crate::config::Config {
        asset_root: std::env::temp_dir().join("skate-present-test/assets"),
        verification_capture: None,
        map: None,
        map_path: None,
        difficulty: default(),
        check_assets: false,
        validate_maps: false,
        start_paused: false,
        multiplayer: default(),
        map_fingerprint: 0,
        teleport: None,
        mute: false,
    })
    .insert_resource(crate::retail_render::RetailScene(true))
    .init_resource::<ReadBack>()
    .add_message::<crate::map_transition::WorldChanged>()
    .add_plugins((crate::retail_exposure::RetailExposurePlugin, super::GraphicsQualityPlugin));
    app.finish();
    // The ordering edges, read before `cleanup` moves the render app to its own thread.
    {
        let render = app.get_sub_app(RenderApp).unwrap();
        let core = render.world().resource::<RenderGraph>().get_sub_graph(Core3d).unwrap();
        let inputs = |node: Node3d| -> Vec<_> {
            core.get_node_state(node).unwrap().edges.input_edges().iter().map(|e| e.get_output_node()).collect()
        };
        let exposure = crate::retail_exposure::ExposureLabel.intern();
        assert!(inputs(Node3d::Fxaa).contains(&exposure) && inputs(Node3d::Smaa).contains(&exposure));
        assert!(inputs(Node3d::ContrastAdaptiveSharpening).contains(&Node3d::Smaa.intern()));
    }
    app.cleanup();

    let colour = [90u8, 140, 200, 255];
    let world = app.world_mut();
    let mut images = world.resource_mut::<Assets<Image>>();
    let size = |width, height| Extent3d { width, height, depth_or_array_layers: 1 };
    let source = images.add(Image::new(
        size(128, 96),
        TextureDimension::D2,
        colour.repeat(128 * 96),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    ));
    // Render targets the test reads back also need to be copy sources.
    let target = || {
        let mut image = Image::new_target_texture(64, 48, TextureFormat::Rgba8UnormSrgb, None);
        image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
        image
    };
    let (scene, shown) = (target(), target());
    let (scene, shown) = (images.add(scene), images.add(shown));
    // Bevy creates every image's sampler on upload: the device must accept the world pages' 16x one.
    let mut page = Image::new(size(4, 4), TextureDimension::D2, vec![255; 64], TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD);
    page.sampler = crate::retail_render::page_sampler(16);
    images.add(page);
    let params = PresentParams { source: Vec2::new(128., 96.), output: Vec2::new(64., 48.), mode: Present::Resolve.shader_mode() };
    let material = world.resource_mut::<Assets<PresentMaterial>>().add(PresentMaterial { params, source: source.clone() });
    let camera = world
        .spawn((
            Camera3d::default(),
            Camera { clear_color: ClearColorConfig::Custom(Color::srgb(0.3, 0.5, 0.7)), ..default() },
            RenderTarget::Image(scene.clone().into()),
            Hdr,
            Tonemapping::None,
            crate::retail_render::RetailTone,
            Msaa::Off,
            Smaa { preset: SmaaPreset::Ultra },
            ContrastAdaptiveSharpening { enabled: true, sharpening_strength: 0.5, denoise: false },
        ))
        .id();
    let ui = world.spawn((Camera2d, Camera { order: 1, ..default() }, RenderTarget::Image(shown.clone().into()))).id();
    world.spawn((Node { width: percent(100), height: percent(100), ..default() }, MaterialNode(material.clone()), UiTargetCamera(ui)));
    let read = |world: &mut World, image: Handle<Image>| {
        world
            .spawn(Readback::texture(image))
            .observe(|done: On<ReadbackComplete>, mut store: ResMut<ReadBack>| {
                store.0.insert(done.entity, done.data.clone());
            })
            .id()
    };
    let shown_read = read(world, shown);
    let scene_read = read(world, scene);
    let frames = |app: &mut App| {
        for _ in 0..8 {
            app.update();
        }
    };
    let flat = |data: &[u8]| data.chunks(4).all(|p| p == &data[..4]);

    frames(&mut app);
    let store = app.world().resource::<ReadBack>();
    let (presented, rendered) = (store.0[&shown_read].clone(), store.0[&scene_read].clone());
    assert!(worst(&presented, &colour.repeat(64 * 48)) <= 1, "resolve: {:?}", &presented[..8]);
    // A flat clear colour through the retail tone, SMAA and CAS stays flat and is not black.
    assert!(flat(&rendered) && rendered[..3] != [0, 0, 0], "{:?}", &rendered[..8]);

    // The other anti-aliasing pass and the other filtering mode.
    app.world_mut().entity_mut(camera).remove::<Smaa>().insert(Fxaa::default());
    app.world_mut().resource_mut::<Assets<PresentMaterial>>().get_mut(&material).unwrap().params.mode = Present::Edge.shader_mode();
    frames(&mut app);
    let store = app.world().resource::<ReadBack>();
    assert!(worst(&store.0[&shown_read], &colour.repeat(64 * 48)) <= 1, "edge: {:?}", &store.0[&shown_read][..8]);
    assert_eq!(store.0[&scene_read], rendered, "FXAA changed a flat image");

}

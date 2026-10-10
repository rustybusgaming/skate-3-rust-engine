//! Optional image quality over the retail renderer, all off by default.
//!
//! - A render scale above 100% supersamples: the scene target is box-filtered down to the window
//!   (`graphics_present.wgsl`). Below 100%, the edge-adaptive upscaler (FSR 1 EASU) can replace
//!   the bilinear stretch.
//! - Contrast-adaptive sharpening (AMD CAS, Bevy's node) on the 3D cameras, at render scale.
//! - FXAA or SMAA on the 3D cameras. MSAA stays off (`render_capacity`): these cost per pixel, not
//!   per draw. TAA is not offered because the retail prepass shaders write no motion vectors.
//! - Anisotropic filtering of the world texture pages, applied when a map is built.
//!
//! The defaults reproduce the existing image exactly. At 100% (or bilinear below it) the scene
//! target is still presented by the stock `ImageNode`, no post-process component is attached, and
//! texture samplers keep an anisotropy of 1.
use bevy::{
    anti_alias::{
        contrast_adaptive_sharpening::ContrastAdaptiveSharpening,
        fxaa::Fxaa,
        smaa::{Smaa, SmaaPreset},
    },
    asset::{AssetPath, embedded_asset, embedded_path},
    camera::RenderTarget,
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
    ui_render::prelude::{MaterialNode, UiMaterial, UiMaterialPlugin},
    window::PrimaryWindow,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Presentation of a scaled scene below 100%.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Upscaler {
    /// The stock bilinear stretch.
    #[default]
    Bilinear,
    /// FSR 1 EASU: edge-directed, deringed Lanczos-like reconstruction.
    Edge,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AntiAliasing {
    #[default]
    Off,
    Fxaa,
    /// SMAA at Bevy's default (High) preset.
    Smaa,
    SmaaUltra,
}

impl AntiAliasing {
    const ALL: [Self; 4] = [Self::Off, Self::Fxaa, Self::Smaa, Self::SmaaUltra];

    fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Fxaa => "FXAA",
            Self::Smaa => "SMAA",
            Self::SmaaUltra => "SMAA Ultra",
        }
    }
}

/// Contrast-adaptive sharpening strengths, in percent (0 = off).
const SHARPENING: &[u32] = &[0, 25, 50, 75, 100];
/// Anisotropic filtering levels (1 = off; 16 is wgpu's ceiling).
const ANISOTROPY: &[u16] = &[1, 2, 4, 8, 16];

/// Saved beside the other display settings in `settings/graphics.json` (flattened into them).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Quality {
    pub upscaler: Upscaler,
    pub sharpening: u32,
    pub anti_aliasing: AntiAliasing,
    pub anisotropy: u16,
}

impl Default for Quality {
    fn default() -> Self {
        Self { upscaler: Upscaler::Bilinear, sharpening: 0, anti_aliasing: AntiAliasing::Off, anisotropy: 1 }
    }
}

impl Quality {
    pub(crate) fn validated(mut self) -> Self {
        if !SHARPENING.contains(&self.sharpening) {
            self.sharpening = 0;
        }
        if !ANISOTROPY.contains(&self.anisotropy) {
            self.anisotropy = 1;
        }
        self
    }
}

/// The menu rows for [`Quality`], one per field, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Row {
    Upscaler,
    Sharpening,
    AntiAliasing,
    Anisotropy,
}

impl Row {
    pub(crate) fn adjust(self, quality: &mut Quality, direction: i32) {
        match self {
            Row::Upscaler => {
                quality.upscaler = crate::graphics_menu::cycle(&[Upscaler::Bilinear, Upscaler::Edge], quality.upscaler, direction);
            }
            Row::Sharpening => quality.sharpening = crate::graphics_menu::cycle(SHARPENING, quality.sharpening, direction),
            Row::AntiAliasing => {
                quality.anti_aliasing = crate::graphics_menu::cycle(&AntiAliasing::ALL, quality.anti_aliasing, direction);
            }
            Row::Anisotropy => quality.anisotropy = crate::graphics_menu::cycle(ANISOTROPY, quality.anisotropy, direction),
        }
    }

    pub(crate) fn label(self, quality: &Quality, scale: u32) -> String {
        match self {
            Row::Upscaler => format!(
                "Upscaling             {}{}",
                match quality.upscaler {
                    Upscaler::Bilinear => "Bilinear",
                    Upscaler::Edge => "Edge-adaptive (FSR 1)",
                },
                match scale {
                    100 => "  (unused at 100%)",
                    s if s > 100 => "  (supersampling)",
                    _ => "",
                }
            ),
            Row::Sharpening => match quality.sharpening {
                0 => "Sharpening            Off".into(),
                s => format!("Sharpening            {s}%"),
            },
            Row::AntiAliasing => format!("Anti-aliasing         {}", quality.anti_aliasing.label()),
            Row::Anisotropy => match quality.anisotropy {
                1 => "Texture filtering     Trilinear".into(),
                a => format!("Texture filtering     {a}x anisotropic"),
            },
        }
    }

    pub(crate) fn description(self) -> &'static str {
        match self {
            Row::Upscaler => "Below 100% internal resolution: bilinear stretch, or an edge-adaptive upscale (FSR 1) that keeps edges sharp.",
            Row::Sharpening => "Contrast-adaptive sharpening (AMD CAS). Restores detail lost to upscaling or anti-aliasing.",
            Row::AntiAliasing => "Smooths jagged edges after the image is drawn. Above 100% internal resolution, supersampling already does this.",
            Row::Anisotropy => "Keeps ground and walls sharp at grazing angles. Applies the next time a map loads.",
        }
    }
}

/// `settings/graphics.json` beside the installation's `assets/`, as the graphics menu saves it.
pub(crate) fn settings_path(asset_root: &Path) -> PathBuf {
    asset_root.parent().unwrap_or(asset_root).join("settings/graphics.json")
}

/// The saved quality settings, read where the menu is not available (the map build worker).
/// Missing or unreadable settings are the defaults, exactly as the menu treats them.
pub(crate) fn saved(asset_root: &Path) -> Quality {
    std::fs::read(settings_path(asset_root))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Quality>(&bytes).ok())
        .unwrap_or_default()
        .validated()
}

/// How the scaled scene target reaches the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Present {
    /// The stock `ImageNode` (bilinear), used at 100% and for bilinear upscaling.
    Stock,
    Edge,
    Resolve,
}

impl Present {
    pub(crate) fn choose(scale: u32, quality: &Quality) -> Self {
        match scale {
            s if s > 100 => Present::Resolve,
            s if s < 100 && quality.upscaler == Upscaler::Edge => Present::Edge,
            _ => Present::Stock,
        }
    }

    fn shader_mode(self) -> u32 {
        match self {
            Present::Stock => 0,
            Present::Edge => 1,
            Present::Resolve => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
struct PresentParams {
    source: Vec2,
    output: Vec2,
    mode: u32,
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub(crate) struct PresentMaterial {
    #[uniform(0)]
    params: PresentParams,
    #[texture(1)]
    #[sampler(2)]
    source: Handle<Image>,
}

impl UiMaterial for PresentMaterial {
    fn fragment_shader() -> ShaderRef {
        AssetPath::from(embedded_path!("graphics_present.wgsl")).with_source("embedded").into()
    }
}

/// The full-window node that shows the scene target.
#[derive(Component)]
pub(crate) struct Presenter;

pub(crate) struct GraphicsQualityPlugin;

impl Plugin for GraphicsQualityPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "graphics_present.wgsl");
        app.add_plugins(UiMaterialPlugin::<PresentMaterial>::default())
            .add_systems(Update, post_process);
    }
}

/// Switches the [`Presenter`] between the stock `ImageNode` and the filtering material, and keeps
/// the material's sizes current. Runs after the menu has resized the scene target.
pub(crate) fn present(
    mut commands: Commands,
    menu: Res<crate::graphics_menu::Menu>,
    target: Res<crate::graphics_menu::SceneTarget>,
    window: Single<&Window, With<PrimaryWindow>>,
    images: Res<Assets<Image>>,
    mut materials: ResMut<Assets<PresentMaterial>>,
    presenters: Query<(Entity, Option<&MaterialNode<PresentMaterial>>), With<Presenter>>,
) {
    let mode = Present::choose(menu.scale(), &menu.quality());
    let Some(source) = images.get(&target.0).map(|image| image.size().as_vec2()) else {
        return;
    };
    let params = PresentParams { source, output: window.physical_size().max(UVec2::ONE).as_vec2(), mode: mode.shader_mode() };
    for (entity, material) in &presenters {
        match (mode, material) {
            (Present::Stock, None) => {}
            (Present::Stock, Some(_)) => {
                commands
                    .entity(entity)
                    .remove::<MaterialNode<PresentMaterial>>()
                    .insert(ImageNode::new(target.0.clone()));
            }
            (_, Some(handle)) => {
                if materials.get(&handle.0).is_some_and(|m| m.params != params) {
                    materials.get_mut(&handle.0).unwrap().params = params;
                }
            }
            (_, None) => {
                let handle = materials.add(PresentMaterial { params, source: target.0.clone() });
                commands.entity(entity).remove::<ImageNode>().insert(MaterialNode(handle));
            }
        }
    }
}

/// Keeps every 3D camera that draws the scene target on the chosen anti-aliasing and sharpening.
/// Runs every frame, like `render_capacity`, because cameras are respawned across map transitions.
#[allow(clippy::type_complexity)]
fn post_process(
    mut commands: Commands,
    menu: Option<Res<crate::graphics_menu::Menu>>,
    target: Option<Res<crate::graphics_menu::SceneTarget>>,
    cameras: Query<
        (Entity, &RenderTarget, Has<Fxaa>, Option<&Smaa>, Option<&ContrastAdaptiveSharpening>),
        With<Camera3d>,
    >,
) {
    let (Some(menu), Some(target)) = (menu, target) else {
        return;
    };
    let quality = menu.quality();
    let smaa = match quality.anti_aliasing {
        AntiAliasing::Smaa => Some(SmaaPreset::High),
        AntiAliasing::SmaaUltra => Some(SmaaPreset::Ultra),
        _ => None,
    };
    let sharpening = (quality.sharpening > 0).then(|| quality.sharpening as f32 / 100.);
    for (entity, render_target, fxaa, current_smaa, cas) in &cameras {
        if !matches!(render_target, RenderTarget::Image(image) if image.handle == target.0) {
            continue;
        }
        let mut camera = commands.entity(entity);
        match (quality.anti_aliasing == AntiAliasing::Fxaa, fxaa) {
            (true, false) => {
                camera.insert(Fxaa::default());
            }
            (false, true) => {
                camera.remove::<Fxaa>();
            }
            _ => {}
        }
        if current_smaa.map(|s| s.preset) != smaa {
            match smaa {
                Some(preset) => camera.insert(Smaa { preset }),
                None => camera.remove::<Smaa>(),
            };
        }
        if cas.filter(|c| c.enabled).map(|c| c.sharpening_strength) != sharpening {
            match sharpening {
                Some(strength) => camera.insert(ContrastAdaptiveSharpening { enabled: true, sharpening_strength: strength, denoise: false }),
                None => camera.remove::<ContrastAdaptiveSharpening>(),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_stock_image_and_old_files_load() {
        assert_eq!(Present::choose(100, &Quality::default()), Present::Stock);
        assert_eq!(Present::choose(67, &Quality::default()), Present::Stock);
        let old: Quality = serde_json::from_str(r#"{"width":1920,"height":1080,"scale":75,"fps":0}"#).unwrap();
        assert_eq!(old, Quality::default());
    }

    #[test]
    fn presentation_follows_scale_and_upscaler() {
        let edge = Quality { upscaler: Upscaler::Edge, ..default() };
        assert_eq!(Present::choose(50, &edge), Present::Edge);
        assert_eq!(Present::choose(100, &edge), Present::Stock);
        assert_eq!(Present::choose(150, &edge), Present::Resolve);
        assert_eq!(Present::choose(125, &Quality::default()), Present::Resolve);
    }

    #[test]
    fn the_map_worker_reads_what_the_menu_saved() {
        let root = std::env::temp_dir().join(format!("skate-quality-{}", std::process::id()));
        let assets = root.join("assets");
        assert_eq!(saved(&assets), Quality::default());
        std::fs::create_dir_all(root.join("settings")).unwrap();
        std::fs::write(settings_path(&assets), r#"{"width":1920,"height":1080,"scale":100,"fps":0,"anisotropy":8}"#).unwrap();
        assert_eq!(saved(&assets).anisotropy, 8);
        std::fs::write(settings_path(&assets), r#"{"anisotropy":5}"#).unwrap();
        assert_eq!(saved(&assets).anisotropy, 1);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn invalid_values_fall_back_and_rows_cycle() {
        let saved: Quality = serde_json::from_str(r#"{"sharpening":33,"anisotropy":3,"anti_aliasing":"smaa_ultra"}"#).unwrap();
        let saved = saved.validated();
        assert_eq!((saved.sharpening, saved.anisotropy, saved.anti_aliasing), (0, 1, AntiAliasing::SmaaUltra));
        let mut q = Quality::default();
        Row::Anisotropy.adjust(&mut q, -1);
        assert_eq!(q.anisotropy, 16);
        Row::AntiAliasing.adjust(&mut q, 1);
        assert_eq!(q.anti_aliasing, AntiAliasing::Fxaa);
        Row::Upscaler.adjust(&mut q, 1);
        assert_eq!(q.upscaler, Upscaler::Edge);
        let round: Quality = serde_json::from_slice(&serde_json::to_vec(&q).unwrap()).unwrap();
        assert_eq!(round, q);
    }
}

#[cfg(test)]
#[path = "graphics_present_tests.rs"]
mod present_tests;

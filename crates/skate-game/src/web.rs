//! The browser build's process-level differences (docs/web-port). A browser tab has no updater,
//! crash supervisor, setup tool modes or trace file, and no installation folder beside an exe:
//! the player's own converted installation is chosen in the page instead (web port milestone 3).
//! Until it can be, startup stops at the configuration and says why, on the WebGPU canvas, so the
//! window, renderer and logging are proven in a real browser.
use bevy::{
    prelude::*,
    render::{
        RenderPlugin,
        renderer::RenderAdapterInfo,
        settings::{Backends, RenderCreation, WgpuSettings},
    },
};

/// The canvas the loader page provides (`web/index.html`).
pub(crate) const CANVAS: &str = "#skate3";

/// WebGPU only: the renderer's storage-buffer and compute use has no WebGL2 fallback.
pub(crate) const BACKENDS: Backends = Backends::BROWSER_WEBGPU;

pub(crate) fn window(title: String) -> Window {
    Window {
        title,
        canvas: Some(CANVAS.into()),
        fit_canvas_to_parent: true,
        // Keys such as Tab and the arrows drive the menu, not the page.
        prevent_default_event_handling: true,
        ..default()
    }
}

#[derive(Resource)]
struct Stopped(String);

/// Shows why startup stopped (and logs it as `SKATE_WEB_STOPPED`), then idles. Logs
/// `SKATE_WEB_READY` once a WebGPU adapter is up; `tools/web-port/smoke.mjs` waits for both.
pub(crate) fn stopped(message: String) -> AppExit {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin { primary_window: Some(window("Skate 3 Rust Engine".into())), ..default() })
                .set(RenderPlugin {
                    render_creation: RenderCreation::Automatic(WgpuSettings { backends: Some(BACKENDS), ..default() }),
                    ..default()
                })
                .disable::<bevy::gilrs::GilrsPlugin>(),
        )
        .insert_resource(Stopped(message))
        .add_systems(Startup, show)
        .add_systems(Update, announce)
        .run()
}

fn show(mut commands: Commands, stopped: Res<Stopped>) {
    info!("SKATE_WEB_STOPPED {}", stopped.0);
    commands.spawn(Camera2d);
    commands
        .spawn(Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            row_gap: px(16),
            ..default()
        })
        .with_children(|page| {
            page.spawn((Text::new("SKATE / 3"), TextFont { font_size: 48., ..default() }, TextColor(Color::srgb(0.78, 0.96, 0.3))));
            page.spawn((Text::new(stopped.0.clone()), TextFont { font_size: 18., ..default() }, TextColor(Color::WHITE)));
        });
}

fn announce(info: Option<Res<RenderAdapterInfo>>, mut done: Local<bool>) {
    if let Some(info) = info.filter(|_| !*done) {
        info!("SKATE_WEB_READY adapter={:?} backend={:?}", info.name, info.backend);
        *done = true;
    }
}

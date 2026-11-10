//! Emergence Lab desktop application.
//!
//! Builds the Bevy app: window, asset folder, the application shell
//! ([`interface::InterfacePlugin`]), the shared world-input resource and the
//! laboratories registered in [`register_modes`].

mod assets;
mod cli;
mod smoke;

use std::process::ExitCode;

use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution, WindowTheme};
use interface::InterfacePlugin;
use rendering::WorldInputPlugin;

/// Registers the laboratories (view plugins). Each view crate adds one line
/// here, e.g. `app.add_plugins(view_conway::ConwayViewPlugin);`. A view
/// plugin calls `interface::ModeAppExt::register_mode` and follows the
/// contract in `docs/UI_SHELL.md`. Modes that are not registered appear on
/// the dashboard as "in development".
// ---- BEGIN MODE REGISTRATION ------------------------------------------
fn register_modes(app: &mut App) {
    app.add_plugins((
        view_conway::ConwayViewPlugin,
        view_ecosystem::EcosystemViewPlugin,
        view_custom_life::CustomLifeViewPlugin,
        view_metapixel::MetapixelViewPlugin,
        view_ca_lab::CaLabViewPlugin,
        view_evolution::EvolutionViewPlugin,
        view_continuous::ContinuousViewPlugin,
        view_voxel::VoxelViewPlugin,
        view_civilization::CivilizationViewPlugin,
    ));
}
// ---- END MODE REGISTRATION --------------------------------------------

fn main() -> ExitCode {
    let options = match cli::parse(std::env::args().skip(1)) {
        Ok(cli::Command::Run(options)) => options,
        Ok(cli::Command::Help) => {
            print!("{}", cli::USAGE);
            println!("MODES:\n    {}", interface::AppMode::ALL.map(|m| m.slug()).join(", "));
            return ExitCode::SUCCESS;
        }
        Ok(cli::Command::Version) => {
            println!("emergence-lab {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let (asset_root, source) = match assets::resolve_for_process() {
        Ok(found) => found,
        Err(e) => {
            eprintln!("error: {e}\nSet {} to the folder that contains `patterns/` and `models/`.", assets::ASSETS_ENV);
            return ExitCode::from(2);
        }
    };

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Emergence Lab".into(),
                    resolution: WindowResolution::new(1600, 1000),
                    present_mode: PresentMode::AutoVsync,
                    window_theme: Some(WindowTheme::Dark),
                    ..default()
                }),
                ..default()
            })
            .set(AssetPlugin { file_path: asset_root.to_string_lossy().into_owned(), ..default() }),
    );
    info!("assets: {} ({source:?})", asset_root.display());
    app.add_plugins((WorldInputPlugin, InterfacePlugin { asset_root, initial_mode: options.mode }));
    register_modes(&mut app);
    if let Some(capture) = options.smoke {
        app.add_plugins(smoke::SmokeScreenshotPlugin(capture));
    }
    match app.run() {
        AppExit::Success => ExitCode::SUCCESS,
        AppExit::Error(code) => ExitCode::from(code.get()),
    }
}

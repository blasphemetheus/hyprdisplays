use anyhow::Result;
use clap::Parser;
use hyprdisplays::ui::app::App;
use hyprdisplays::ui::theme;
use hyprdisplays::{audio, hypr, lua_writer, model::Layout, profiles};

/// Display manager for Hyprland: arrange, modes, mirror, audio, profiles.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    /// List saved profiles and exit
    #[arg(long)]
    list_profiles: bool,
    /// Apply a saved profile (monitor rules + audio sink) and exit
    #[arg(long, value_name = "NAME")]
    apply: Option<String>,
    /// Save the CURRENT live layout + default sink as a profile and exit
    #[arg(long, value_name = "NAME")]
    save_profile: Option<String>,
    /// Write the current live layout into lua/monitors.lua and exit
    #[arg(long)]
    write_lua: bool,
    /// List audio outputs (sinks + HDMI ports with the display on each) and exit
    #[arg(long)]
    outputs: bool,
    /// Route audio to the HDMI port of a monitor (Hyprland name like HDMI-A-1,
    /// or any part of its description / ELD name like "TOSHIBA") and exit
    #[arg(long, value_name = "MONITOR")]
    audio_to: Option<String>,
}

fn app_theme(_app: &App) -> iced::Theme {
    theme::theme()
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.list_profiles {
        for p in profiles::list() { println!("{p}"); }
        return Ok(());
    }
    if let Some(name) = cli.apply {
        println!("{}", profiles::apply(&profiles::load(&name)?)?);
        return Ok(());
    }
    if let Some(name) = cli.save_profile {
        let p = profiles::Profile { name: name.clone(), layout: Layout::from_live(&hypr::monitors()?), sink: audio::default_sink().ok() };
        profiles::save(&p)?;
        println!("saved profile '{name}'");
        return Ok(());
    }
    if cli.outputs {
        let snap = audio::snapshot()?;
        let cur = snap.default_output();
        for o in snap.outputs() {
            println!("{} {}  [{}]", if Some(&o) == cur.as_ref() { "*" } else { " " }, o.label(), o.sink_name());
        }
        return Ok(());
    }
    if let Some(mon) = cli.audio_to {
        // Accept a Hyprland output name (HDMI-A-1) by resolving it to its description.
        let desc = hypr::monitors().ok().and_then(|ms| ms.into_iter().find(|m| m.name == mon).map(|m| m.description)).unwrap_or(mon);
        println!("audio → {}", audio::route_to_monitor(&desc)?);
        return Ok(());
    }
    if cli.write_lua {
        let path = lua_writer::default_path();
        lua_writer::write(&path, &Layout::from_live(&hypr::monitors()?))?;
        println!("wrote {}", path.display());
        return Ok(());
    }

    iced::application(App::new, App::update, App::view)
        .title(App::title)
        .subscription(App::subscription)
        .theme(app_theme)
        .window(iced::window::Settings {
            size: iced::Size::new(1200.0, 720.0),
            min_size: Some(iced::Size::new(900.0, 560.0)),
            platform_specific: iced::window::settings::PlatformSpecific { application_id: "hyprdisplays".into(), ..Default::default() },
            ..Default::default()
        })
        .antialiasing(true)
        .run()
        .map_err(|e| anyhow::anyhow!("{e}"))
}

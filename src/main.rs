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

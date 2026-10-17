//! hyprbinds: keybind editor for the dotfiles' Hyprland Lua config
//! (~/.config/hypr/lua/binds.lua). Same crate/theme as hyprdisplays.

use anyhow::Result;
use clap::Parser;
use hyprdisplays::binds::{self, BindsFile};
use hyprdisplays::ui::binds::BindsApp;
use hyprdisplays::ui::theme;

/// Show and edit Hyprland keybinds (lua/binds.lua).
#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    /// Print every bind as "keys\taction" and exit (for rofi/grep)
    #[arg(long)]
    list: bool,
    /// Path to binds.lua (default: ~/.config/hypr/lua/binds.lua)
    #[arg(long, value_name = "FILE")]
    file: Option<std::path::PathBuf>,
}

fn app_theme(_app: &BindsApp) -> iced::Theme {
    theme::theme()
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let path = cli.file.unwrap_or_else(binds::default_path);
    if cli.list {
        let f = BindsFile::load(&path)?;
        for b in &f.binds {
            let flags = b.flags.summary();
            println!("{}\t{}{}", b.keys, b.describe(), if flags.is_empty() { String::new() } else { format!("  [{flags}]") });
        }
        return Ok(());
    }

    iced::application(move || BindsApp::new(path.clone()), BindsApp::update, BindsApp::view)
        .title(BindsApp::title)
        .subscription(BindsApp::subscription)
        .theme(app_theme)
        .window(iced::window::Settings {
            size: iced::Size::new(1180.0, 760.0),
            min_size: Some(iced::Size::new(860.0, 520.0)),
            platform_specific: iced::window::settings::PlatformSpecific { application_id: "hyprbinds".into(), ..Default::default() },
            ..Default::default()
        })
        .antialiasing(true)
        .run()
        .map_err(|e| anyhow::anyhow!("{e}"))
}

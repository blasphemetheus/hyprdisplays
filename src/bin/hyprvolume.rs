//! hyprvolume: PipeWire mixer (outputs incl. every HDMI port by monitor, app
//! streams, inputs, card profiles). Same crate/theme as hyprdisplays.

use anyhow::Result;
use hyprdisplays::ui::theme;
use hyprdisplays::ui::volume::VolApp;

fn app_theme(_app: &VolApp) -> iced::Theme {
    theme::theme()
}

fn main() -> Result<()> {
    iced::application(VolApp::new, VolApp::update, VolApp::view)
        .title(VolApp::title)
        .subscription(VolApp::subscription)
        .theme(app_theme)
        .window(iced::window::Settings {
            size: iced::Size::new(760.0, 680.0),
            min_size: Some(iced::Size::new(560.0, 400.0)),
            platform_specific: iced::window::settings::PlatformSpecific { application_id: "hyprvolume".into(), ..Default::default() },
            ..Default::default()
        })
        .antialiasing(true)
        .run()
        .map_err(|e| anyhow::anyhow!("{e}"))
}

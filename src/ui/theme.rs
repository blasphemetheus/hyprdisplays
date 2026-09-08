//! "Gold to Rose" — the palette shared with waybar/hyprlock in the dotfiles.

use iced::widget::{button, container};
use iced::{color, gradient, Background, Border, Color, Radians, Shadow, Theme, Vector};

pub const BG: Color = color!(0x1e1c23);
pub const PANEL: Color = Color { r: 30.0 / 255.0, g: 28.0 / 255.0, b: 35.0 / 255.0, a: 0.95 };
pub const GOLD: Color = color!(0xe8a854);
pub const BRIGHT_GOLD: Color = color!(0xf5a623);
pub const ROSE: Color = color!(0xe875a1);
pub const DUSTY_ROSE: Color = color!(0xd4848c);
pub const DIM: Color = color!(0x6b8cae);
pub const CREAM: Color = color!(0xe8e0d6);
pub const FAIL: Color = color!(0xe85a5a);
pub const OK: Color = color!(0xa8c686);

pub fn theme() -> Theme {
    Theme::custom(
        "GoldRose".to_string(),
        iced::theme::Palette {
            background: BG,
            text: CREAM,
            primary: GOLD,
            success: OK,
            warning: BRIGHT_GOLD,
            danger: FAIL,
        },
    )
}

fn with_alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

/// Rounded panel with the waybar container look.
pub fn card(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(CREAM),
        background: Some(Background::Color(PANEL)),
        border: Border { color: with_alpha(GOLD, 0.35), width: 1.0, radius: 15.0.into() },
        shadow: Shadow { color: with_alpha(Color::BLACK, 0.3), offset: Vector::new(0.0, 4.0), blur_radius: 16.0 },
        snap: false,
    }
}

/// Inner module: steel-blue at 30%, radius 10.
pub fn module(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(CREAM),
        background: Some(Background::Color(with_alpha(DIM, 0.3))),
        border: Border { color: Color::TRANSPARENT, width: 0.0, radius: 10.0.into() },
        shadow: Shadow::default(),
        snap: false,
    }
}

/// Gold → rose gradient button (the waybar active-workspace look).
pub fn primary(_theme: &Theme, status: button::Status) -> button::Style {
    let (a, b) = match status {
        button::Status::Hovered | button::Status::Pressed => (BRIGHT_GOLD, ROSE),
        button::Status::Disabled => (with_alpha(GOLD, 0.35), with_alpha(ROSE, 0.35)),
        button::Status::Active => (GOLD, DUSTY_ROSE),
    };
    let g = gradient::Linear::new(Radians(std::f32::consts::FRAC_PI_4)).add_stop(0.0, a).add_stop(1.0, b);
    button::Style {
        background: Some(Background::Gradient(g.into())),
        text_color: BG,
        border: Border { color: Color::TRANSPARENT, width: 0.0, radius: 10.0.into() },
        shadow: Shadow::default(),
        snap: false,
    }
}

/// Quiet button: dim steel background, cream text; gold on hover.
pub fn quiet(_theme: &Theme, status: button::Status) -> button::Style {
    let (bg, fg) = match status {
        button::Status::Hovered | button::Status::Pressed => (with_alpha(GOLD, 0.25), GOLD),
        button::Status::Disabled => (with_alpha(DIM, 0.15), with_alpha(CREAM, 0.4)),
        button::Status::Active => (with_alpha(DIM, 0.3), CREAM),
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: fg,
        border: Border { color: Color::TRANSPARENT, width: 0.0, radius: 10.0.into() },
        shadow: Shadow::default(),
        snap: false,
    }
}

/// Destructive: rose/fail.
pub fn danger(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => FAIL,
        button::Status::Disabled => with_alpha(FAIL, 0.3),
        button::Status::Active => with_alpha(FAIL, 0.7),
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: BG,
        border: Border { color: Color::TRANSPARENT, width: 0.0, radius: 10.0.into() },
        shadow: Shadow::default(),
        snap: false,
    }
}

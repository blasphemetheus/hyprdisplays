//! Hyprland side: read `hyprctl monitors all -j`, write `hyprctl eval "hl.monitor{…}"`.
//! Hyprland ≥0.55 (Lua config): `hyprctl keyword` is gone, dispatchers are Lua
//! expressions. Everything here is a thin, blocking wrapper around `hyprctl`.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::process::Command;

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct Monitor {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    #[serde(rename = "refreshRate")]
    pub refresh_rate: f64,
    pub scale: f64,
    #[serde(default)]
    pub vrr: bool,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub focused: bool,
    /// "none", or the id of the mirrored monitor (Hyprland prints a number).
    #[serde(rename = "mirrorOf", default)]
    pub mirror_of: serde_json::Value,
    #[serde(rename = "availableModes", default)]
    pub available_modes: Vec<String>,
}

impl Monitor {
    /// Id of the monitor this one mirrors, if any.
    pub fn mirror_of_id(&self) -> Option<i64> {
        match &self.mirror_of {
            serde_json::Value::Number(n) => n.as_i64(),
            serde_json::Value::String(s) => s.parse().ok(),
            _ => None,
        }
    }
    pub fn current_mode(&self) -> Mode {
        Mode { width: self.width, height: self.height, refresh: self.refresh_rate }
    }
    /// Available modes parsed, de-duplicated, highest refresh first per resolution.
    pub fn modes(&self) -> Vec<Mode> {
        let mut v: Vec<Mode> = self.available_modes.iter().filter_map(|s| Mode::parse(s)).collect();
        v.sort_by(|a, b| {
            (b.width * b.height).cmp(&(a.width * a.height)).then(b.refresh.partial_cmp(&a.refresh).unwrap())
        });
        v.dedup_by(|a, b| a.to_string() == b.to_string());
        v
    }
}

/// A display mode. `Display` renders the form Hyprland accepts: `1920x1080@165`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    pub refresh: f64,
}

impl Mode {
    /// Parses `1920x1080@164.99Hz`, `1920x1080@60`, `1920x1080`.
    pub fn parse(s: &str) -> Option<Mode> {
        let s = s.trim().trim_end_matches("Hz");
        let (res, hz) = match s.split_once('@') {
            Some((r, h)) => (r, h.parse::<f64>().ok()?),
            None => (s, 0.0),
        };
        let (w, h) = res.split_once('x')?;
        Some(Mode { width: w.parse().ok()?, height: h.parse().ok()?, refresh: hz })
    }
    /// Closest available mode to a wanted refresh at the same resolution.
    pub fn closest<'a>(&self, avail: &'a [Mode]) -> Option<&'a Mode> {
        avail
            .iter()
            .filter(|m| m.width == self.width && m.height == self.height)
            .min_by(|a, b| (a.refresh - self.refresh).abs().partial_cmp(&(b.refresh - self.refresh).abs()).unwrap())
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 60.00 → 60, 59.94 → 59.94, 164.99 → 164.99
        let hz = format!("{:.2}", self.refresh);
        let hz = hz.trim_end_matches('0').trim_end_matches('.');
        write!(f, "{}x{}@{}", self.width, self.height, hz)
    }
}

/// What we push back into Hyprland for one output (one `hl.monitor{}` rule).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MonitorCfg {
    /// Connector name (HDMI-A-1). Informational; rules match on description.
    pub name: String,
    /// EDID description — stable across replugs, used as `output = "desc:…"`.
    pub description: String,
    pub mode: String,
    pub x: i32,
    pub y: i32,
    pub scale: f64,
    pub vrr: bool,
    /// Name of the monitor to mirror, or None.
    pub mirror: Option<String>,
    pub enabled: bool,
    #[serde(default)]
    pub available_modes: Vec<String>,
}

impl MonitorCfg {
    pub fn output(&self) -> String {
        if self.description.is_empty() { self.name.clone() } else { format!("desc:{}", self.description) }
    }
    /// The Lua `hl.monitor{…}` call. `mirror` is ALWAYS explicit: omitting it
    /// keeps whatever mirror the monitor already had.
    pub fn lua(&self) -> String {
        let esc = |s: &str| s.replace('\\', "\\\\").replace('\'', "\\'");
        format!(
            "hl.monitor({{ output = '{}', mode = '{}', position = '{}x{}', scale = {}, vrr = {}, mirror = '{}', disabled = {} }})",
            esc(&self.output()),
            esc(&self.mode),
            self.x,
            self.y,
            fmt_scale(self.scale),
            if self.vrr { 1 } else { 0 },
            esc(self.mirror.as_deref().unwrap_or("none")),
            !self.enabled
        )
    }
    pub fn mode_parsed(&self) -> Option<Mode> {
        Mode::parse(&self.mode)
    }
    pub fn size(&self) -> (u32, u32) {
        self.mode_parsed().map(|m| (m.width, m.height)).unwrap_or((1920, 1080))
    }
}

pub fn fmt_scale(s: f64) -> String {
    let t = format!("{s:.2}");
    t.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn hyprctl(args: &[&str]) -> Result<String> {
    let out = Command::new("hyprctl").args(args).output().context("running hyprctl")?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() {
        return Err(anyhow!("hyprctl {} failed: {}{}", args.join(" "), stdout, String::from_utf8_lossy(&out.stderr)));
    }
    Ok(stdout)
}

pub fn monitors() -> Result<Vec<Monitor>> {
    let json = hyprctl(&["monitors", "all", "-j"])?;
    parse_monitors(&json)
}

pub fn parse_monitors(json: &str) -> Result<Vec<Monitor>> {
    serde_json::from_str(json).context("parsing hyprctl monitors JSON")
}

/// `hyprctl eval <lua>`; Hyprland answers "ok" or an error text.
pub fn eval(lua: &str) -> Result<()> {
    let r = hyprctl(&["eval", lua])?;
    if r.starts_with("ok") || r.is_empty() { Ok(()) } else { Err(anyhow!("{r}")) }
}

pub fn apply(cfg: &MonitorCfg) -> Result<()> {
    eval(&cfg.lua())
}

pub fn dpms(on: bool) -> Result<()> {
    let r = hyprctl(&["dispatch", &format!("hl.dsp.dpms({{ action = \"{}\" }})", if on { "on" } else { "off" })])?;
    if r.starts_with("ok") { Ok(()) } else { Err(anyhow!("{r}")) }
}

pub fn reload() -> Result<()> {
    hyprctl(&["reload"]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIX: &str = include_str!("../tests/fixtures/monitors_all.json");

    #[test]
    fn parses_fixture() {
        let m = parse_monitors(FIX).unwrap();
        assert_eq!(m.len(), 2);
        let hdmi = m.iter().find(|m| m.name == "HDMI-A-1").unwrap();
        assert!(hdmi.description.contains("VG27V"));
        assert!(hdmi.modes().len() > 10);
        assert_eq!(hdmi.mirror_of_id(), None);
    }

    #[test]
    fn mode_roundtrip() {
        assert_eq!(Mode::parse("1920x1080@164.99Hz").unwrap().to_string(), "1920x1080@164.99");
        assert_eq!(Mode::parse("1920x1080@60.00Hz").unwrap().to_string(), "1920x1080@60");
        assert_eq!(Mode::parse("800x600@60.32Hz").unwrap().to_string(), "800x600@60.32");
        assert!(Mode::parse("garbage").is_none());
    }

    #[test]
    fn lua_rule_is_explicit_about_mirror() {
        let c = MonitorCfg {
            name: "HDMI-A-1".into(),
            description: "Toshiba TV 0x1".into(),
            mode: "1920x1080@60".into(),
            x: 3840, y: 0, scale: 1.0, vrr: false, mirror: None, enabled: true,
            available_modes: vec![],
        };
        assert_eq!(
            c.lua(),
            "hl.monitor({ output = 'desc:Toshiba TV 0x1', mode = '1920x1080@60', position = '3840x0', scale = 1, vrr = 0, mirror = 'none', disabled = false })"
        );
        let mut d = c.clone();
        d.mirror = Some("DP-2".into());
        d.scale = 1.25;
        assert!(d.lua().contains("mirror = 'DP-2'"));
        assert!(d.lua().contains("scale = 1.25"));
    }
}

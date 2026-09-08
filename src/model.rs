//! The editable layout: one `MonitorCfg` per connected output, plus geometry helpers.

use crate::hypr::{Monitor, MonitorCfg};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Layout {
    pub monitors: Vec<MonitorCfg>,
}

/// Snap distance in monitor pixels.
pub const SNAP: i32 = 48;

impl Layout {
    pub fn from_live(live: &[Monitor]) -> Layout {
        let name_of = |id: i64| live.iter().find(|m| m.id == id).map(|m| m.name.clone());
        Layout {
            monitors: live
                .iter()
                .map(|m| MonitorCfg {
                    name: m.name.clone(),
                    description: m.description.clone(),
                    mode: m.current_mode().to_string(),
                    x: m.x,
                    y: m.y,
                    scale: m.scale,
                    vrr: m.vrr,
                    mirror: m.mirror_of_id().and_then(name_of),
                    enabled: !m.disabled,
                    available_modes: m.modes().iter().map(|x| x.to_string()).collect(),
                })
                .collect(),
        }
    }

    pub fn by_name(&self, name: &str) -> Option<&MonitorCfg> {
        self.monitors.iter().find(|m| m.name == name)
    }

    /// Logical size (mode / scale) as Hyprland lays it out.
    pub fn logical_size(m: &MonitorCfg) -> (i32, i32) {
        let (w, h) = m.size();
        ((w as f64 / m.scale).round() as i32, (h as f64 / m.scale).round() as i32)
    }

    /// Snap a proposed position of monitor `idx` to the edges of the others.
    pub fn snap(&self, idx: usize, x: i32, y: i32) -> (i32, i32) {
        let (w, h) = Self::logical_size(&self.monitors[idx]);
        let (mut bx, mut by) = (x, y);
        let (mut best_x, mut best_y) = (SNAP + 1, SNAP + 1);
        for (i, o) in self.monitors.iter().enumerate() {
            if i == idx || !o.enabled || o.mirror.is_some() { continue; }
            let (ow, oh) = Self::logical_size(o);
            // candidate x positions: right of o, left of o, aligned left edges
            for cx in [o.x + ow, o.x - w, o.x] {
                let d = (cx - x).abs();
                if d < best_x { best_x = d; bx = cx; }
            }
            for cy in [o.y + oh, o.y - h, o.y] {
                let d = (cy - y).abs();
                if d < best_y { best_y = d; by = cy; }
            }
        }
        (if best_x <= SNAP { bx } else { x }, if best_y <= SNAP { by } else { y })
    }

    /// Shift everything so the top-left-most enabled monitor sits at 0,0.
    pub fn normalize(&mut self) {
        let en: Vec<&MonitorCfg> = self.monitors.iter().filter(|m| m.enabled && m.mirror.is_none()).collect();
        let minx = en.iter().map(|m| m.x).min().unwrap_or(0);
        let miny = en.iter().map(|m| m.y).min().unwrap_or(0);
        for m in &mut self.monitors {
            m.x -= minx;
            m.y -= miny;
        }
    }

    /// Bounding box (x, y, w, h) of enabled, non-mirror monitors in logical px.
    pub fn bounds(&self) -> (i32, i32, i32, i32) {
        let mut minx = i32::MAX; let mut miny = i32::MAX; let mut maxx = i32::MIN; let mut maxy = i32::MIN;
        for m in self.monitors.iter().filter(|m| m.enabled) {
            let (w, h) = Self::logical_size(m);
            minx = minx.min(m.x); miny = miny.min(m.y);
            maxx = maxx.max(m.x + w); maxy = maxy.max(m.y + h);
        }
        if minx == i32::MAX { return (0, 0, 1920, 1080); }
        (minx, miny, maxx - minx, maxy - miny)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hypr::parse_monitors;

    fn layout() -> Layout {
        Layout::from_live(&parse_monitors(include_str!("../tests/fixtures/monitors_all.json")).unwrap())
    }

    #[test]
    fn from_live_keeps_positions_and_modes() {
        let l = layout();
        let dp2 = l.by_name("DP-2").unwrap();
        assert_eq!((dp2.x, dp2.y), (1920, 0));
        assert_eq!(dp2.mode, "1920x1080@100");
        assert!(dp2.enabled);
        assert!(dp2.mirror.is_none());
    }

    #[test]
    fn snaps_to_neighbour_edge() {
        let l = layout();
        let idx = l.monitors.iter().position(|m| m.name == "DP-2").unwrap();
        // dropped 30px short of the HDMI monitor's right edge (1920) → snaps to 1920
        assert_eq!(l.snap(idx, 1890, 12), (1920, 0));
        // far away → untouched
        assert_eq!(l.snap(idx, 3000, 500), (3000, 500));
    }

    #[test]
    fn normalize_shifts_to_origin() {
        let mut l = layout();
        for m in &mut l.monitors { m.x += 100; m.y += 50; }
        l.normalize();
        assert_eq!(l.monitors.iter().map(|m| m.x).min(), Some(0));
        assert_eq!(l.monitors.iter().map(|m| m.y).min(), Some(0));
    }
}

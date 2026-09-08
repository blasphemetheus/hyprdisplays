//! The monitor map: boxes at true relative scale, drag to arrange.

use crate::model::Layout;
use crate::ui::theme;
use iced::mouse;
use iced::widget::canvas::{self, Event, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, Vector};

#[derive(Debug, Clone)]
pub enum MapMessage {
    Select(usize),
    /// Monitor `idx` dropped at logical position (x, y) — not yet snapped.
    Dropped { idx: usize, x: i32, y: i32 },
}

pub struct Map<'a> {
    pub layout: &'a Layout,
    pub selected: Option<usize>,
}

#[derive(Default)]
pub struct MapState {
    drag: Option<Drag>,
    hover: Option<usize>,
}

struct Drag {
    idx: usize,
    grab: Vector, // cursor offset from the box's top-left, in canvas px
    pos: Point,   // current box top-left, in canvas px
}

const PAD: f32 = 28.0;

impl Map<'_> {
    /// Scale (canvas px per logical px) and the origin offset that fit the layout.
    fn transform(&self, bounds: Rectangle) -> (f32, Vector) {
        let (bx, by, bw, bh) = self.layout.bounds();
        let sx = (bounds.width - 2.0 * PAD) / bw.max(1) as f32;
        let sy = (bounds.height - 2.0 * PAD) / bh.max(1) as f32;
        let s = sx.min(sy).min(0.5);
        let ox = (bounds.width - bw as f32 * s) / 2.0 - bx as f32 * s;
        let oy = (bounds.height - bh as f32 * s) / 2.0 - by as f32 * s;
        (s, Vector::new(ox, oy))
    }

    fn rect(&self, i: usize, s: f32, o: Vector) -> Rectangle {
        let m = &self.layout.monitors[i];
        let (w, h) = Layout::logical_size(m);
        let (x, y) = match m.mirror.as_deref().and_then(|src| self.layout.by_name(src)) {
            Some(src) => (src.x, src.y),
            None => (m.x, m.y),
        };
        Rectangle::new(Point::new(x as f32 * s + o.x, y as f32 * s + o.y), Size::new(w as f32 * s, h as f32 * s))
    }

    fn hit(&self, p: Point, s: f32, o: Vector) -> Option<usize> {
        (0..self.layout.monitors.len()).rev().find(|&i| self.rect(i, s, o).contains(p))
    }
}

impl canvas::Program<MapMessage> for Map<'_> {
    type State = MapState;

    fn update(&self, state: &mut MapState, event: &Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<canvas::Action<MapMessage>> {
        let (s, o) = self.transform(bounds);
        let pos = cursor.position_in(bounds);
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = pos?;
                let idx = self.hit(p, s, o)?;
                let r = self.rect(idx, s, o);
                state.drag = Some(Drag { idx, grab: p - r.position(), pos: r.position() });
                Some(canvas::Action::publish(MapMessage::Select(idx)).and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let (Some(d), Some(p)) = (state.drag.as_mut(), pos) {
                    d.pos = p - d.grab;
                    return Some(canvas::Action::request_redraw().and_capture());
                }
                let h = pos.and_then(|p| self.hit(p, s, o));
                if h != state.hover {
                    state.hover = h;
                    return Some(canvas::Action::request_redraw());
                }
                None
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let d = state.drag.take()?;
                let m = &self.layout.monitors[d.idx];
                if m.mirror.is_some() { return Some(canvas::Action::request_redraw()); }
                let x = ((d.pos.x - o.x) / s).round() as i32;
                let y = ((d.pos.y - o.y) / s).round() as i32;
                Some(canvas::Action::publish(MapMessage::Dropped { idx: d.idx, x, y }).and_capture())
            }
            _ => None,
        }
    }

    fn draw(&self, state: &MapState, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let (s, o) = self.transform(bounds);
        let mut frame = Frame::new(renderer, bounds.size());
        // subtle grid
        let grid = Stroke::default().with_color(Color { a: 0.06, ..theme::CREAM }).with_width(1.0);
        let step = 480.0 * s;
        let mut gx = o.x % step;
        while gx < bounds.width { frame.stroke(&Path::line(Point::new(gx, 0.0), Point::new(gx, bounds.height)), grid); gx += step; }
        let mut gy = o.y % step;
        while gy < bounds.height { frame.stroke(&Path::line(Point::new(0.0, gy), Point::new(bounds.width, gy)), grid); gy += step; }

        for (i, m) in self.layout.monitors.iter().enumerate() {
            let mut r = self.rect(i, s, o);
            let dragging = matches!(&state.drag, Some(d) if d.idx == i);
            if let Some(d) = &state.drag { if d.idx == i { r = Rectangle::new(d.pos, r.size()); } }
            let selected = self.selected == Some(i);
            let hovered = state.hover == Some(i);
            let mirror = m.mirror.is_some();
            let (fill, border) = if !m.enabled {
                (Color { a: 0.15, ..theme::DIM }, Color { a: 0.5, ..theme::DIM })
            } else if mirror {
                (Color { a: 0.12, ..theme::ROSE }, Color { a: 0.8, ..theme::ROSE })
            } else if selected {
                (Color { a: 0.28, ..theme::GOLD }, theme::BRIGHT_GOLD)
            } else if hovered {
                (Color { a: 0.35, ..theme::DIM }, theme::GOLD)
            } else {
                (Color { a: 0.28, ..theme::DIM }, Color { a: 0.7, ..theme::CREAM })
            };
            let inset = if mirror { 10.0 } else { 0.0 };
            let rr = Rectangle::new(r.position() + Vector::new(inset, inset), Size::new(r.width - 2.0 * inset, r.height - 2.0 * inset));
            let path = Path::rounded_rectangle(rr.position(), rr.size(), (8.0).into());
            frame.fill(&path, fill);
            frame.stroke(&path, Stroke::default().with_color(border).with_width(if selected || dragging { 2.5 } else { 1.5 }));

            let small = rr.height < 64.0;
            let label = if mirror { format!("{}  ⟲ {}", m.name, m.mirror.as_deref().unwrap_or("")) } else { m.name.clone() };
            frame.fill_text(Text {
                content: label,
                position: Point::new(rr.x + 12.0, rr.y + 10.0),
                color: if m.enabled { theme::CREAM } else { Color { a: 0.5, ..theme::CREAM } },
                size: 15.0.into(),
                ..Text::default()
            });
            if !small { frame.fill_text(Text {
                content: format!("{}  ·  {}", m.mode, if m.scale != 1.0 { format!("×{}", crate::hypr::fmt_scale(m.scale)) } else { String::new() }).trim_end_matches("  ·  ").to_string(),
                position: Point::new(rr.x + 12.0, rr.y + 32.0),
                color: Color { a: 0.75, ..theme::CREAM },
                size: 12.0.into(),
                ..Text::default()
            }); }
            if !small { frame.fill_text(Text {
                content: if m.enabled { format!("{}, {}", m.x, m.y) } else { "disabled".into() },
                position: Point::new(rr.x + 12.0, rr.y + rr.height - 22.0),
                color: Color { a: 0.55, ..theme::CREAM },
                size: 11.0.into(),
                ..Text::default()
            }); }
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, state: &MapState, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if state.drag.is_some() { return mouse::Interaction::Grabbing; }
        let (s, o) = self.transform(bounds);
        match cursor.position_in(bounds).and_then(|p| self.hit(p, s, o)) {
            Some(_) => mouse::Interaction::Grab,
            None => mouse::Interaction::default(),
        }
    }
}

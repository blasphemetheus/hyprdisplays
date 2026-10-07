//! hyprvolume: a mixer that shows what pavucontrol hides — every HDMI port of
//! the GPU labelled by the monitor on it (active or not), each app's stream
//! with the sink it's actually linked to, inputs, and card profiles.

use crate::audio::{self, Output, Snapshot};
use crate::ui::theme;
use anyhow::Result;
use iced::widget::{button, column, container, pick_list, row, scrollable, slider, space, text};
use iced::{Element, Length, Subscription, Task};
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    Refreshed(Result<Snapshot, String>),
    /// Slider moved (local only until released).
    Vol(u64, f32),
    VolReleased(u64),
    Mute(u64, bool),
    Default(Output),
    DefaultSource(u64),
    /// Move a stream (node id) to a sink (node.name).
    Move(u64, String),
    /// Card id, profile description.
    Profile(u64, String),
    Done(Result<String, String>),
}

pub struct VolApp {
    snap: Snapshot,
    status: String,
    busy: bool,
    dragging: Option<u64>,
}

fn refresh() -> Task<Message> {
    Task::perform(async { audio::snapshot().map_err(|e| e.to_string()) }, Message::Refreshed)
}

fn run<F: FnOnce() -> Result<String> + Send + 'static>(f: F) -> Task<Message> {
    Task::perform(async move { f().map_err(|e| e.to_string()) }, Message::Done)
}

impl VolApp {
    pub fn new() -> (Self, Task<Message>) {
        (VolApp { snap: Snapshot::default(), status: "reading pipewire…".into(), busy: false, dragging: None }, refresh())
    }

    pub fn title(&self) -> String {
        "hyprvolume".into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick)
    }

    fn node_mut(&mut self, id: u64) -> Option<(&mut f32, &mut bool)> {
        if let Some(s) = self.snap.sinks.iter_mut().find(|s| s.id == id) { return Some((&mut s.volume, &mut s.mute)); }
        if let Some(s) = self.snap.sources.iter_mut().find(|s| s.id == id) { return Some((&mut s.volume, &mut s.mute)); }
        if let Some(s) = self.snap.streams.iter_mut().find(|s| s.id == id) { return Some((&mut s.volume, &mut s.mute)); }
        None
    }

    fn node_volume(&self, id: u64) -> Option<f32> {
        self.snap
            .sinks
            .iter()
            .chain(self.snap.sources.iter())
            .find(|s| s.id == id)
            .map(|s| s.volume)
            .or_else(|| self.snap.streams.iter().find(|s| s.id == id).map(|s| s.volume))
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => if self.busy || self.dragging.is_some() { Task::none() } else { refresh() },
            Message::Refreshed(Err(e)) => { self.status = format!("pw-dump: {e}"); Task::none() }
            Message::Refreshed(Ok(s)) => {
                self.snap = s;
                if self.status.starts_with("reading") { self.status = "ready".into(); }
                Task::none()
            }
            Message::Vol(id, v) => {
                self.dragging = Some(id);
                if let Some((vol, _)) = self.node_mut(id) { *vol = v; }
                Task::none()
            }
            Message::VolReleased(id) => {
                self.dragging = None;
                let Some(v) = self.node_volume(id) else { return Task::none() };
                self.busy = true;
                run(move || { audio::set_volume(id, v)?; Ok(format!("volume {:.0}%", v * 100.0)) })
            }
            Message::Mute(id, m) => {
                if let Some((_, mute)) = self.node_mut(id) { *mute = m; }
                self.busy = true;
                run(move || { audio::set_mute(id, m)?; Ok(if m { "muted".into() } else { "unmuted".into() }) })
            }
            Message::Default(o) => {
                self.snap.default_sink = o.sink_name();
                self.busy = true;
                run(move || { let n = audio::select_output(&o)?; Ok(format!("default output → {} ({n})", o.label())) })
            }
            Message::DefaultSource(id) => {
                if let Some(s) = self.snap.sources.iter().find(|s| s.id == id) { self.snap.default_source = s.name.clone(); }
                self.busy = true;
                run(move || { audio::set_default_node(id)?; Ok("default input set".into()) })
            }
            Message::Move(stream, sink) => {
                let sid = self.snap.sink_by_name(&sink).map(|s| s.id);
                if let Some(st) = self.snap.streams.iter_mut().find(|s| s.id == stream) { st.sink_id = sid; }
                self.busy = true;
                run(move || { audio::move_stream(stream, &sink)?; Ok(format!("moved to {sink}")) })
            }
            Message::Profile(card, desc) => {
                let Some(c) = self.snap.card_by_id(card) else { return Task::none() };
                let Some(p) = c.profiles.iter().find(|p| p.description == desc).cloned() else { return Task::none() };
                self.busy = true;
                run(move || { audio::set_profile(card, p.index)?; Ok(format!("profile → {}", p.description)) })
            }
            Message::Done(r) => {
                self.busy = false;
                match r { Ok(s) => self.status = s, Err(e) => self.status = format!("error: {e}") }
                refresh()
            }
        }
    }

    /// One level row: title, optional subtitle, slider, percentage, mute, trailing widget.
    fn level<'a>(&self, title: String, sub: Option<String>, id: u64, vol: f32, mute: bool, trailing: Element<'a, Message>) -> Element<'a, Message> {
        let head: Element<'a, Message> = match sub {
            Some(s) => column![text(title).size(14).color(theme::CREAM), text(s).size(10).color(theme::DIM)].spacing(1).into(),
            None => text(title).size(14).color(theme::CREAM).into(),
        };
        let mute_btn = button(text(if mute { "muted" } else { "mute" }).size(11))
            .style(if mute { theme::danger } else { theme::quiet })
            .on_press(Message::Mute(id, !mute));
        let pct = text(format!("{:>4.0}%", vol * 100.0)).size(12).color(if vol > 1.0 { theme::BRIGHT_GOLD } else { theme::CREAM }).width(46);
        container(
            column![
                row![head, space().width(Length::Fill), trailing].spacing(8).align_y(iced::Alignment::Center),
                row![
                    slider(0.0_f32..=1.5, vol, move |v| Message::Vol(id, v)).step(0.01_f32).on_release(Message::VolReleased(id)).width(Length::Fill),
                    pct,
                    mute_btn,
                ]
                .spacing(10)
                .align_y(iced::Alignment::Center),
            ]
            .spacing(6),
        )
        .style(theme::module)
        .padding(10)
        .width(Length::Fill)
        .into()
    }

    fn outputs(&self) -> Element<'_, Message> {
        let cur = self.snap.default_output();
        let ports = self.snap.hdmi_ports();
        let mut rows: Vec<Element<'_, Message>> = vec![text("Outputs").size(14).color(theme::GOLD).into()];
        for s in &self.snap.sinks {
            // An HDMI sink is labelled by its port + display.
            let port = ports.iter().find(|p| p.sink_name() == s.name).cloned();
            let out = port.clone().map(Output::Hdmi).unwrap_or(Output::Sink(s.clone()));
            let is_default = s.name == self.snap.default_sink;
            let star = button(text(if is_default { "★ default" } else { "☆ default" }).size(11))
                .style(if is_default { theme::primary } else { theme::quiet })
                .on_press(Message::Default(out.clone()));
            let sub = port.map(|p| p.card_name.clone()).or_else(|| Some(s.name.clone()));
            rows.push(self.level(out.label(), sub, s.id, s.volume, s.mute, star.into()));
        }
        // HDMI ports whose sink is NOT active right now (other profile selected).
        for p in ports.iter().filter(|p| p.monitor.is_some() && self.snap.sink_by_name(&p.sink_name()).is_none()) {
            let o = Output::Hdmi(p.clone());
            let is_cur = cur.as_ref() == Some(&o);
            rows.push(
                container(
                    row![
                        column![text(o.label()).size(14).color(theme::CREAM), text(format!("{} · inactive, switches the card profile", p.profile_name)).size(10).color(theme::DIM)].spacing(1),
                        space().width(Length::Fill),
                        button(text(if is_cur { "★ default" } else { "use" }).size(11)).style(theme::quiet).on_press(Message::Default(o.clone())),
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                )
                .style(theme::module)
                .padding(10)
                .width(Length::Fill)
                .into(),
            );
        }
        column(rows).spacing(8).into()
    }

    fn streams(&self) -> Element<'_, Message> {
        let mut rows: Vec<Element<'_, Message>> = vec![text("Playing").size(14).color(theme::GOLD).into()];
        if self.snap.streams.is_empty() {
            rows.push(text("nothing playing").size(12).color(theme::DIM).into());
        }
        let ports = self.snap.hdmi_ports();
        let sink_label = |s: &audio::Sink| ports.iter().find(|p| p.sink_name() == s.name).map(|p| Output::Hdmi(p.clone()).label()).unwrap_or_else(|| s.description.clone());
        let labels: Vec<String> = self.snap.sinks.iter().map(sink_label).collect();
        for st in &self.snap.streams {
            let sel = st.sink_id.and_then(|id| self.snap.sinks.iter().find(|s| s.id == id)).map(sink_label);
            let sinks = self.snap.sinks.clone();
            let labels2 = labels.clone();
            let id = st.id;
            let target = pick_list(labels.clone(), sel, move |d| {
                let name = labels2.iter().position(|l| *l == d).and_then(|i| sinks.get(i)).map(|s| s.name.clone()).unwrap_or_default();
                Message::Move(id, name)
            })
            .text_size(11)
            .width(260);
            rows.push(self.level(st.label(), None, st.id, st.volume, st.mute, target.into()));
        }
        column(rows).spacing(8).into()
    }

    fn inputs(&self) -> Element<'_, Message> {
        let mut rows: Vec<Element<'_, Message>> = vec![text("Inputs").size(14).color(theme::GOLD).into()];
        for s in &self.snap.sources {
            let is_default = s.name == self.snap.default_source;
            let star = button(text(if is_default { "★ default" } else { "☆ default" }).size(11))
                .style(if is_default { theme::primary } else { theme::quiet })
                .on_press(Message::DefaultSource(s.id));
            rows.push(self.level(s.description.clone(), Some(s.name.clone()), s.id, s.volume, s.mute, star.into()));
        }
        column(rows).spacing(8).into()
    }

    fn cards(&self) -> Element<'_, Message> {
        let mut rows: Vec<Element<'_, Message>> = vec![text("Cards").size(14).color(theme::GOLD).into()];
        for c in &self.snap.cards {
            let opts: Vec<String> = c.profiles.iter().filter(|p| p.available).map(|p| p.description.clone()).collect();
            let sel = c.profile_named(&c.profile).map(|p| p.description.clone());
            let id = c.id;
            rows.push(
                container(
                    row![
                        column![text(&c.description).size(14).color(theme::CREAM), text(&c.name).size(10).color(theme::DIM)].spacing(1),
                        space().width(Length::Fill),
                        pick_list(opts, sel, move |d| Message::Profile(id, d)).text_size(11).width(300),
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                )
                .style(theme::module)
                .padding(10)
                .width(Length::Fill)
                .into(),
            );
        }
        column(rows).spacing(8).into()
    }

    pub fn view(&self) -> Element<'_, Message> {
        let body = column![self.outputs(), self.streams(), self.inputs(), self.cards()].spacing(18);
        let status = row![
            text(&self.status).size(12).color(theme::CREAM),
            space().width(Length::Fill),
            text(if self.busy { "…" } else { "" }).size(12).color(theme::DIM),
        ]
        .padding([4, 8]);
        column![
            row![text("hyprvolume").size(22).color(theme::GOLD)].padding([0, 4]),
            container(scrollable(body).height(Length::Fill)).style(theme::card).padding(16).width(Length::Fill).height(Length::Fill),
            status,
        ]
        .spacing(8)
        .padding(12)
        .into()
    }
}

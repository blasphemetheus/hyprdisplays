//! iced application: state, messages, update, view.

use crate::hypr::{self, Monitor, MonitorCfg};
use crate::model::Layout;
use crate::ui::map::{Map, MapMessage};
use crate::ui::theme;
use crate::{audio, lua_writer, profiles};
use anyhow::Result;
use iced::widget::{button, canvas, column, container, pick_list, row, scrollable, slider, space, text, text_input, toggler};
use iced::{Element, Length, Subscription, Task};
use std::time::Duration;

const MIRROR_STATE: &str = ".local/state/hypr/mirror-prev-sink";

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    Refreshed(Result<Snapshot, String>),
    Map(MapMessage),
    ModeChanged(String),
    ScaleChanged(f64),
    ScaleReleased,
    VrrToggled(bool),
    EnabledToggled(bool),
    MirrorChanged(String),
    SinkChanged(String),
    WakeBounce,
    RescueWorkspaces,
    TvPreset,
    Normalize,
    WriteLua,
    ReloadConfig,
    ProfileNameChanged(String),
    ProfilePicked(String),
    SaveProfile,
    LoadProfile,
    DeleteProfile,
    Done(Result<String, String>),
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub monitors: Vec<Monitor>,
    pub sinks: Vec<audio::Sink>,
    pub default_sink: String,
}

pub struct App {
    live: Vec<Monitor>,
    layout: Layout,
    selected: Option<usize>,
    sinks: Vec<audio::Sink>,
    default_sink: String,
    profiles: Vec<String>,
    profile_name: String,
    status: String,
    busy: bool,
    /// Live changes not yet written to lua/monitors.lua.
    dirty: bool,
}

fn snapshot() -> Result<Snapshot> {
    Ok(Snapshot { monitors: hypr::monitors()?, sinks: audio::sinks().unwrap_or_default(), default_sink: audio::default_sink().unwrap_or_default() })
}

fn refresh() -> Task<Message> {
    Task::perform(async { snapshot().map_err(|e| e.to_string()) }, Message::Refreshed)
}

fn run<F: FnOnce() -> Result<String> + Send + 'static>(f: F) -> Task<Message> {
    Task::perform(async move { f().map_err(|e| e.to_string()) }, Message::Done)
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        let app = App {
            live: vec![],
            layout: Layout::default(),
            selected: None,
            sinks: vec![],
            default_sink: String::new(),
            profiles: profiles::list(),
            profile_name: String::new(),
            status: "reading monitors…".into(),
            busy: false,
            dirty: false,
        };
        (app, refresh())
    }

    pub fn title(&self) -> String {
        "hyprdisplays".into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        iced::time::every(Duration::from_secs(2)).map(|_| Message::Tick)
    }

    fn sel(&mut self) -> Option<&mut MonitorCfg> {
        let i = self.selected?;
        self.layout.monitors.get_mut(i)
    }

    /// Apply the selected monitor's rule live.
    fn apply_selected(&mut self) -> Task<Message> {
        let Some(cfg) = self.selected.and_then(|i| self.layout.monitors.get(i).cloned()) else { return Task::none() };
        self.busy = true;
        self.dirty = true;
        run(move || { hypr::apply(&cfg)?; Ok(format!("applied {}: {}", cfg.name, cfg.mode)) })
    }

    fn apply_all(&mut self, what: &'static str) -> Task<Message> {
        let cfgs: Vec<MonitorCfg> = self.layout.monitors.clone();
        self.busy = true;
        self.dirty = true;
        run(move || { for c in &cfgs { hypr::apply(c)?; } Ok(format!("{what}: {} rule(s) applied", cfgs.len())) })
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => if self.busy { Task::none() } else { refresh() },
            Message::Refreshed(Err(e)) => { self.status = format!("hyprctl: {e}"); Task::none() }
            Message::Refreshed(Ok(s)) => {
                let fresh = Layout::from_live(&s.monitors);
                // Keep local edits while hyprctl catches up; adopt live state when the set of monitors changes.
                let names_changed = fresh.monitors.iter().map(|m| &m.name).ne(self.layout.monitors.iter().map(|m| &m.name));
                if names_changed || !self.dirty { self.layout = fresh; }
                if names_changed { self.selected = None; }
                if self.selected.is_none() {
                    self.selected = s.monitors.iter().position(|m| m.focused).or(if s.monitors.is_empty() { None } else { Some(0) });
                }
                self.live = s.monitors;
                self.sinks = s.sinks;
                self.default_sink = s.default_sink;
                if self.status.starts_with("reading") { self.status = "ready".into(); }
                Task::none()
            }
            Message::Map(MapMessage::Select(i)) => { self.selected = Some(i); Task::none() }
            Message::Map(MapMessage::Dropped { idx, x, y }) => {
                let (sx, sy) = self.layout.snap(idx, x, y);
                self.layout.monitors[idx].x = sx;
                self.layout.monitors[idx].y = sy;
                self.layout.normalize();
                self.selected = Some(idx);
                self.apply_all("moved")
            }
            Message::ModeChanged(m) => { if let Some(c) = self.sel() { c.mode = m; } self.apply_selected() }
            Message::ScaleChanged(v) => { if let Some(c) = self.sel() { c.scale = (v * 4.0).round() / 4.0; } Task::none() }
            Message::ScaleReleased => self.apply_selected(),
            Message::VrrToggled(v) => { if let Some(c) = self.sel() { c.vrr = v; } self.apply_selected() }
            Message::EnabledToggled(v) => { if let Some(c) = self.sel() { c.enabled = v; } self.apply_selected() }
            Message::MirrorChanged(v) => {
                if let Some(c) = self.sel() { c.mirror = if v == "none" { None } else { Some(v) }; }
                self.apply_selected()
            }
            Message::SinkChanged(name) => {
                self.default_sink = name.clone();
                self.busy = true;
                run(move || { audio::set_default(&name)?; Ok(format!("audio → {name}")) })
            }
            Message::WakeBounce => {
                let Some(cfg) = self.selected.and_then(|i| self.layout.monitors.get(i).cloned()) else { return Task::none() };
                self.busy = true;
                Task::perform(async move {
                    let r: Result<String> = (|| {
                        hypr::dpms(true)?;
                        std::thread::sleep(Duration::from_secs(1));
                        let mut low = cfg.clone();
                        if let Some(m) = cfg.mode_parsed() { low.mode = format!("{}x{}@60", m.width, m.height); }
                        low.vrr = false;
                        hypr::apply(&low)?;
                        std::thread::sleep(Duration::from_secs(2));
                        hypr::apply(&cfg)?;
                        Ok(format!("{}: 60Hz bounce → {}", cfg.name, cfg.mode))
                    })();
                    r.map_err(|e| e.to_string())
                }, Message::Done)
            }
            Message::TvPreset => {
                // Toggle: mirror the FOCUSED monitor onto the SELECTED one + route audio to HDMI,
                // remembering the previous sink in the same state file mirror-toggle.sh uses.
                let Some(i) = self.selected else { return Task::none() };
                let focused = self.live.iter().find(|m| m.focused).map(|m| m.name.clone());
                let hdmi = self.sinks.iter().find(|s| s.is_hdmi()).map(|s| s.name.clone());
                let state = dirs::home_dir().unwrap_or_default().join(MIRROR_STATE);
                let cur_sink = self.default_sink.clone();
                let target = &mut self.layout.monitors[i];
                let turning_on = target.mirror.is_none();
                target.mirror = if turning_on { focused.filter(|f| f != &target.name) } else { None };
                let cfg = target.clone();
                self.busy = true;
                self.dirty = true;
                run(move || {
                    hypr::apply(&cfg)?;
                    if turning_on {
                        if let Some(h) = hdmi {
                            let _ = std::fs::create_dir_all(state.parent().unwrap());
                            let _ = std::fs::write(&state, &cur_sink);
                            audio::set_default(&h)?;
                        }
                        Ok(format!("{} mirrors {}, audio via HDMI", cfg.name, cfg.mirror.as_deref().unwrap_or("?")))
                    } else {
                        if let Ok(prev) = std::fs::read_to_string(&state) {
                            let _ = audio::set_default(prev.trim());
                            let _ = std::fs::remove_file(&state);
                        }
                        Ok(format!("{} extended again, audio restored", cfg.name))
                    }
                })
            }
            Message::RescueWorkspaces => { self.busy = true; run(|| { hypr::rescue_workspaces()?; Ok("workspaces re-homed to their monitors".into()) }) }
            Message::Normalize => { self.layout.normalize(); self.apply_all("normalized") }
            Message::WriteLua => {
                let layout = self.layout.clone();
                self.busy = true;
                run(move || {
                    let p = lua_writer::default_path();
                    lua_writer::write(&p, &layout)?;
                    Ok(format!("wrote {}", p.display()))
                })
                .chain(Task::done(Message::Done(Ok(String::new()))))
                .map(|m| match m { Message::Done(Ok(s)) if s.is_empty() => Message::Done(Ok("saved".into())), m => m })
            }
            Message::ReloadConfig => { self.busy = true; self.dirty = false; run(|| { hypr::reload()?; Ok("hyprctl reload".into()) }) }
            Message::ProfileNameChanged(s) => { self.profile_name = s; Task::none() }
            Message::ProfilePicked(s) => { self.profile_name = s; Task::none() }
            Message::SaveProfile => {
                let name = self.profile_name.trim().to_string();
                if name.is_empty() { self.status = "profile name?".into(); return Task::none(); }
                let p = profiles::Profile { name: name.clone(), layout: self.layout.clone(), sink: Some(self.default_sink.clone()) };
                let r = profiles::save(&p);
                self.profiles = profiles::list();
                self.status = match r { Ok(()) => format!("saved profile '{name}'"), Err(e) => e.to_string() };
                Task::none()
            }
            Message::LoadProfile => {
                let name = self.profile_name.trim().to_string();
                if name.is_empty() { return Task::none(); }
                self.busy = true;
                self.dirty = true;
                run(move || profiles::apply(&profiles::load(&name)?))
            }
            Message::DeleteProfile => {
                let name = self.profile_name.trim().to_string();
                self.status = match profiles::delete(&name) { Ok(()) => format!("deleted '{name}'"), Err(e) => e.to_string() };
                self.profiles = profiles::list();
                self.profile_name.clear();
                Task::none()
            }
            Message::Done(r) => {
                self.busy = false;
                match r {
                    Ok(s) => { if s == "saved" { self.dirty = false; } else { self.status = s; } }
                    Err(e) => self.status = format!("error: {e}"),
                }
                refresh()
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let map_el: Element<'_, MapMessage> = canvas(Map { layout: &self.layout, selected: self.selected }).width(Length::Fill).height(Length::Fill).into();
        let map = container(map_el.map(Message::Map))
        .style(theme::card)
        .padding(8)
        .width(Length::FillPortion(3))
        .height(Length::Fill);

        let panel = container(scrollable(self.panel()).height(Length::Fill))
            .style(theme::card)
            .padding(16)
            .width(Length::FillPortion(2))
            .height(Length::Fill);

        let status = row![
            text(if self.dirty { "● unsaved" } else { "○ saved" }).size(12).color(if self.dirty { theme::BRIGHT_GOLD } else { theme::DIM }),
            space().width(12),
            text(&self.status).size(12).color(theme::CREAM),
            space().width(Length::Fill),
            text(if self.busy { "…" } else { "" }).size(12).color(theme::DIM),
        ]
        .padding([4, 8]);

        column![row![map, panel].spacing(12).height(Length::Fill), status]
            .spacing(8)
            .padding(12)
            .into()
    }

    fn panel(&self) -> Element<'_, Message> {
        let header = text("hyprdisplays").size(22).color(theme::GOLD);
        let Some(i) = self.selected else {
            return column![header, text("no monitor selected").color(theme::DIM)].spacing(12).into();
        };
        let m = &self.layout.monitors[i];
        let others: Vec<String> = std::iter::once("none".to_string())
            .chain(self.layout.monitors.iter().filter(|o| o.name != m.name).map(|o| o.name.clone()))
            .collect();
        let mirror_sel = m.mirror.clone().unwrap_or_else(|| "none".into());
        let modes: Vec<String> = if m.available_modes.is_empty() { vec![m.mode.clone()] } else { m.available_modes.clone() };

        let title = column![
            text(&m.name).size(18).color(theme::CREAM),
            text(&m.description).size(11).color(theme::DIM),
        ]
        .spacing(2);

        let controls = column![
            row![text("Enabled").width(90), toggler(m.enabled).on_toggle(Message::EnabledToggled)].spacing(10),
            row![text("Mode").width(90), pick_list(modes, Some(m.mode.clone()), Message::ModeChanged).width(Length::Fill)].spacing(10),
            row![
                text("Scale").width(90),
                slider(0.5..=3.0, m.scale, Message::ScaleChanged).step(0.25).on_release(Message::ScaleReleased).width(Length::Fill),
                text(format!("×{}", hypr::fmt_scale(m.scale))).width(50),
            ]
            .spacing(10),
            row![text("VRR").width(90), toggler(m.vrr).on_toggle(Message::VrrToggled)].spacing(10),
            row![text("Mirror of").width(90), pick_list(others, Some(mirror_sel), Message::MirrorChanged).width(Length::Fill)].spacing(10),
            row![text("Position").width(90), text(format!("{}, {}", m.x, m.y)).color(theme::DIM), space().width(Length::Fill),
                 button(text("Normalize").size(12)).style(theme::quiet).on_press(Message::Normalize)].spacing(10),
        ]
        .spacing(10);

        let sink_names: Vec<String> = self.sinks.iter().map(|s| s.description.clone()).collect();
        let sink_sel = self.sinks.iter().find(|s| s.name == self.default_sink).map(|s| s.description.clone());
        let sinks = self.sinks.clone();
        let audio_row = row![
            text("Audio").width(90),
            pick_list(sink_names, sink_sel, move |d| Message::SinkChanged(sinks.iter().find(|s| s.description == d).map(|s| s.name.clone()).unwrap_or_default())).width(Length::Fill),
        ]
        .spacing(10);

        let actions = column![
            row![
                button(text(if m.mirror.is_some() { "⟲ Stop mirroring / restore audio" } else { "⟲ Mirror focused → here + HDMI audio" }).size(13)).style(theme::primary).on_press(Message::TvPreset),
            ],
            row![
                button(text("HDMI wake bounce (60Hz → back)").size(13)).style(theme::quiet).on_press(Message::WakeBounce),
            ],
            row![
                button(text("Rescue workspaces (after unplug)").size(13)).style(theme::quiet).on_press(Message::RescueWorkspaces),
            ],
        ]
        .spacing(8);

        let prof_pick = pick_list(self.profiles.clone(), if self.profiles.contains(&self.profile_name) { Some(self.profile_name.clone()) } else { None }, Message::ProfilePicked).placeholder("profiles…").width(Length::Fill);
        let prof = column![
            text("Profiles").size(14).color(theme::GOLD),
            row![prof_pick, text_input("name", &self.profile_name).on_input(Message::ProfileNameChanged).width(140)].spacing(8),
            row![
                button(text("Save").size(12)).style(theme::primary).on_press(Message::SaveProfile),
                button(text("Load").size(12)).style(theme::quiet).on_press(Message::LoadProfile),
                button(text("Delete").size(12)).style(theme::danger).on_press(Message::DeleteProfile),
            ]
            .spacing(8),
        ]
        .spacing(8);

        let persist = column![
            text("Config").size(14).color(theme::GOLD),
            row![
                button(text("Write lua/monitors.lua").size(12)).style(theme::primary).on_press(Message::WriteLua),
                button(text("Reload from config").size(12)).style(theme::quiet).on_press(Message::ReloadConfig),
            ]
            .spacing(8),
        ]
        .spacing(8);

        column![
            header,
            container(title).style(theme::module).padding(10).width(Length::Fill),
            controls,
            audio_row,
            actions,
            container(prof).style(theme::module).padding(10).width(Length::Fill),
            container(persist).style(theme::module).padding(10).width(Length::Fill),
        ]
        .spacing(14)
        .into()
    }
}

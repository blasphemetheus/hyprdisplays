//! hyprbinds iced application: list + editor for lua/binds.lua.

use crate::binds::{self, Action, Bind, BindsFile};
use crate::hypr;
use crate::ui::theme;
use anyhow::Result;
use iced::keyboard::{self, key};
use iced::widget::{button, checkbox, column, container, mouse_area, row, rule, scrollable, space, text, text_input};
use iced::{Color, Element, Font, Length, Subscription, Task};
use std::path::PathBuf;

const SUBMAP: &str = "hyprbinds-capture";

#[derive(Debug, Clone)]
pub enum Message {
    FilterChanged(String),
    Select(usize),
    KeysChanged(String),
    ActionChanged(String),
    KindChanged(bool),
    Repeating(bool),
    Locked(bool),
    Release(bool),
    Drag(bool),
    CommentChanged(String),
    StartCapture,
    CancelCapture,
    Key(keyboard::Event),
    AddBind,
    DeleteBind,
    Save,
    Revert,
    Done(Result<String, String>),
}

pub struct BindsApp {
    path: PathBuf,
    file: BindsFile,
    filter: String,
    selected: Option<usize>,
    capturing: bool,
    status: String,
    busy: bool,
}

fn run<F: FnOnce() -> Result<String> + Send + 'static>(f: F) -> Task<Message> {
    Task::perform(async move { f().map_err(|e| e.to_string()) }, Message::Done)
}

fn alpha(c: Color, a: f32) -> Color { Color { a, ..c } }

fn selected_row(_t: &iced::Theme) -> container::Style {
    container::Style { background: Some(alpha(theme::GOLD, 0.22).into()), border: iced::Border { radius: 8.0.into(), ..Default::default() }, ..Default::default() }
}
fn plain_row(_t: &iced::Theme) -> container::Style {
    container::Style { border: iced::Border { radius: 8.0.into(), ..Default::default() }, ..Default::default() }
}
fn chip(_t: &iced::Theme) -> container::Style {
    container::Style {
        text_color: Some(theme::BG),
        background: Some(alpha(theme::GOLD, 0.85).into()),
        border: iced::Border { radius: 6.0.into(), ..Default::default() },
        ..Default::default()
    }
}

impl BindsApp {
    pub fn new(path: PathBuf) -> (Self, Task<Message>) {
        let (file, status) = match BindsFile::load(&path) {
            Ok(f) => { let n = f.binds.len(); (f, format!("{n} binds in {}", path.display())) }
            Err(e) => (BindsFile::parse(&path, ""), format!("{e}")),
        };
        (BindsApp { path, file, filter: String::new(), selected: None, capturing: false, status, busy: false }, Task::none())
    }

    pub fn title(&self) -> String {
        if self.file.is_dirty() { "hyprbinds •".into() } else { "hyprbinds".into() }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        if self.capturing { keyboard::listen().map(Message::Key) } else { Subscription::none() }
    }

    fn sel(&mut self) -> Option<&mut Bind> {
        let i = self.selected?;
        self.file.binds.get_mut(i).filter(|b| b.editable)
    }

    fn leave_capture(&mut self) -> Task<Message> {
        self.capturing = false;
        run(|| { hypr::submap("reset")?; Ok(String::new()) })
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::FilterChanged(s) => { self.filter = s; Task::none() }
            Message::Select(i) => {
                self.selected = Some(i);
                if self.capturing { return self.leave_capture(); }
                Task::none()
            }
            Message::KeysChanged(s) => { if let Some(b) = self.sel() { b.keys = s; } Task::none() }
            Message::ActionChanged(s) => {
                if let Some(b) = self.sel() {
                    b.action = match b.action { Action::Exec(_) => Action::Exec(s), Action::Lua(_) => Action::Lua(s) };
                }
                Task::none()
            }
            Message::KindChanged(exec) => {
                if let Some(b) = self.sel() {
                    let t = b.action.text().to_string();
                    b.action = if exec { Action::Exec(t) } else { Action::Lua(t) };
                }
                Task::none()
            }
            Message::Repeating(v) => { if let Some(b) = self.sel() { b.flags.repeating = v; } Task::none() }
            Message::Locked(v) => { if let Some(b) = self.sel() { b.flags.locked = v; } Task::none() }
            Message::Release(v) => { if let Some(b) = self.sel() { b.flags.release = v; } Task::none() }
            Message::Drag(v) => { if let Some(b) = self.sel() { b.flags.drag = v; } Task::none() }
            Message::CommentChanged(s) => { if let Some(b) = self.sel() { b.comment = s; } Task::none() }
            Message::StartCapture => {
                if self.sel().is_none() { return Task::none(); }
                self.capturing = true;
                self.status = "press the new combo (Escape cancels; mouse buttons must be typed, e.g. mouse:272)".into();
                // Empty submap: Hyprland stops firing binds, so the combo reaches this window.
                run(|| { hypr::submap(SUBMAP)?; Ok(String::new()) })
            }
            Message::CancelCapture => { self.status = "capture cancelled".into(); self.leave_capture() }
            Message::Key(keyboard::Event::KeyPressed { physical_key, modifiers, repeat, .. }) => {
                if repeat || !self.capturing { return Task::none(); }
                let code = match physical_key { key::Physical::Code(c) => format!("{c:?}"), key::Physical::Unidentified(_) => String::new() };
                if code == "Escape" && modifiers.is_empty() { return self.update(Message::CancelCapture); }
                let Some(name) = binds::hypr_key_name(&code) else { return Task::none() }; // modifier-only or unknown: keep waiting
                let mut parts: Vec<&str> = vec![];
                if modifiers.logo() { parts.push("SUPER"); }
                if modifiers.control() { parts.push("CTRL"); }
                if modifiers.alt() { parts.push("ALT"); }
                if modifiers.shift() { parts.push("SHIFT"); }
                parts.push(&name);
                let combo = parts.join(" + ");
                if let Some(b) = self.sel() { b.keys = combo.clone(); }
                self.status = format!("captured {combo}");
                self.leave_capture()
            }
            Message::Key(_) => Task::none(),
            Message::AddBind => {
                let section = self.selected.and_then(|i| self.file.binds.get(i)).map(|b| b.section.clone()).unwrap_or_else(|| "General".into());
                self.file.binds.push(Bind::new(&section));
                self.selected = Some(self.file.binds.len() - 1);
                self.filter.clear();
                self.status = "new bind: set keys and a command, then Save".into();
                Task::none()
            }
            Message::DeleteBind => {
                if let Some(i) = self.selected.filter(|i| self.file.binds.get(*i).map(|b| b.editable).unwrap_or(false)) {
                    let b = self.file.binds.remove(i);
                    self.selected = None;
                    self.status = format!("removed {} (unsaved)", b.keys);
                }
                Task::none()
            }
            Message::Save => {
                let empty: Vec<String> = self.file.binds.iter().filter(|b| b.editable && (b.keys.trim().is_empty() || b.keys.trim() == "SUPER +" || b.action.text().trim().is_empty())).map(|b| b.keys.clone()).collect();
                if !empty.is_empty() { self.status = "a bind is missing its keys or command".into(); return Task::none(); }
                let file = self.file.clone();
                let n = file.binds.len();
                self.busy = true;
                run(move || {
                    file.save()?;
                    hypr::reload()?;
                    Ok(format!("saved {n} binds to {} and reloaded Hyprland (previous copy in binds.lua.bak)", file.path.display()))
                })
            }
            Message::Revert => {
                match BindsFile::load(&self.path) {
                    Ok(f) => { self.file = f; self.selected = None; self.status = "reverted to the file on disk".into(); }
                    Err(e) => self.status = e.to_string(),
                }
                Task::none()
            }
            Message::Done(r) => {
                self.busy = false;
                match r {
                    Ok(s) if s.is_empty() => {}
                    Ok(s) => self.status = s,
                    Err(e) => self.status = format!("error: {e}"),
                }
                if self.status.starts_with("saved") {
                    // Re-read so line numbers match the written file.
                    if let Ok(f) = BindsFile::load(&self.path) { self.file = f; }
                }
                Task::none()
            }
        }
    }

    fn matches(&self, b: &Bind) -> bool {
        let q = self.filter.trim().to_lowercase();
        if q.is_empty() { return true; }
        q.split_whitespace().all(|w| {
            b.keys.to_lowercase().contains(w) || b.describe().to_lowercase().contains(w)
                || b.action.text().to_lowercase().contains(w) || b.section.to_lowercase().contains(w)
        })
    }

    fn list(&self) -> Element<'_, Message> {
        let mut col = column![].spacing(2);
        let mut sections: Vec<String> = self.file.sections.clone();
        for b in &self.file.binds { if !sections.contains(&b.section) { sections.push(b.section.clone()); } }
        for sec in sections {
            let rows: Vec<(usize, &Bind)> = self.file.binds.iter().enumerate().filter(|(_, b)| b.section == sec && self.matches(b)).collect();
            if rows.is_empty() { continue; }
            col = col.push(space().height(8));
            col = col.push(text(sec.clone()).size(12).color(theme::ROSE));
            for (i, b) in rows {
                let dup = !self.file.conflicts(i).is_empty();
                let mut r = row![
                    container(text(b.keys.clone()).size(12).font(Font::MONOSPACE)).style(chip).padding([2, 7]),
                    text(b.describe()).size(13).width(Length::Fill),
                ].spacing(10).align_y(iced::Alignment::Center);
                if b.submap.is_some() { r = r.push(text(format!("submap {}", b.submap.clone().unwrap())).size(11).color(theme::DIM)); }
                if !b.editable { r = r.push(text("generated").size(11).color(theme::DIM)); }
                let flags = b.flags.summary();
                if !flags.is_empty() { r = r.push(text(flags).size(11).color(theme::DIM)); }
                if dup { r = r.push(text("⚠ duplicate").size(11).color(theme::FAIL)); }
                let style: fn(&iced::Theme) -> container::Style = if self.selected == Some(i) { selected_row } else { plain_row };
                col = col.push(mouse_area(container(r).padding([4, 8]).width(Length::Fill).style(style)).on_press(Message::Select(i)));
            }
        }
        scrollable(col.padding([0, 8])).height(Length::Fill).into()
    }

    fn editor(&self) -> Element<'_, Message> {
        let Some(b) = self.selected.and_then(|i| self.file.binds.get(i)) else {
            return container(column![
                text("Select a bind to edit it").size(14),
                text("Capture presses the real combo; Save writes lua/binds.lua and reloads Hyprland.").size(12).color(theme::DIM),
            ].spacing(8)).padding(16).into();
        };
        if !b.editable {
            return container(column![
                text("Generated bind").size(16).color(theme::GOLD),
                text("This one comes from a loop, a submap, or a local variable the editor can't rewrite safely. Edit it in binds.lua directly.").size(12).color(theme::DIM),
                space().height(6),
                text(format!("keys:    {}", b.keys)).font(Font::MONOSPACE).size(12),
                text(format!("action:  {}", b.action.text())).font(Font::MONOSPACE).size(12),
            ].spacing(6)).padding(16).into();
        }
        let is_exec = b.action.is_exec();
        let capture = if self.capturing {
            button(text("● listening…").size(12)).style(theme::danger).on_press(Message::CancelCapture)
        } else {
            button(text("Capture").size(12)).style(theme::primary).on_press(Message::StartCapture)
        };
        let conflicts = self.selected.map(|i| self.file.conflicts(i)).unwrap_or_default();
        let conflict_note: Element<'_, Message> = if conflicts.is_empty() {
            space().height(0).into()
        } else {
            let names: Vec<String> = conflicts.iter().map(|j| self.file.binds[*j].describe()).collect();
            text(format!("⚠ also bound to: {}", names.join(", "))).size(12).color(theme::FAIL).into()
        };
        let kind = row![
            button(text("Shell command").size(12)).style(if is_exec { theme::primary } else { theme::quiet }).on_press(Message::KindChanged(true)),
            button(text("Lua dispatcher").size(12)).style(if is_exec { theme::quiet } else { theme::primary }).on_press(Message::KindChanged(false)),
        ].spacing(6);
        let hint = if is_exec { "Shell, run via hl.dsp.exec_cmd. ~/.config/hypr/scripts/… is written as S .. \"/…\"." }
                   else { "Raw Lua, e.g. hl.dsp.window.close() or hl.dsp.focus({ direction = \"left\" })." };
        container(column![
            text(if b.line.is_some() { "Edit bind" } else { "New bind" }).size(16).color(theme::GOLD),
            text("Keys").size(12).color(theme::DIM),
            row![text_input("SUPER + SHIFT + K", &b.keys).on_input(Message::KeysChanged).font(Font::MONOSPACE).size(13), capture].spacing(8),
            conflict_note,
            text("Action").size(12).color(theme::DIM),
            kind,
            text_input(if is_exec { "command" } else { "hl.dsp.…" }, b.action.text()).on_input(Message::ActionChanged).font(Font::MONOSPACE).size(13),
            text(hint).size(11).color(theme::DIM),
            text("Flags").size(12).color(theme::DIM),
            row![
                checkbox(b.flags.repeating).label("repeat").on_toggle(Message::Repeating).size(16).text_size(12),
                checkbox(b.flags.locked).label("locked (works on lockscreen)").on_toggle(Message::Locked).size(16).text_size(12),
            ].spacing(14),
            row![
                checkbox(b.flags.release).label("on release").on_toggle(Message::Release).size(16).text_size(12),
                checkbox(b.flags.drag).label("mouse drag").on_toggle(Message::Drag).size(16).text_size(12),
            ].spacing(14),
            text("Comment").size(12).color(theme::DIM),
            text_input("shown in the list", &b.comment).on_input(Message::CommentChanged).size(13),
            space().height(6),
            text(b.lua()).font(Font::MONOSPACE).size(11).color(theme::DIM),
            space().height(Length::Fill),
            row![space().width(Length::Fill), button(text("Delete bind").size(12)).style(theme::danger).on_press(Message::DeleteBind)],
        ].spacing(6)).padding(16).into()
    }

    pub fn view(&self) -> Element<'_, Message> {
        let dirty = self.file.is_dirty();
        let top = row![
            text_input("filter: keys, command, section…", &self.filter).on_input(Message::FilterChanged).width(Length::Fill).size(13),
            button(text("+ Add bind").size(12)).style(theme::quiet).on_press(Message::AddBind),
        ].spacing(8);
        let bottom = row![
            text(self.status.clone()).size(12).color(theme::DIM).width(Length::Fill),
            button(text("Revert").size(12)).style(theme::quiet).on_press_maybe(dirty.then_some(Message::Revert)),
            button(text("Save + reload Hyprland").size(12)).style(theme::primary).on_press_maybe((dirty && !self.busy).then_some(Message::Save)),
        ].spacing(8).align_y(iced::Alignment::Center);
        let body = row![
            container(self.list()).style(theme::card).padding(8).width(Length::FillPortion(3)).height(Length::Fill),
            container(scrollable(self.editor()).height(Length::Fill)).style(theme::card).width(Length::FillPortion(2)).height(Length::Fill),
        ].spacing(12);
        column![top, body, rule::horizontal(1), bottom].spacing(10).padding(14).into()
    }
}

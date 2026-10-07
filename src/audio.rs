//! PipeWire via `pw-dump` / `wpctl` / `pw-metadata` (there is no pactl on this box).
//!
//! One `pw-dump` gives everything: sinks, sources, playback streams (with the
//! sink they're linked to), cards with their profiles/routes, and the default
//! sink/source from the "default" metadata object.
//!
//! HDMI on an NVIDIA card is the awkward one: the card exposes ONE sink at a
//! time, chosen by the card *profile* (`output:hdmi-stereo`, `…-extra1`, …),
//! and PipeWire's route descriptions ("HDMI / DisplayPort 3") never say which
//! monitor hangs off that port. The kernel does: `/proc/asound/cardN/eld#*`
//! carries each pin's EDID-like data (`monitor_name`). [`HdmiPort`] joins the
//! two so "audio to the TV" means: pick the port whose ELD names the TV, switch
//! the card to that port's stereo profile, wait for the sink, make it default.

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub struct Sink {
    pub id: u64,
    pub name: String,
    pub description: String,
    /// `device.id` of the owning card (None for virtual sinks).
    pub device_id: Option<u64>,
    /// Volume on wpctl's scale (1.0 = 100%, cube root of channelVolumes).
    pub volume: f32,
    pub mute: bool,
}

impl Sink {
    pub fn is_hdmi(&self) -> bool {
        self.name.contains("hdmi")
    }
}

impl std::fmt::Display for Sink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.description)
    }
}

/// A capture node (microphone, line in).
pub type Source = Sink;

/// An application's playback stream.
#[derive(Debug, Clone, PartialEq)]
pub struct Stream {
    pub id: u64,
    pub app: String,
    pub media: String,
    pub volume: f32,
    pub mute: bool,
    /// Node id of the sink this stream is linked to right now.
    pub sink_id: Option<u64>,
}

impl Stream {
    pub fn label(&self) -> String {
        if self.media.is_empty() || self.media == "Playback" { self.app.clone() } else { format!("{} — {}", self.app, self.media) }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub index: u32,
    pub name: String,
    pub description: String,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub index: u32,
    pub name: String,
    pub description: String,
    pub available: bool,
    pub profiles: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub id: u64,
    pub name: String,
    pub description: String,
    /// `alsa.card` index → `/proc/asound/card<N>`.
    pub alsa_card: Option<u32>,
    /// Name of the active profile.
    pub profile: String,
    pub profiles: Vec<Profile>,
    pub routes: Vec<Route>,
}

impl Card {
    pub fn profile_named(&self, name: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.name == name)
    }
}

/// One physical HDMI/DP audio port of a card, with the monitor plugged into it.
#[derive(Debug, Clone, PartialEq)]
pub struct HdmiPort {
    pub card_id: u64,
    pub card_name: String,
    pub route_index: u32,
    pub route_description: String,
    /// The stereo profile that exposes this port as a sink.
    pub profile_index: u32,
    pub profile_name: String,
    /// `monitor_name` from the ELD, e.g. "TOSHIBA-TV", if a display is present.
    pub monitor: Option<String>,
}

impl HdmiPort {
    /// node.name the sink will have once the profile is active.
    pub fn sink_name(&self) -> String {
        format!("alsa_output.{}.{}", self.card_name.trim_start_matches("alsa_card."), self.profile_name.trim_start_matches("output:"))
    }

    /// Case-insensitive match of the ELD monitor name against a Hyprland
    /// monitor description ("Toshiba America Info Systems Inc TOSHIBA-TV 0x1").
    pub fn matches_monitor(&self, description: &str) -> bool {
        match &self.monitor {
            Some(m) if !m.is_empty() => description.to_lowercase().contains(&m.to_lowercase()),
            _ => false,
        }
    }
}

/// Something the user can pick as the default output: an existing sink, or an
/// HDMI port whose sink only appears once its profile is selected.
#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    Sink(Sink),
    Hdmi(HdmiPort),
}

impl Output {
    pub fn label(&self) -> String {
        match self {
            Output::Sink(s) => s.description.clone(),
            Output::Hdmi(h) => match &h.monitor {
                Some(m) => format!("{} → {}", h.route_description, m),
                None => format!("{} (no display)", h.route_description),
            },
        }
    }
    /// node.name this output resolves to.
    pub fn sink_name(&self) -> String {
        match self {
            Output::Sink(s) => s.name.clone(),
            Output::Hdmi(h) => h.sink_name(),
        }
    }
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub sinks: Vec<Sink>,
    pub sources: Vec<Source>,
    pub streams: Vec<Stream>,
    pub cards: Vec<Card>,
    pub default_sink: String,
    pub default_source: String,
}

impl Snapshot {
    pub fn sink_by_name(&self, name: &str) -> Option<&Sink> {
        self.sinks.iter().find(|s| s.name == name)
    }

    pub fn card_by_id(&self, id: u64) -> Option<&Card> {
        self.cards.iter().find(|c| c.id == id)
    }

    /// HDMI/DP ports of every card that has them, with ELD monitor names.
    pub fn hdmi_ports(&self) -> Vec<HdmiPort> {
        self.cards.iter().flat_map(|c| hdmi_ports_of(c, &c.alsa_card.map(eld_monitors).unwrap_or_default())).collect()
    }

    /// The HDMI port whose ELD monitor name appears in `description`.
    pub fn hdmi_port_for_monitor(&self, description: &str) -> Option<HdmiPort> {
        self.hdmi_ports().into_iter().find(|p| p.matches_monitor(description))
    }

    /// Picker list: non-HDMI sinks as-is; HDMI sinks replaced by their ports
    /// (one entry per port with a display), so the inactive ports show too.
    pub fn outputs(&self) -> Vec<Output> {
        let ports = self.hdmi_ports();
        let mut v: Vec<Output> = self.sinks.iter().filter(|s| !s.is_hdmi()).cloned().map(Output::Sink).collect();
        v.extend(ports.into_iter().filter(|p| p.monitor.is_some()).map(Output::Hdmi));
        v
    }

    /// Which [`Output`] the current default sink corresponds to.
    pub fn default_output(&self) -> Option<Output> {
        let cur = self.sink_by_name(&self.default_sink)?;
        if cur.is_hdmi() {
            self.hdmi_ports().into_iter().find(|p| p.sink_name() == cur.name).map(Output::Hdmi)
        } else {
            Some(Output::Sink(cur.clone()))
        }
    }
}

// ---------------------------------------------------------------- parsing

fn prop<'a>(o: &'a Value, key: &str) -> Option<&'a Value> {
    o.get("info")?.get("props")?.get(key)
}
fn prop_str(o: &Value, key: &str) -> String {
    prop(o, key).and_then(Value::as_str).unwrap_or("").to_string()
}
fn media_class(o: &Value) -> String {
    prop_str(o, "media.class")
}

/// (volume on wpctl's scale, mute) from a node's `Props` param.
fn node_volume(o: &Value) -> (f32, bool) {
    let props = o.get("info").and_then(|i| i.get("params")).and_then(|p| p.get("Props")).and_then(|p| p.get(0));
    let vol = props
        .and_then(|p| p.get("channelVolumes"))
        .and_then(Value::as_array)
        .and_then(|a| a.iter().filter_map(Value::as_f64).fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v)))))
        .map(|v| v.cbrt() as f32)
        .unwrap_or(1.0);
    let mute = props.and_then(|p| p.get("mute")).and_then(Value::as_bool).unwrap_or(false);
    (vol, mute)
}

fn parse_node(o: &Value) -> Option<Sink> {
    let (volume, mute) = node_volume(o);
    Some(Sink {
        id: o.get("id")?.as_u64()?,
        name: prop(o, "node.name")?.as_str()?.to_string(),
        description: prop_str(o, "node.description"),
        device_id: prop(o, "device.id").and_then(Value::as_u64),
        volume,
        mute,
    })
}

fn parse_card(o: &Value) -> Option<Card> {
    let params = o.get("info")?.get("params")?;
    let profiles = params
        .get("EnumProfile")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    Some(Profile {
                        index: p.get("index")?.as_u64()? as u32,
                        name: p.get("name")?.as_str()?.to_string(),
                        description: p.get("description").and_then(Value::as_str).unwrap_or("").to_string(),
                        available: p.get("available").and_then(Value::as_str) != Some("no"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let routes = params
        .get("EnumRoute")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    Some(Route {
                        index: r.get("index")?.as_u64()? as u32,
                        name: r.get("name")?.as_str()?.to_string(),
                        description: r.get("description").and_then(Value::as_str).unwrap_or("").to_string(),
                        available: r.get("available").and_then(Value::as_str) != Some("no"),
                        profiles: r.get("profiles").and_then(Value::as_array).map(|p| p.iter().filter_map(Value::as_u64).map(|x| x as u32).collect()).unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    // The ACTIVE profile is the `Profile` param; the `device.profile` prop is
    // whatever WirePlumber picked at boot and goes stale after `set-profile`.
    let active = params
        .get("Profile")
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find_map(|p| p.get("name").and_then(Value::as_str)))
        .map(str::to_string)
        .unwrap_or_else(|| prop_str(o, "device.profile"));
    Some(Card {
        id: o.get("id")?.as_u64()?,
        name: prop(o, "device.name")?.as_str()?.to_string(),
        description: prop_str(o, "device.description"),
        alsa_card: prop(o, "alsa.card").and_then(Value::as_u64).map(|x| x as u32),
        profile: active,
        profiles,
        routes,
    })
}

/// Parse a full `pw-dump` document.
pub fn parse_snapshot(json: &str) -> Result<Snapshot> {
    let objs: Vec<Value> = serde_json::from_str(json).context("parsing pw-dump")?;
    let mut snap = Snapshot::default();
    let mut links: Vec<(u64, u64)> = vec![]; // (output node, input node)
    let mut streams_raw: Vec<Value> = vec![];
    for o in &objs {
        // Objects without a type (older fixtures) are treated as nodes.
        match o.get("type").and_then(Value::as_str).unwrap_or("PipeWire:Interface:Node") {
            "PipeWire:Interface:Node" => match media_class(o).as_str() {
                "Audio/Sink" => snap.sinks.extend(parse_node(o)),
                "Audio/Source" => snap.sources.extend(parse_node(o)),
                "Stream/Output/Audio" => streams_raw.push(o.clone()),
                _ => {}
            },
            "PipeWire:Interface:Device" => {
                if media_class(o) == "Audio/Device" { snap.cards.extend(parse_card(o)); }
            }
            "PipeWire:Interface:Link" => {
                if let (Some(a), Some(b)) = (o.pointer("/info/output-node-id").and_then(Value::as_u64), o.pointer("/info/input-node-id").and_then(Value::as_u64)) {
                    links.push((a, b));
                }
            }
            "PipeWire:Interface:Metadata" => {
                if o.pointer("/props/metadata.name").and_then(Value::as_str) == Some("default") {
                    for m in o.get("metadata").and_then(Value::as_array).into_iter().flatten() {
                        let name = m.pointer("/value/name").and_then(Value::as_str).unwrap_or("").to_string();
                        match m.get("key").and_then(Value::as_str) {
                            Some("default.audio.sink") => snap.default_sink = name,
                            Some("default.audio.source") => snap.default_source = name,
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }
    for o in &streams_raw {
        let Some(id) = o.get("id").and_then(Value::as_u64) else { continue };
        let (volume, mute) = node_volume(o);
        let sink_id = links.iter().find(|(out, inp)| *out == id && snap.sinks.iter().any(|s| s.id == *inp)).map(|(_, inp)| *inp);
        snap.streams.push(Stream { id, app: prop_str(o, "application.name"), media: prop_str(o, "media.name"), volume, mute, sink_id });
    }
    Ok(snap)
}

/// Back-compat: just the sinks.
pub fn parse_sinks(json: &str) -> Result<Vec<Sink>> {
    Ok(parse_snapshot(json)?.sinks)
}

// ---------------------------------------------------------------- ELD

/// The card's ELD pins: which report a monitor, and how many pins there are.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Elds {
    /// `(pin index, monitor_name)` for every pin with a monitor present.
    pub present: Vec<(u32, String)>,
    /// Number of `eld#*` files (pins × MST dev-ids).
    pub total: u32,
}

/// Read `/proc/asound/card<N>/eld#*`.
pub fn eld_monitors(alsa_card: u32) -> Elds {
    eld_monitors_in(Path::new(&format!("/proc/asound/card{alsa_card}")))
}

pub fn eld_monitors_in(dir: &Path) -> Elds {
    let mut elds = Elds::default();
    for e in std::fs::read_dir(dir).into_iter().flatten().filter_map(|e| e.ok()) {
        let fname = e.file_name().to_string_lossy().to_string();
        let Some(idx) = fname.strip_prefix("eld#").and_then(|r| r.rsplit('.').next()).and_then(|s| s.parse::<u32>().ok()) else { continue };
        elds.total += 1;
        if let Some(name) = std::fs::read_to_string(e.path()).ok().and_then(|b| parse_eld(&b)) {
            elds.present.push((idx, name));
        }
    }
    elds.present.sort();
    elds
}

/// monitor_name if the ELD says a monitor is present.
pub fn parse_eld(body: &str) -> Option<String> {
    let mut present = false;
    let mut name = None;
    for l in body.lines() {
        let mut it = l.splitn(2, '\t');
        let (k, v) = (it.next()?.trim(), it.next().unwrap_or("").trim());
        match k {
            "monitor_present" => present = v == "1",
            "monitor_name" => name = Some(v.to_string()),
            _ => {}
        }
    }
    if present { Some(name.unwrap_or_default()) } else { None }
}

/// Join a card's `hdmi-output-N` routes with its ELD pins. The HDA driver
/// creates `eld#c.i` per (pin, MST dev-id), so with 16 ELDs and 4 routes pin
/// 8 belongs to route 2: `i / (elds / routes)`.
pub fn hdmi_ports_of(card: &Card, elds: &Elds) -> Vec<HdmiPort> {
    let routes: Vec<&Route> = card.routes.iter().filter(|r| r.name.starts_with("hdmi-output")).collect();
    if routes.is_empty() { return vec![]; }
    let per_route = (elds.total / routes.len() as u32).max(1);
    routes
        .iter()
        .filter(|r| r.available)
        .filter_map(|r| {
            let prof = r
                .profiles
                .iter()
                .filter_map(|i| card.profiles.iter().find(|p| p.index == *i))
                .find(|p| p.name.contains("stereo"))
                .or_else(|| r.profiles.first().and_then(|i| card.profiles.iter().find(|p| p.index == *i)))?;
            let monitor = elds.present.iter().find(|(i, _)| i / per_route == r.index).map(|(_, n)| n.clone());
            Some(HdmiPort {
                card_id: card.id,
                card_name: card.name.clone(),
                route_index: r.index,
                route_description: r.description.clone(),
                profile_index: prof.index,
                profile_name: prof.name.clone(),
                monitor,
            })
        })
        .collect()
}

// ---------------------------------------------------------------- live

fn out(cmd: &str, args: &[&str]) -> Result<String> {
    let o = Command::new(cmd).args(args).output().with_context(|| format!("running {cmd}"))?;
    if !o.status.success() {
        return Err(anyhow!("{cmd} {} failed: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&o.stdout).to_string())
}

pub fn snapshot() -> Result<Snapshot> {
    parse_snapshot(&out("pw-dump", &[])?)
}

pub fn sinks() -> Result<Vec<Sink>> {
    Ok(snapshot()?.sinks)
}

/// node.name of the current default sink.
pub fn default_sink() -> Result<String> {
    let s = snapshot()?.default_sink;
    if s.is_empty() { Err(anyhow!("no default sink")) } else { Ok(s) }
}

/// `wpctl set-default` on a node id (sink or source).
pub fn set_default_node(id: u64) -> Result<()> {
    out("wpctl", &["set-default", &id.to_string()]).map(|_| ())
}

pub fn set_profile(card_id: u64, index: u32) -> Result<()> {
    out("wpctl", &["set-profile", &card_id.to_string(), &index.to_string()]).map(|_| ())
}

/// Wait for a sink with this node.name to appear (profile switches take ~0.5s).
fn wait_for_sink(name: &str, timeout: Duration) -> Result<Sink> {
    let t0 = Instant::now();
    loop {
        if let Some(s) = snapshot()?.sinks.into_iter().find(|s| s.name == name) { return Ok(s); }
        if t0.elapsed() > timeout { return Err(anyhow!("sink {name} did not appear")); }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Make `name` the default sink. An HDMI sink that isn't there yet (its
/// profile is off or on another port) is brought up first by switching the
/// card's profile — this is what lets saved profiles name the TV's sink.
pub fn set_default(name: &str) -> Result<()> {
    let snap = snapshot()?;
    if let Some(s) = snap.sink_by_name(name) { return set_default_node(s.id); }
    if let Some(port) = snap.hdmi_ports().into_iter().find(|p| p.sink_name() == name) {
        return select_output(&Output::Hdmi(port)).map(|_| ());
    }
    Err(anyhow!("sink {name} not found"))
}

/// Make an [`Output`] the default sink; returns its node.name.
pub fn select_output(o: &Output) -> Result<String> {
    match o {
        Output::Sink(s) => { set_default_node(s.id)?; Ok(s.name.clone()) }
        Output::Hdmi(h) => {
            let snap = snapshot()?;
            let active = snap.card_by_id(h.card_id).map(|c| c.profile == h.profile_name).unwrap_or(false);
            if !active { set_profile(h.card_id, h.profile_index)?; }
            let name = h.sink_name();
            let s = wait_for_sink(&name, Duration::from_secs(4))?;
            set_default_node(s.id)?;
            Ok(name)
        }
    }
}

/// Route the default output to the HDMI port a monitor is plugged into.
/// `monitor` is a Hyprland description (or any substring the ELD name appears in).
pub fn route_to_monitor(monitor_description: &str) -> Result<String> {
    let snap = snapshot()?;
    let port = snap.hdmi_port_for_monitor(monitor_description).ok_or_else(|| anyhow!("no HDMI audio port reports a display matching '{monitor_description}'"))?;
    select_output(&Output::Hdmi(port))
}

pub fn set_volume(id: u64, v: f32) -> Result<()> {
    out("wpctl", &["set-volume", &id.to_string(), &format!("{:.2}", v.clamp(0.0, 1.5))]).map(|_| ())
}

pub fn set_mute(id: u64, mute: bool) -> Result<()> {
    out("wpctl", &["set-mute", &id.to_string(), if mute { "1" } else { "0" }]).map(|_| ())
}

/// Move a playback stream to a sink (WirePlumber honours `target.object` in
/// the default metadata; node.name is stable across reconnects, ids are not).
pub fn move_stream(stream_id: u64, sink_name: &str) -> Result<()> {
    out("pw-metadata", &[&stream_id.to_string(), "target.object", sink_name]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sink_fixture() {
        let s = parse_sinks(include_str!("../tests/fixtures/pw_sinks.json")).unwrap();
        assert_eq!(s.len(), 5);
        let hdmi = s.iter().find(|s| s.is_hdmi()).unwrap();
        assert_eq!(hdmi.id, 58);
        assert!(hdmi.description.contains("HDMI"));
    }

    #[test]
    fn parses_full_dump() {
        let snap = parse_snapshot(include_str!("../tests/fixtures/pw_dump.json")).unwrap();
        assert_eq!(snap.default_sink, "alsa_output.usb-Generic_USB_Audio-00.HiFi__SPDIF__sink");
        let card = snap.cards.iter().find(|c| c.name == "alsa_card.pci-0000_01_00.1").unwrap();
        assert_eq!(card.alsa_card, Some(0));
        assert_eq!(card.profile, "output:hdmi-stereo-extra2", "active profile comes from the Profile param, not the stale prop");
        assert_eq!(card.routes.iter().filter(|r| r.available).count(), 3);
        let chrome = snap.streams.iter().find(|s| s.app == "Google Chrome").unwrap();
        assert_eq!(chrome.sink_id, Some(59));
        let tv = snap.sink_by_name("alsa_output.pci-0000_01_00.1.hdmi-stereo-extra2").unwrap();
        assert!((tv.volume - 1.0).abs() < 0.01 && !tv.mute);
        assert_eq!(snap.sources.len(), 4);
    }

    #[test]
    fn eld_pins_map_to_routes() {
        let snap = parse_snapshot(include_str!("../tests/fixtures/pw_dump.json")).unwrap();
        let card = snap.cards.iter().find(|c| c.name == "alsa_card.pci-0000_01_00.1").unwrap();
        let elds = eld_monitors_in(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/eld")));
        assert_eq!(elds.present, vec![(0, "VY279HGR".into()), (4, "ASUS VG27V".into()), (8, "TOSHIBA-TV".into())]);
        assert_eq!(elds.total, 16);
        let ports = hdmi_ports_of(card, &elds);
        assert_eq!(ports.len(), 3, "only available routes");
        let tv = ports.iter().find(|p| p.monitor.as_deref() == Some("TOSHIBA-TV")).unwrap();
        assert_eq!(tv.route_index, 2);
        assert_eq!(tv.profile_name, "output:hdmi-stereo-extra2");
        assert_eq!(tv.sink_name(), "alsa_output.pci-0000_01_00.1.hdmi-stereo-extra2");
        assert!(tv.matches_monitor("Toshiba America Info Systems Inc TOSHIBA-TV 0x00000001"));
        assert!(!tv.matches_monitor("ASUSTek COMPUTER INC VY279HGR TCLMTR040596"));
        let asus = ports.iter().find(|p| p.route_index == 0).unwrap();
        assert_eq!(asus.monitor.as_deref(), Some("VY279HGR"));
        assert_eq!(asus.profile_name, "output:hdmi-stereo");
    }

    #[test]
    fn eld_parse() {
        assert_eq!(parse_eld("monitor_present\t\t1\neld_valid\t\t1\nmonitor_name\t\tTOSHIBA-TV\n"), Some("TOSHIBA-TV".into()));
        assert_eq!(parse_eld("monitor_present\t\t0\neld_valid\t\t0\n"), None);
    }
}

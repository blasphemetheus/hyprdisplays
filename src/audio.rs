//! PipeWire sinks via `pw-dump` / `wpctl` (there is no pactl on this box).

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct Sink {
    pub id: u64,
    pub name: String,
    pub description: String,
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

#[derive(Deserialize)]
struct Node {
    id: u64,
    #[serde(default)]
    info: Option<Info>,
}
#[derive(Deserialize)]
struct Info {
    #[serde(default)]
    props: serde_json::Map<String, serde_json::Value>,
}

pub fn parse_sinks(json: &str) -> Result<Vec<Sink>> {
    let nodes: Vec<Node> = serde_json::from_str(json).context("parsing pw-dump")?;
    Ok(nodes
        .into_iter()
        .filter_map(|n| {
            let p = n.info?.props;
            if p.get("media.class")?.as_str()? != "Audio/Sink" { return None; }
            Some(Sink {
                id: n.id,
                name: p.get("node.name")?.as_str()?.to_string(),
                description: p.get("node.description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            })
        })
        .collect())
}

pub fn sinks() -> Result<Vec<Sink>> {
    let out = Command::new("pw-dump").output().context("running pw-dump")?;
    parse_sinks(&String::from_utf8_lossy(&out.stdout))
}

/// node.name of the current default sink.
pub fn default_sink() -> Result<String> {
    let out = Command::new("wpctl").args(["inspect", "@DEFAULT_AUDIO_SINK@"]).output().context("wpctl inspect")?;
    let s = String::from_utf8_lossy(&out.stdout);
    s.lines()
        .find_map(|l| {
            let l = l.trim();
            l.strip_prefix("node.name = \"").and_then(|r| r.strip_suffix('"')).map(str::to_string)
        })
        .ok_or_else(|| anyhow!("no default sink"))
}

pub fn set_default(name: &str) -> Result<()> {
    let id = sinks()?.into_iter().find(|s| s.name == name).map(|s| s.id).ok_or_else(|| anyhow!("sink {name} not found"))?;
    let st = Command::new("wpctl").args(["set-default", &id.to_string()]).status().context("wpctl set-default")?;
    if st.success() { Ok(()) } else { Err(anyhow!("wpctl set-default failed")) }
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
}

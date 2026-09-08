//! Named layouts on disk: ~/.config/hyprdisplays/profiles/<name>.json

use crate::{audio, hypr, model::Layout};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub name: String,
    pub layout: Layout,
    /// node.name of the default sink to select with this profile.
    pub sink: Option<String>,
}

pub fn dir() -> PathBuf {
    dirs::config_dir().unwrap_or_default().join("hyprdisplays/profiles")
}

fn path(name: &str) -> PathBuf {
    dir().join(format!("{}.json", sanitize(name)))
}

pub fn sanitize(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

pub fn list() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir())
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| e.path().file_stem().map(|s| s.to_string_lossy().to_string()))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

pub fn save(p: &Profile) -> Result<()> {
    std::fs::create_dir_all(dir())?;
    std::fs::write(path(&p.name), serde_json::to_string_pretty(p)?).context("writing profile")
}

pub fn load(name: &str) -> Result<Profile> {
    let s = std::fs::read_to_string(path(name)).with_context(|| format!("profile {name} not found"))?;
    serde_json::from_str(&s).context("parsing profile")
}

pub fn delete(name: &str) -> Result<()> {
    std::fs::remove_file(path(name)).context("deleting profile")
}

/// Apply every rule (only for monitors that are currently connected, matched by
/// description) and the audio sink. Returns a human summary.
pub fn apply(p: &Profile) -> Result<String> {
    let live = hypr::monitors()?;
    let mut n = 0;
    for m in &p.layout.monitors {
        if live.iter().any(|l| l.description == m.description) {
            hypr::apply(m)?;
            n += 1;
        }
    }
    if let Some(s) = &p.sink {
        audio::set_default(s)?;
    }
    Ok(format!("profile '{}': {n} monitor rule(s) applied{}", p.name, if p.sink.is_some() { " + audio" } else { "" }))
}

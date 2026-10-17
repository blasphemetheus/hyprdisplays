//! Keybinds model: parse `lua/binds.lua` (the dotfiles' Hyprland Lua config),
//! edit binds, and write the file back touching only the lines that changed.
//!
//! The file is treated line by line: every `bind(...)` call that fits on one
//! line is a [`Bind`]; everything else (comments, locals, loops, submaps) is
//! kept verbatim. Binds inside `for`/`function`/`define_submap` blocks are
//! shown but not editable, since their key expression is generated.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Scripts directory the file abbreviates as `S`.
pub const SCRIPTS: &str = "~/.config/hypr/scripts";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Flags {
    pub repeating: bool,
    pub locked: bool,
    pub release: bool,
    pub drag: bool,
}

impl Flags {
    pub fn lua(&self) -> Option<String> {
        let mut parts = vec![];
        if self.repeating { parts.push("repeating = true"); }
        if self.locked { parts.push("locked = true"); }
        if self.release { parts.push("release = true"); }
        if self.drag { parts.push("drag = true"); }
        if parts.is_empty() { None } else { Some(format!("{{ {} }}", parts.join(", "))) }
    }
    pub fn summary(&self) -> String {
        let mut p = vec![];
        if self.repeating { p.push("repeat"); }
        if self.locked { p.push("locked"); }
        if self.release { p.push("release"); }
        if self.drag { p.push("drag"); }
        p.join(" · ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// `exec("…")` — a shell command.
    Exec(String),
    /// Any other Lua expression (`hl.dsp.window.close()`, a function, …).
    Lua(String),
}

impl Action {
    pub fn is_exec(&self) -> bool { matches!(self, Action::Exec(_)) }
    pub fn text(&self) -> &str {
        match self { Action::Exec(s) | Action::Lua(s) => s }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bind {
    /// 0-based line in the file; `None` for binds added in this session.
    pub line: Option<usize>,
    pub section: String,
    /// Resolved key combo ("SUPER + SHIFT + M"), or the raw Lua expression
    /// when it can't be resolved (loop variables).
    pub keys: String,
    pub keys_resolved: bool,
    pub action: Action,
    pub flags: Flags,
    /// Trailing `-- comment` text, without the dashes.
    pub comment: String,
    pub editable: bool,
    /// Name of the submap the bind lives in, if any.
    pub submap: Option<String>,
}

impl Bind {
    pub fn new(section: &str) -> Self {
        Bind {
            line: None,
            section: section.to_string(),
            keys: "SUPER + ".into(),
            keys_resolved: true,
            action: Action::Exec(String::new()),
            flags: Flags::default(),
            comment: String::new(),
            editable: true,
            submap: None,
        }
    }

    /// Canonical form for conflict detection: sorted modifiers + lowercase key.
    pub fn key_id(&self) -> String {
        canonical_keys(&self.keys)
    }

    /// Short human description: the comment, else the command.
    pub fn describe(&self) -> String {
        if !self.comment.is_empty() { return self.comment.clone(); }
        match &self.action {
            Action::Exec(c) => c.strip_prefix(&format!("{SCRIPTS}/")).unwrap_or(c).to_string(),
            Action::Lua(l) => l.strip_prefix("hl.dsp.").unwrap_or(l).to_string(),
        }
    }

    /// Render as a `bind(...)` line in the file's house style.
    pub fn lua(&self) -> String {
        let keys = if let Some(rest) = self.keys.strip_prefix("SUPER + ") {
            format!("B .. {}", lua_string(rest))
        } else {
            lua_string(&self.keys)
        };
        let action = match &self.action {
            Action::Exec(cmd) => {
                if let Some(rest) = cmd.strip_prefix(SCRIPTS) {
                    format!("exec(S .. {})", lua_string(rest))
                } else {
                    format!("exec({})", lua_string(cmd))
                }
            }
            Action::Lua(l) => l.clone(),
        };
        let mut s = format!("bind({keys}, {action}");
        if let Some(opts) = self.flags.lua() { s.push_str(&format!(", {opts}")); }
        s.push(')');
        if !self.comment.is_empty() { s.push_str(&format!("   -- {}", self.comment)); }
        s
    }
}

/// Normalise "SUPER + SHIFT + m" → "ctrl+shift+super+m" style id.
pub fn canonical_keys(keys: &str) -> String {
    let mut mods: Vec<String> = vec![];
    let mut key = String::new();
    for part in keys.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_uppercase().as_str() {
            "SUPER" | "MOD4" | "WIN" | "LOGO" => mods.push("super".into()),
            "SHIFT" => mods.push("shift".into()),
            "CTRL" | "CONTROL" => mods.push("ctrl".into()),
            "ALT" | "MOD1" => mods.push("alt".into()),
            _ => key = part.to_ascii_lowercase(),
        }
    }
    mods.sort();
    mods.push(key);
    mods.join("+")
}

/// Quote a string as a Lua literal: `"…"` when it contains no quotes or
/// backslashes, otherwise a long bracket string.
pub fn lua_string(s: &str) -> String {
    if !s.contains('"') && !s.contains('\\') && !s.contains('\n') {
        return format!("\"{s}\"");
    }
    let mut level = 0;
    while s.contains(&format!("]{}]", "=".repeat(level))) { level += 1; }
    let eq = "=".repeat(level);
    format!("[{eq}[{s}]{eq}]")
}

// ── Lua scanning helpers ────────────────────────────────────────────────

/// Split the text of a call's argument list at top-level commas.
fn split_args(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut depth = 0i32;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' || c == '\'' {
            let end = skip_quoted(&chars, i);
            cur.extend(&chars[i..end]);
            i = end;
            continue;
        }
        if c == '[' {
            if let Some(end) = long_string_end(&chars, i) {
                cur.extend(&chars[i..end]);
                i = end;
                continue;
            }
        }
        match c {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => depth -= 1,
            ',' if depth == 0 => { out.push(cur.trim().to_string()); cur.clear(); i += 1; continue; }
            _ => {}
        }
        cur.push(c);
        i += 1;
    }
    if !cur.trim().is_empty() { out.push(cur.trim().to_string()); }
    out
}

/// Index just past the closing quote of the string starting at `start`.
fn skip_quoted(chars: &[char], start: usize) -> usize {
    let q = chars[start];
    let mut i = start + 1;
    while i < chars.len() {
        if chars[i] == '\\' { i += 2; continue; }
        if chars[i] == q { return i + 1; }
        i += 1;
    }
    chars.len()
}

/// If a long bracket string `[=*[` starts at `start`, index just past its end.
fn long_string_end(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start + 1;
    let mut level = 0;
    while i < chars.len() && chars[i] == '=' { level += 1; i += 1; }
    if i >= chars.len() || chars[i] != '[' { return None; }
    let close: Vec<char> = format!("]{}]", "=".repeat(level)).chars().collect();
    let mut j = i + 1;
    while j + close.len() <= chars.len() {
        if chars[j..j + close.len()] == close[..] { return Some(j + close.len()); }
        j += 1;
    }
    None
}

/// Find the index of the `)` matching the `(` at `open`.
fn matching_paren(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' || c == '\'' { i = skip_quoted(chars, i); continue; }
        if c == '[' {
            if let Some(end) = long_string_end(chars, i) { i = end; continue; }
        }
        match c {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => { depth -= 1; if depth == 0 { return Some(i); } }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Decode a Lua string literal (short with common escapes, or long bracket).
fn lua_literal(s: &str) -> Option<String> {
    let chars: Vec<char> = s.chars().collect();
    if chars.is_empty() { return None; }
    if chars[0] == '"' || chars[0] == '\'' {
        if skip_quoted(&chars, 0) != chars.len() { return None; }
        let mut out = String::new();
        let mut i = 1;
        while i < chars.len() - 1 {
            if chars[i] == '\\' && i + 1 < chars.len() - 1 {
                out.push(match chars[i + 1] { 'n' => '\n', 't' => '\t', c => c });
                i += 2;
            } else {
                out.push(chars[i]);
                i += 1;
            }
        }
        return Some(out);
    }
    if chars[0] == '[' {
        let end = long_string_end(&chars, 0)?;
        if end != chars.len() { return None; }
        let level = chars[1..].iter().take_while(|c| **c == '=').count();
        let inner: String = chars[2 + level..chars.len() - 2 - level].iter().collect();
        return Some(inner);
    }
    None
}

/// Evaluate a `..`-concatenation of string literals and known locals.
fn eval_concat(expr: &str, locals: &HashMap<String, String>) -> Option<String> {
    let mut out = String::new();
    for part in split_concat(expr) {
        let part = part.trim();
        if let Some(lit) = lua_literal(part) { out.push_str(&lit); continue; }
        if let Some(v) = locals.get(part) { out.push_str(v); continue; }
        return None;
    }
    Some(out)
}

/// Split an expression on top-level `..`.
fn split_concat(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = vec![];
    let mut cur = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' || c == '\'' { let e = skip_quoted(&chars, i); cur.extend(&chars[i..e]); i = e; continue; }
        if c == '[' {
            if let Some(e) = long_string_end(&chars, i) { cur.extend(&chars[i..e]); i = e; continue; }
        }
        if c == '.' && i + 1 < chars.len() && chars[i + 1] == '.' {
            out.push(cur.clone()); cur.clear(); i += 2; continue;
        }
        cur.push(c);
        i += 1;
    }
    out.push(cur);
    out
}

/// Parse `{ repeating = true, locked = true }`.
fn parse_flags(s: &str) -> Option<Flags> {
    let inner = s.trim().strip_prefix('{')?.strip_suffix('}')?;
    let mut f = Flags::default();
    for kv in inner.split(',').map(str::trim).filter(|k| !k.is_empty()) {
        let (k, v) = kv.split_once('=')?;
        let on = v.trim() == "true";
        match k.trim() {
            "repeating" => f.repeating = on,
            "locked" => f.locked = on,
            "release" => f.release = on,
            "drag" => f.drag = on,
            _ => return None,
        }
    }
    Some(f)
}

// ── File model ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct BindsFile {
    pub path: PathBuf,
    lines: Vec<String>,
    pub binds: Vec<Bind>,
    /// Sections in file order.
    pub sections: Vec<String>,
}

pub fn default_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".config/hypr/lua/binds.lua")
}

impl BindsFile {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Ok(Self::parse(path, &text))
    }

    pub fn parse(path: &Path, text: &str) -> Self {
        let lines: Vec<String> = text.lines().map(String::from).collect();
        let mut locals: HashMap<String, String> = HashMap::new();
        let mut binds = vec![];
        let mut sections: Vec<String> = vec![];
        let mut section = String::from("General");
        let mut depth = 0i32;
        let mut submap: Option<String> = None;
        let mut pending_comment: Option<String> = None;
        let mut prev_blank = true;

        for (n, raw) in lines.iter().enumerate() {
            let line = raw.trim();
            // `local name = <string expr>` feeds key/command resolution.
            if let Some(rest) = line.strip_prefix("local ") {
                if let Some((name, expr)) = rest.split_once('=') {
                    let name = name.trim();
                    if name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        if let Some(v) = eval_concat(strip_comment(expr), &locals) {
                            locals.insert(name.to_string(), v);
                        }
                    }
                }
            }
            if let Some(c) = line.strip_prefix("--") {
                let c = c.trim();
                // A comment paragraph (after a blank line) above a bind or loop names
                // the section; an inline note right after another bind does not.
                if depth == 0 && prev_blank && !c.is_empty() && !c.starts_with('-') {
                    pending_comment = Some(c.to_string());
                }
                prev_blank = false;
                continue;
            }
            if line.is_empty() { pending_comment = None; prev_blank = true; continue; }
            prev_blank = false;
            if let Some(c) = pending_comment.take() {
                if depth == 0 && (line.starts_with("bind(") || line.starts_with("for ")) {
                    section = heading(&c);
                    if !sections.contains(&section) { sections.push(section.clone()); }
                }
            }
            if let Some(rest) = line.strip_prefix("hl.define_submap(") {
                let chars: Vec<char> = rest.chars().collect();
                let name = if !chars.is_empty() && chars[0] == '"' {
                    let e = skip_quoted(&chars, 0);
                    chars[1..e.saturating_sub(1)].iter().collect::<String>()
                } else { "?".into() };
                submap = Some(name);
                depth += 1;
                continue;
            }
            if line.starts_with("bind(") {
                if let Some(b) = parse_bind(raw, n, &section, &locals, depth == 0, submap.clone()) { binds.push(b); }
                continue;
            }
            if line.starts_with("for ") || line.ends_with(" do") || line.ends_with("function()") { depth += 1; continue; }
            if line == "end" || line == "end)" {
                depth -= 1;
                if depth <= 0 { depth = 0; submap = None; }
                continue;
            }
        }
        if !sections.contains(&section) && binds.iter().any(|b| b.section == section) { sections.push(section); }
        BindsFile { path: path.to_path_buf(), lines, binds, sections }
    }

    /// The file text with current binds applied: edited binds replace their
    /// original line, deleted ones vanish, new ones land at the end under a
    /// "hyprbinds" heading.
    pub fn render(&self) -> String {
        let original = BindsFile::parse(&self.path, &self.original_text());
        let orig_at = |i: usize| original.binds.iter().find(|b| b.line == Some(i));
        let mut out: Vec<Option<String>> = self.lines.iter().cloned().map(Some).collect();
        let mut seen = vec![false; self.lines.len()];
        let mut added: Vec<&Bind> = vec![];
        for b in &self.binds {
            match b.line {
                Some(i) => {
                    seen[i] = true;
                    if !b.editable { continue; }
                    // Only rewrite a line when something really changed, so the
                    // file's hand alignment and local-variable style survive.
                    let unchanged = orig_at(i)
                        .map(|o| o.keys == b.keys && o.action == b.action && o.flags == b.flags && o.comment == b.comment)
                        .unwrap_or(false);
                    if !unchanged {
                        let indent: String = self.lines[i].chars().take_while(|c| c.is_whitespace()).collect();
                        out[i] = Some(format!("{indent}{}", b.lua()));
                    }
                }
                None => added.push(b),
            }
        }
        // Deleted binds: lines that held an editable bind and are no longer referenced.
        for i in 0..self.lines.len() {
            if !seen[i] && orig_at(i).map(|b| b.editable).unwrap_or(false) { out[i] = None; }
        }
        let mut text: Vec<String> = out.into_iter().flatten().collect();
        if !added.is_empty() {
            if text.last().map(|l| !l.trim().is_empty()).unwrap_or(false) { text.push(String::new()); }
            text.push("-- Added with hyprbinds".into());
            for b in added { text.push(b.lua()); }
        }
        let mut s = text.join("\n");
        s.push('\n');
        s
    }

    pub fn is_dirty(&self) -> bool {
        self.render() != self.original_text()
    }

    pub fn original_text(&self) -> String {
        let mut s = self.lines.join("\n");
        s.push('\n');
        s
    }

    /// Write back with a `.bak` of the previous version beside it.
    pub fn save(&self) -> Result<()> {
        let text = self.render();
        let bak = self.path.with_extension("lua.bak");
        if self.path.exists() { std::fs::copy(&self.path, &bak).ok(); }
        std::fs::write(&self.path, text).with_context(|| format!("writing {}", self.path.display()))
    }

    /// Other binds sharing a key combo with `idx` (same press/release kind).
    pub fn conflicts(&self, idx: usize) -> Vec<usize> {
        let me = &self.binds[idx];
        let id = me.key_id();
        self.binds.iter().enumerate()
            .filter(|(j, b)| *j != idx && b.keys_resolved && b.submap == me.submap && b.flags.release == me.flags.release && b.key_id() == id)
            .map(|(j, _)| j)
            .collect()
    }
}

fn strip_comment(s: &str) -> &str {
    // Only strips a trailing `-- …` outside strings for simple local lines.
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' || c == '\'' { i = skip_quoted(&chars, i); continue; }
        if c == '[' { if let Some(e) = long_string_end(&chars, i) { i = e; continue; } }
        if c == '-' && i + 1 < chars.len() && chars[i + 1] == '-' {
            let byte = chars[..i].iter().map(|c| c.len_utf8()).sum();
            return s[..byte].trim();
        }
        i += 1;
    }
    s.trim()
}

fn heading(c: &str) -> String {
    // "Displays / hardware" stays; "Workspaces (hyprsplit: …)" → "Workspaces".
    let h = c.split(" (").next().unwrap_or(c).split(':').next().unwrap_or(c).trim();
    let mut s = h.to_string();
    if let Some(f) = s.get(0..1) { s = f.to_uppercase() + &s[1..]; }
    s
}

fn parse_bind(raw: &str, n: usize, section: &str, locals: &HashMap<String, String>, editable: bool, submap: Option<String>) -> Option<Bind> {
    let chars: Vec<char> = raw.chars().collect();
    let open = raw.find("bind(")? + "bind".len();
    let open_ci = raw[..open].chars().count();
    let close_ci = matching_paren(&chars, open_ci)?;
    let inner: String = chars[open_ci + 1..close_ci].iter().collect();
    let tail: String = chars[close_ci + 1..].iter().collect();
    let comment = tail.trim().strip_prefix("--").map(|c| c.trim().to_string()).unwrap_or_default();
    let args = split_args(&inner);
    if args.len() < 2 || args.len() > 3 { return None; }
    let (keys, keys_resolved) = match eval_concat(&args[0], locals) {
        Some(k) => (k, true),
        None => (args[0].clone(), false),
    };
    let action = parse_action(&args[1], locals);
    let (flags, editable) = match args.get(2) {
        None => (Flags::default(), editable),
        Some(f) => match parse_flags(f) { Some(f) => (f, editable), None => (Flags::default(), false) },
    };
    let editable = editable && keys_resolved;
    Some(Bind { line: Some(n), section: section.to_string(), keys, keys_resolved, action, flags, comment, editable, submap })
}

fn parse_action(expr: &str, locals: &HashMap<String, String>) -> Action {
    if let Some(inner) = expr.strip_prefix("exec(").and_then(|r| r.strip_suffix(')')) {
        if let Some(cmd) = eval_concat(inner, locals) { return Action::Exec(cmd); }
    }
    Action::Lua(expr.to_string())
}

/// Translate an iced physical key into Hyprland's xkb key name.
pub fn hypr_key_name(code_debug: &str) -> Option<String> {
    let c = code_debug;
    if let Some(k) = c.strip_prefix("Key") { if k.len() == 1 { return Some(k.to_string()); } }
    if let Some(d) = c.strip_prefix("Digit") { return Some(d.to_string()); }
    if let Some(d) = c.strip_prefix("Numpad") {
        return Some(match d {
            "Add" => "KP_Add".into(), "Subtract" => "KP_Subtract".into(), "Multiply" => "KP_Multiply".into(),
            "Divide" => "KP_Divide".into(), "Enter" => "KP_Enter".into(), "Decimal" => "KP_Decimal".into(),
            n => format!("KP_{n}"),
        });
    }
    if c.len() <= 3 && c.starts_with('F') && c[1..].chars().all(|x| x.is_ascii_digit()) { return Some(c.to_string()); }
    Some(match c {
        "ArrowLeft" => "left", "ArrowRight" => "right", "ArrowUp" => "up", "ArrowDown" => "down",
        "Space" => "space", "Escape" => "escape", "Tab" => "Tab", "Enter" => "Return", "Backspace" => "BackSpace",
        "Delete" => "Delete", "Insert" => "Insert", "Home" => "Home", "End" => "End",
        "PageUp" => "Page_Up", "PageDown" => "Page_Down", "PrintScreen" => "Print",
        "Period" => "period", "Comma" => "comma", "Backquote" => "grave", "Equal" => "equal", "Minus" => "minus",
        "Semicolon" => "semicolon", "Slash" => "slash", "Quote" => "apostrophe", "Backslash" => "backslash",
        "BracketLeft" => "bracketleft", "BracketRight" => "bracketright", "CapsLock" => "Caps_Lock",
        "AudioVolumeUp" => "XF86AudioRaiseVolume", "AudioVolumeDown" => "XF86AudioLowerVolume",
        "AudioVolumeMute" => "XF86AudioMute", "MediaPlayPause" => "XF86AudioPlay", "MediaPlay" => "XF86AudioPlay",
        "MediaPause" => "XF86AudioPause", "MediaTrackNext" => "XF86AudioNext", "MediaTrackPrevious" => "XF86AudioPrev",
        "MediaStop" => "XF86AudioStop", "BrightnessUp" => "XF86MonBrightnessUp", "BrightnessDown" => "XF86MonBrightnessDown",
        _ => return None,
    }.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIXTURE: &str = include_str!("../tests/fixtures/binds.lua");

    fn parsed() -> BindsFile { BindsFile::parse(Path::new("binds.lua"), FIXTURE) }

    #[test]
    fn parses_resolved_keys_and_exec() {
        let f = parsed();
        let b = f.binds.iter().find(|b| b.keys == "SUPER + SHIFT + M").unwrap();
        assert_eq!(b.action, Action::Exec("hyprdisplays".into()));
        assert!(b.editable);
        assert_eq!(b.section, "Displays / hardware");
        let lock = f.binds.iter().find(|b| b.keys == "SUPER + L").unwrap();
        assert_eq!(lock.action, Action::Exec(format!("{SCRIPTS}/lock-wrapper.sh")));
        let vol = f.binds.iter().find(|b| b.keys == "SUPER + Up").unwrap();
        assert!(vol.action.text().starts_with("wpctl set-volume -l 1"));
        assert_eq!(vol.flags, Flags { repeating: true, locked: true, ..Default::default() });
        let term = f.binds.iter().find(|b| b.keys == "SUPER + Q").unwrap();
        assert_eq!(term.action, Action::Exec("kitty".into()));
    }

    #[test]
    fn sections_come_from_comment_paragraphs_only() {
        let f = parsed();
        assert!(f.sections.contains(&"Basics".to_string()));
        assert!(f.sections.contains(&"Displays / hardware".to_string()));
        assert!(f.sections.contains(&"Dictation".to_string()), "{:?}", f.sections);
        assert!(!f.sections.contains(&"Stock Dolphin".to_string()), "{:?}", f.sections);
        let g = f.binds.iter().find(|b| b.keys == "SUPER + SHIFT + G").unwrap();
        assert_eq!(g.section, "Basics");
    }

    #[test]
    fn loop_binds_are_read_only() {
        let f = parsed();
        let lp = f.binds.iter().find(|b| !b.keys_resolved).unwrap();
        assert!(!lp.editable);
        assert!(f.binds.iter().filter(|b| !b.editable).count() >= 4);
    }

    #[test]
    fn roundtrip_is_identity() {
        let f = parsed();
        assert_eq!(f.render(), f.original_text());
        assert!(!f.is_dirty());
    }

    #[test]
    fn edit_rewrites_only_that_line() {
        let mut f = parsed();
        let i = f.binds.iter().position(|b| b.keys == "SUPER + B").unwrap();
        f.binds[i].action = Action::Exec("chromium".into());
        f.binds[i].comment = "browser".into();
        let out = f.render();
        let diff: Vec<(&str, &str)> = FIXTURE.lines().zip(out.lines()).filter(|(a, b)| a != b).collect();
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].1, r#"bind(B .. "B", exec("chromium"))   -- browser"#);
    }

    #[test]
    fn script_commands_use_the_s_local_and_long_strings_when_needed() {
        let mut b = Bind::new("x");
        b.keys = "SUPER + CTRL + K".into();
        b.action = Action::Exec(format!("{SCRIPTS}/foo.sh go"));
        assert_eq!(b.lua(), r#"bind(B .. "CTRL + K", exec(S .. "/foo.sh go"))"#);
        b.action = Action::Exec(r#"notify-send "hi""#.into());
        b.flags.release = true;
        assert_eq!(b.lua(), r#"bind(B .. "CTRL + K", exec([[notify-send "hi"]]), { release = true })"#);
        b.keys = "F13".into();
        b.action = Action::Lua("hl.dsp.window.close()".into());
        b.flags = Flags::default();
        assert_eq!(b.lua(), r#"bind("F13", hl.dsp.window.close())"#);
    }

    #[test]
    fn add_and_delete() {
        let mut f = parsed();
        let n = f.binds.len();
        let mut b = Bind::new("General");
        b.keys = "SUPER + F13".into();
        b.action = Action::Exec("true".into());
        f.binds.push(b);
        let i = f.binds.iter().position(|b| b.keys == "SUPER + B").unwrap();
        f.binds.remove(i);
        let out = f.render();
        assert!(out.ends_with("-- Added with hyprbinds\nbind(B .. \"F13\", exec(\"true\"))\n"));
        assert!(!out.contains(r#"exec("firefox")"#));
        assert_eq!(BindsFile::parse(Path::new("x"), &out).binds.len(), n);
    }

    #[test]
    fn conflicts_ignore_release_pairs() {
        let f = parsed();
        let i = f.binds.iter().position(|b| b.keys == "SUPER + SPACE" && !b.flags.release).unwrap();
        assert!(f.conflicts(i).is_empty());
        let mut g = f.clone();
        let mut b = Bind::new("x");
        b.keys = "SUPER + shift + m".into();
        g.binds.push(b);
        let j = g.binds.len() - 1;
        assert_eq!(g.conflicts(j).len(), 1);
    }

    #[test]
    fn key_names() {
        assert_eq!(hypr_key_name("KeyQ").as_deref(), Some("Q"));
        assert_eq!(hypr_key_name("Digit3").as_deref(), Some("3"));
        assert_eq!(hypr_key_name("F12").as_deref(), Some("F12"));
        assert_eq!(hypr_key_name("Slash").as_deref(), Some("slash"));
        assert_eq!(hypr_key_name("ShiftLeft"), None);
    }
}

# hyprdisplays

Display manager for Hyprland (Lua config era, ≥0.55): a small Rust/iced GUI to
arrange monitors, pick modes, mirror, route audio, and persist it all.

- **Map**: monitors at true relative scale; drag to arrange (edge snapping),
  layout applied live via `hyprctl eval "hl.monitor{…}"`.
- **Panel**: enable, mode (from `availableModes`), scale, VRR, mirror-of,
  default audio sink (PipeWire via `wpctl`/`pw-dump`).
- **Presets**: mirror the focused monitor onto the selected one + HDMI audio
  (remembers the previous sink in `~/.local/state/hypr/mirror-prev-sink`);
  HDMI wake bounce (60 Hz → back) for links that won't retrain cold.
  Rescue workspaces: re-home stray hyprsplit workspaces after a monitor unplug
  (calls the dotfiles' `HYPRSPLIT_RESCUE()` via `hyprctl eval`).
- **Persist**: "Write lua/monitors.lua" regenerates the block between
  `-- BEGIN hyprdisplays` / `-- END hyprdisplays` in `~/.config/hypr/lua/monitors.lua`
  and leaves everything else untouched; rules for displays that are unplugged
  right now are carried over. Named profiles live in
  `~/.config/hyprdisplays/profiles/*.json`.

```
hyprdisplays                      # GUI
hyprdisplays --list-profiles
hyprdisplays --save-profile desk  # current live layout + default sink
hyprdisplays --apply tv
hyprdisplays --write-lua
```

## Build

`nix develop` gives rustc/cargo/rust-analyzer plus the Wayland/Vulkan runtime
libs; `cargo run`. `nix build` produces a wrapped binary (LD_LIBRARY_PATH for
wgpu on NVIDIA via `/run/opengl-driver/lib`). Tests: `cargo test`.

Consumed from the dotfiles flake as an input; window rule + `Super+Shift+M`
bind live there.

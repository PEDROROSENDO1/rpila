# Roadmap

Pila's one roadmap: what comes next and in what order, what waits for testers or for other
projects, and what isn't planned. It changes with each release; the
[changelog](https://github.com/igalmarino/pila/blob/main/CHANGELOG.md) says what's done.

Last updated for **0.8.0** (2026-11-20). Pila is alpha and a daily driver on its author's
machine: windows, Xwayland, workspaces, snapping, rules, input methods, screen capture, desktop
shells and tools (waybar and friends), bare metal with direct scanout, all covered by
1084 tests and real-app checks. What's left is mostly hardware Pila hasn't met and protocols it
doesn't speak yet.

## Now: the next release

Small, testable in CI, no new hardware needed.

Nothing open: 0.8.0 took what was here.

## Next

A feature users ask for goes here first.

- **A frame shown late as the wallpaper first appears** (bare metal): on the 0.8.0 `scripts/check`
  of a TTY, the frame where swaybg's picture first showed took 11.6 ms of CPU and was shown a
  refresh late, once, at startup; the whole-screen buffer an app sends is uploaded as it's
  drawn. Look into uploading it before the frame that shows it.

## Waiting for hardware and testers

Pila is developed on one AMD laptop with an external monitor. These are written, or ready to be,
once someone can try them; [issue #1](https://github.com/igalmarino/pila/issues/1) is the place
to say so.

- **Monitors wired to a second GPU** (`other-gpus = true`, off by default): untried on a real
  two-GPU machine; then direct scanout on those monitors.
- **NVIDIA** beyond one GTX 1060, and hybrid laptops.
- **Touchscreens** (`wl_touch`): touches only count as activity today.
- **Drawing tablets** (`tablet-v2`).
- **VR headsets** (`wp-drm-lease-v1`): only if someone asks.

## Waiting for other projects

- **xdg-session-management** in a release of the `wayland-protocols` crate: Pila then drops the
  bindings it generates itself (`src/session_protocol.rs`).
- **Standard replacements for the wlroots protocols** Pila accepts (layer shell, screencopy,
  foreign toplevel management, gamma, output management and power, virtual keyboard, input
  method): each is swapped for its `ext-` version once one lands in wayland-protocols and
  Smithay.
- **Smithay with the changes Pila's fork carries** (a text cursor with no width, asynchronous
  page flips, the X display reserved apart from Xwayland): Pila goes back to Smithay's own
  repository.
- **Maintained dependencies of zbus and cosmic-text**: `cargo audit` reports `zbus_names`,
  `zvariant` and `zvariant_derive` (zbus 5) and `ttf-parser` (cosmic-text) unmaintained
  (RUSTSEC-2026-0370 and later; no vulnerability). Moving up with their next releases.

## Not planned

- A bar, launcher, on-screen display or screenshot tool of Pila's own: standard tools (waybar,
  fuzzel, swayosd, grim and slurp, ...) do those jobs, and Pila is the compositor alone. Pila
  draws what a compositor must or does best itself: title bars and the window switcher. Idle
  (swayidle) and the wallpaper (swaybg) are standard tools' too.
- Vendor protocols (KDE, GNOME, Hyprland, …), even when Smithay implements them; wlroots ones only
  as the protocol policy in
  [CONTRIBUTING.md](https://github.com/igalmarino/pila/blob/main/CONTRIBUTING.md) allows.
  Features with no protocol go through Pila's configuration and `pila msg`.
- A build without X11 support: `[xwayland] enabled = false` and Xwayland left uninstalled give a
  session with no X at all, and the X11 code Pila compiles in is pure Rust, with no X libraries.
- Breaking configuration or `pila msg`: keys, actions and fields are only added to
  ("Compatibility" in CONTRIBUTING.md).

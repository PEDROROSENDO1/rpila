# Roadmap

Pila's one roadmap: what comes next and in what order, what waits for testers or for other
projects, and what isn't planned. It changes with each release; the
[changelog](https://github.com/igalmarino/pila/blob/main/CHANGELOG.md) says what's done.

Last updated 2026-11-16, after **0.5.9**. Pila is alpha and a daily driver on its author's
machine: windows, Xwayland, workspaces, snapping, rules, input methods, screen capture, desktop
shells (Noctalia, Dux Shell, waybar and friends), bare metal with direct scanout, all covered by
925 tests and real-app checks. What's left is mostly hardware Pila hasn't met and protocols it
doesn't speak yet.

## Now: the next release

Small, testable in CI, no new hardware needed.

**0.6.0** is ready and waits for its author's own check on bare metal: tearing for games,
Xwayland started on demand (and stopped when idle, or turned off), X11 keyboard grabs, faded
surfaces, pointer warps, blur behind popups (an input method's candidate window too), snapping to
the top and bottom halves, and the manual's list of the desktop tools that work. The changelog's
Unreleased section has all of it.

## Next

Nothing planned beyond what waits below: a feature users ask for goes here first.

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

## Not planned

- Vendor protocols (KDE, GNOME, Hyprland, …), even when Smithay implements them; wlroots ones only
  as the protocol policy in
  [CONTRIBUTING.md](https://github.com/igalmarino/pila/blob/main/CONTRIBUTING.md) allows.
  Features with no protocol go through Pila's configuration and `pila msg`.
- A build without X11 support: `[xwayland] enabled = false` and Xwayland left uninstalled give a
  session with no X at all, and the X11 code Pila compiles in is pure Rust, with no X libraries.
- Breaking configuration or `pila msg`: keys, actions and fields are only added to
  ("Compatibility" in CONTRIBUTING.md).

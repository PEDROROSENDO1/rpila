# Roadmap

Pila's one roadmap: what comes next and in what order, what waits for testers or for other
projects, and what isn't planned. It changes with each release; the
[changelog](https://github.com/igalmarino/pila/blob/main/CHANGELOG.md) says what's done.

Last updated for **0.5.9** (2026-11-15). Pila is alpha and a daily driver on its author's
machine: windows, Xwayland, workspaces, snapping, rules, input methods, screen capture, desktop
shells (Noctalia, Dux Shell, waybar and friends), bare metal with direct scanout, all covered by
891 tests and real-app checks. What's left is mostly hardware Pila hasn't met and protocols it
doesn't speak yet.

## Now: the next release

Small, testable in CI, no new hardware needed.

1. **An input method's candidate window takes the pointer inside a popover:** in a text field
   in a popover (a GTK search field), the candidate window doesn't react to the pointer, and a
   click on it closes the popover (a known bug, its test ignored: `tests/FEATURES.md`). A grab
   of Pila's around Smithay's popup grab lets the candidate window have it.

## Next

1. **Tearing for fullscreen games (`wp-tearing-control-v1`):** a fullscreen game that asks for it
   gets its frames on screen at once instead of at the next vblank. Needs asynchronous page flips
   in Smithay's DRM code, added to Pila's Smithay fork until Smithay has them; used only for a
   fullscreen window shown by direct scanout. Measured on bare metal only.
2. **`xwayland-keyboard-grab`:** X11 apps that take the whole keyboard (virtual machines, remote
   desktops) get it, under the same rules as keyboard-shortcuts-inhibit: the binding that quits
   Pila and the lock screen still work.
3. **Smithay update:** onto Smithay's current main (xdg-decoration v2 among the changes), with
   the fork's fix rebased on it.

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
- **Smithay with the fix Pila's fork carries:** Pila goes back to Smithay's own repository.

## Not planned

- Vendor protocols (KDE, GNOME, Hyprland, …), even when Smithay implements them; wlroots ones only
  as the protocol policy in
  [CONTRIBUTING.md](https://github.com/igalmarino/pila/blob/main/CONTRIBUTING.md) allows.
  Features with no protocol go through Pila's configuration and `pila msg`.
- Breaking configuration or `pila msg`: keys, actions and fields are only added to
  ("Compatibility" in CONTRIBUTING.md).

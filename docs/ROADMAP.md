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

1. **Tests for EDID parsing** (`src/edid.rs`, the least covered file outside the bare-metal
   backend): monitor names, makers and serials come from it, and `[[output]]` sections and
   `pila-report` read them. Unit tests on sample EDIDs, broken ones included (bad checksum,
   truncated, no name descriptor).
2. **Input methods' known limits in the manual:** a click on a candidate closes a popover that
   holds the pointer, and arrow keys don't walk a GTK menu opened while the input method is on.
3. **`wp-alpha-modifier-v1`:** a client fades one of its surfaces without redrawing it (video
   players, overlays). Smithay already applies it when drawing; Pila offers the global and
   checks captures, blur and animations keep it.
4. **`wp-pointer-warp-v1`:** a client moves the pointer within its own focused surface (games,
   SDL 3). Refused while locked, as every other input path.
5. **Interactive resize sends one configure per frame**, not one per pointer motion (up to a
   thousand a second with a fast mouse), as other compositors do: heavy clients (Firefox,
   Electron) resize more smoothly.
6. **Multi-GPU: the copy between GPUs in the frame statistics,** logged apart as captures and
   direct scanout are (`src/frame_stats.rs`), so a report from a two-GPU machine shows what it
   costs ([MULTI_GPU.md](https://github.com/igalmarino/pila/blob/main/docs/MULTI_GPU.md),
   "Risks").

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

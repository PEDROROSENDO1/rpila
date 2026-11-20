<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/igalmarino/pila/main/docs/logo/pila-logo-dark.svg">
    <img src="https://raw.githubusercontent.com/igalmarino/pila/main/docs/logo/pila-logo.svg" alt="Pila" width="420">
  </picture>
</p>

# Pila

[![CI](https://github.com/igalmarino/pila/actions/workflows/ci.yml/badge.svg)](https://github.com/igalmarino/pila/actions/workflows/ci.yml)

A stacking (not tiling) Wayland compositor written in Rust on [Smithay](https://github.com/Smithay/smithay).
Windows float and overlap, with server-side title bars, like a classic desktop. Pila draws
its window switcher itself; the bar, the launcher, notifications, the wallpaper and lock
screens come from the usual Wayland tools (by default swaybg, with Pila's wallpaper, waybar,
fuzzel, mako, swayidle and swaylock;
[which ones work](https://github.com/igalmarino/pila/blob/main/docs/MANUAL.md#building-your-desktop)).

Why Pila: desktops built on layer shell (a bar, launcher and notifications such as waybar,
fuzzel and mako, or a full desktop shell) usually run on tiling compositors such as Hyprland
or sway; Pila gives them a classic desktop of overlapping windows, with subtle animations, blur and a
config applied as you save it. labwc keeps the Openbox look and its XML config, Wayfire is
built around Compiz-style effects, and COSMIC's floating mode comes with the rest of COSMIC. More in
[What sets Pila apart](https://github.com/igalmarino/pila/blob/main/docs/WHY-PILA.md).

Pila prefers **standard protocols**: Wayland core and the official `wayland-protocols`.
The wlroots protocols that panels, shells and input methods still need and that have no
standard replacement yet (layer shell, screencopy, foreign toplevel management, gamma control,
output management and power management, virtual keyboard, input method) are supported too,
each to be replaced when an `ext-` version lands.

![Pila in 35 seconds: windows opening and snapping to halves and quarters, blur behind a translucent terminal, workspaces, and the configuration applied as it's saved](https://raw.githubusercontent.com/igalmarino/pila/main/docs/demo.webp)

How to use and set it up is in the [user manual](https://github.com/igalmarino/pila/blob/main/docs/MANUAL.md) (and `man pila`).

## Status

Pila is **alpha**: usable day to day (its author runs it as their only desktop and
develops it from inside itself), but developed on one machine: an AMD laptop (Mesa, `amdgpu`)
with an external monitor, on Arch Linux with a greetd greeter, and tried from GDM. Testers
report it working on Intel integrated GPUs, on Debian with SDDM, on NixOS, from LightDM in an
Arch virtual machine, and on one NVIDIA card (a GTX 1060 with the proprietary driver, on
NixOS). Other GPUs, distros and setups are exactly what testing is needed for: please
[report](#reporting-bugs) what works and what doesn't.

Known gaps: touchscreens, drawing tablets. **Monitors wired to another GPU**
(on many laptops with two GPUs, NVIDIA ones especially, the HDMI and DisplayPort outputs are on the
dedicated GPU) stay off by default: with `other-gpus = true` in the configuration Pila draws them with the
seat's primary GPU and copies the picture over, which is new and untried. `pila-report` marks
them; a report from such a machine helps most. What comes next, and in what order, is in the
[roadmap](https://github.com/igalmarino/pila/blob/main/docs/ROADMAP.md).

## Help test Pila

The biggest unknown is how Pila behaves away from its author's laptop. Most useful right
now, in this order:

- **NVIDIA**, newer cards especially (only a GTX 1060 has tried it so far), and **Intel** or
  other AMD GPUs. On a laptop with two
  GPUs, Pila draws with the seat's primary one, often the integrated one, and with `other-gpus = true`
  in the configuration shows the monitors on the other through it, which has never been tried:
  `pila-report` says which GPU is which;
- **no systemd** (Void, Alpine, with elogind or seatd), and other **distros**, such as
  [Ubuntu and Fedora](https://github.com/igalmarino/pila/blob/main/docs/INSTALL.md#debian-ubuntu-and-fedora);
- **display managers** other than GDM, SDDM, greetd and LightDM;
- **monitors:** high refresh rates, 4K, mixed scales, several at once;
- **other shells and apps:** bars, launchers, games, Steam, Electron.

A short report helps even if everything works:

1. [Try it nested first](https://github.com/igalmarino/pila/blob/main/docs/INSTALL.md#try-it-nested-first): it changes nothing on your system.
   [Getting out](#getting-out) says how to leave a real session.
2. For your real hardware, run `pila-check` on a spare TTY (Ctrl+Alt+F3, log in), which leaves
   your session running. It starts real programs (`kitty` or `foot`; `python3`, `grim`, `xterm`
   and `fuzzel` for more scenarios) for about a minute, quits by itself and prints PASS or FAIL
   per check, with frame times and latency. Its log is `~/.local/state/pila/pila-tty.log`. It comes with
   the packages, Nix's build and `scripts/install`; in a checkout it's `scripts/check`.
3. Send a [test report](https://github.com/igalmarino/pila/issues/new?template=test_report.yml)
   with what you tried and what worked, or a [bug report](#reporting-bugs) for what didn't.

## Features

- **Windows:** server-side title bars (minimize, maximize, close) and resize borders, or
  client-side decorations for apps that draw their own; Mod+drag to move and resize;
  maximize and fullscreen; snapping to screen halves and quarters by keyboard or by dragging to
  an edge or corner; moving, resizing and centering from the keyboard; minimize; showing the
  desktop; keeping a window above the others or on all workspaces; moving windows and focus
  between monitors; switching by recent use (Mod+Tab, Alt+Tab), the windows shown as live
  previews while it's held; tapping Mod alone opens the
  launcher (fuzzel by default); workspaces spanning all monitors (Mod+1..N, Mod and the mouse wheel,
  four-finger swipes); a window menu on the title bar (right click); windows asking for
  attention marked with an amber title bar; volume, brightness, media and screenshot keys bound
  by default (wpctl, brightnessctl, playerctl, grim and slurp); the screen locked and the
  monitors off after a while without input, by default (swayidle and swaylock); a shadow under each window (`[shadow]`); optional
  rounded corners (`corner-radius`, `rounded-corners`: GTK's by default); your own colors (`[colors]`).
- **Animations**, short and subtle: windows opening, closing, minimizing, maximizing and
  snapping; launchers and notifications; workspaces sliding. Configurable, or off.
- **Window and layer rules** by app id, title or namespace: animations, workspace, size,
  position, maximized or fullscreen, always on top, on all workspaces, title bar, corner radius,
  opacity, blur and shadow per app; blur and animations per panel or launcher.
- **Monitors:** several at once, hotplugged, each with its own mode, position, rotation,
  (fractional) scale and adaptive sync; a fullscreen app's frames sent to the screen as it drew
  them (direct scanout, `direct-scanout`); each frame drawn as close to the refresh as it safely
  can be, for less latency (`render-late`).
- **Apps get their windows back:** place, size, maximized or fullscreen, for apps that ask
  (xdg-session-management), even after Pila restarts.
- **X11 apps** through Xwayland, with the clipboard shared both ways. Xwayland starts when an
  X11 app needs it and, with `[xwayland] idle-timeout`, stops when none is left.
- **Desktop tools:** layer shell (waybar, fuzzel, mako, swaybg), session lock (swaylock),
  idle notification and inhibition (swayidle, video players), clipboard managers
  (ext-data-control), screenshots (grim; ext-image-copy-capture and wlr-screencopy) and screen sharing
  (xdg-desktop-portal-wlr, for browsers and OBS), taskbars that act on windows
  (foreign-toplevel-management), night light (gamma control), screen off and monitor
  settings (output power and management), blur behind panels and windows (background-effect),
  workspace indicators and switchers (ext-workspace), a visual system bell, dialogs that stay
  above their window and modal ones that keep the focus (xdg-dialog, and X11's), window rules by tag
  (xdg-toplevel-tag), games with vsync off shown at once, torn (wp-tearing-control), surfaces an
  app fades without drawing them again (wp-alpha-modifier),
  the pointer moved by the app it's on (wp-pointer-warp: games, SDL 3), X11 apps that grab the
  keyboard (xwayland-keyboard-grab: virtual machines, remote desktops),
  Vulkan's FIFO presentation (wp-fifo, wp-commit-timing), explicit sync
  (linux-drm-syncobj), virtual keyboards (`wtype`, on-screen keyboards). Enough for a full
  desktop shell. Sandboxed apps (Flatpak) don't see the protocols that read the
  screen or act on other windows and input (wp-security-context).
- **Input:** keyboard layout, repeat and Num Lock at startup, touchpad and mouse settings (tap, natural scrolling,
  speed, disable while typing, ...), three-finger swipes to switch windows and maximize,
  four-finger swipes to switch workspaces (configurable), other touchpad gestures for apps,
  pointer lock and confinement for games, shaking the pointer to find it (`[cursor]`), any key
  bindable (media keys included, repeating while held if asked), and the mouse wheel with
  modifiers; input methods such as fcitx5 for Chinese, Japanese and Korean (input-method-v2,
  text-input-v3).
- **Configuration** in one TOML file, applied live when saved.
- **Scripting:** `pila msg` lists windows, monitors and workspaces as text or JSON and runs any
  action ([docs/IPC.md](https://github.com/igalmarino/pila/blob/main/docs/IPC.md)).

## Installing Pila

Pila runs on a TTY of any Linux machine (DRM/KMS, libinput, and a session from logind,
elogind or seatd; systemd isn't required), or nested as a window inside another Wayland or
X11 session. The safest first run is nested: from a checkout, `cargo run --release` opens it as a
window on your current desktop and changes nothing else (nested, Mod is **Alt**: Mod+Return
opens a terminal, Mod+Shift+E quits).

Every package comes in two: `pila-desktop`, Pila with the standard tools its defaults run (bar,
launcher, notifications, lock, the keys' programs), and `pila`, the compositor alone.

- **Debian, Ubuntu, Fedora:** signed APT and DNF repositories
  ([setup](https://github.com/igalmarino/pila/blob/main/docs/INSTALL.md#debian-ubuntu-and-fedora)).
- **Arch Linux:** [`pila-git`](https://aur.archlinux.org/packages/pila-git) and
  [`pila-desktop`](https://aur.archlinux.org/packages/pila-desktop) in the AUR
  ([more](https://github.com/igalmarino/pila/blob/main/docs/INSTALL.md#arch-linux-aur)).
- **Nix and NixOS:** `nix run github:igalmarino/pila`, or as a login session
  ([more](https://github.com/igalmarino/pila/blob/main/docs/INSTALL.md#nix-and-nixos)).
- **From source:** the [build dependencies](https://github.com/igalmarino/pila/blob/main/docs/INSTALL.md#building), then `scripts/install`
  ([as a login session](https://github.com/igalmarino/pila/blob/main/docs/INSTALL.md#as-a-login-session)) or `scripts/tty`
  ([on a spare TTY](https://github.com/igalmarino/pila/blob/main/docs/INSTALL.md#by-hand-on-a-tty)).

All of it, and the [programs a full session uses](https://github.com/igalmarino/pila/blob/main/docs/INSTALL.md#packages-for-a-full-session),
is in [docs/INSTALL.md](https://github.com/igalmarino/pila/blob/main/docs/INSTALL.md).

### Getting out

On a TTY or as a login session, Mod is **Super**:

- **Super+Shift+E** quits Pila (back to the login screen or the TTY).
- **Ctrl+Alt+F1..F12** switch to another virtual terminal, where you can log in and
  `pkill pila` if the screen stops responding.
- If the Pila session doesn't start, pick your usual session at the login screen and
  look at the log (see [Reporting bugs](#reporting-bugs)); `scripts/uninstall` (or
  removing the package) takes the session entry away.

## Configuring Pila

Pila reads `~/.config/pila/config.toml` (or `$XDG_CONFIG_HOME/pila/config.toml`, or the file
given with `--config`), and writes the example there the first time it's missing.
[`config.example.toml`](https://github.com/igalmarino/pila/blob/main/config.example.toml) documents
every option and the default key bindings. Saving the file applies it right away; an invalid one is
reported in a notification and the log, and Pila keeps the previous one. `pila --check-config`
checks it without starting Pila. How to set up a desktop, monitors, screen sharing, locking and
games is in the [user manual](https://github.com/igalmarino/pila/blob/main/docs/MANUAL.md).

## Reporting bugs

Open an [issue](https://github.com/igalmarino/pila/issues/new/choose) and attach
`pila-report.txt`, which `pila-report` (`scripts/report` in a checkout) writes: version, system,
GPU, monitors, whether the configuration is valid and the end of the log, without your home
directory, user name and host name. It sends nothing anywhere. Add how Pila was started (login session,
`scripts/tty` or nested) and the steps to reproduce. Where the logs are and how to make them
detailed is in the manual's [Troubleshooting](https://github.com/igalmarino/pila/blob/main/docs/MANUAL.md#troubleshooting).

Look through logs and configs before posting them: they can hold window titles, commands,
file paths and monitor details.

Questions, ideas and "does X work with Pila?" go to
[Discussions](https://github.com/igalmarino/pila/discussions).

## Use of AI

Pila is developed with AI assistance: [Claude Code](https://claude.com/claude-code) helps
with programming, testing, writing the docs and the changelog, and translating what I write
into English. The commits it worked on carry a `Co-Authored-By` trailer, and the
instructions it works under are public, in [CLAUDE.md](https://github.com/igalmarino/pila/blob/main/CLAUDE.md).

The AI doesn't make the decisions. The important ones, on technology, architecture,
security, testing and more, are mine; I'm a software developer with over 30 years of
experience. I review the code that goes in and I answer for it, and every feature has
integration tests that CI runs on every push. In other projects, I follow their rules on
AI. More about how that works, and what has gone wrong so far, in
[How Pila is made](https://github.com/igalmarino/pila/blob/main/docs/HOW-PILA-IS-MADE.md).

Contributions made with AI help are welcome on the same terms: say so in the pull request,
and send only what you understand and can answer for.

## Contributing

See [CONTRIBUTING.md](https://github.com/igalmarino/pila/blob/main/CONTRIBUTING.md) for conventions and the development environment (a
dev container in which Pila runs nested on your desktop).

## License

Pila is free software: you can redistribute it and/or modify it under the terms of the GNU
General Public License as published by the Free Software Foundation, either version 3 of
the License, or (at your option) any later version. See [LICENSE](https://github.com/igalmarino/pila/blob/main/LICENSE).

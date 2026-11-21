<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/igalmarino/pila/main/docs/logo/pila-logo-dark.svg">
    <img src="https://raw.githubusercontent.com/igalmarino/pila/main/docs/logo/pila-logo.svg" alt="Pila" width="420">
  </picture>
</p>

# Pila

[![CI](https://github.com/igalmarino/pila/actions/workflows/ci.yml/badge.svg)](https://github.com/igalmarino/pila/actions/workflows/ci.yml)

A stacking (not tiling) Wayland compositor written in Rust on [Smithay](https://github.com/Smithay/smithay).
Windows float and overlap and have title bars, like on a classic desktop.

**Who it's for:** people who like the layer-shell tools (waybar, fuzzel, mako, swaylock, or a
full desktop shell) but want overlapping windows instead of tiling. Pila draws the windows,
the window switcher and the overview. The bar, launcher, notifications, wallpaper and lock
screen are the usual Wayland tools. Configuration is one TOML file, applied when you save it.

![Pila in 35 seconds: windows opening and snapping to halves and quarters, blur behind a translucent terminal, workspaces, and the configuration applied as it's saved](https://raw.githubusercontent.com/igalmarino/pila/main/docs/demo.webp)

## Try it

The safest first run is nested, as a window on your current desktop. It changes nothing else:

```bash
cargo run --release
```

Nested, Mod is **Alt**: Mod+Return opens a terminal, Mod+Shift+E quits. On a TTY or as a login
session, Mod is **Super**, and [Getting out](https://github.com/igalmarino/pila/blob/main/docs/INSTALL.md#getting-out)
says how to leave if something goes wrong.

## Install

`pila-desktop` installs Pila with the tools its defaults use. `pila` is the compositor alone.

- **Debian, Ubuntu, Fedora:** signed APT and DNF repositories
  ([setup](https://github.com/igalmarino/pila/blob/main/docs/INSTALL.md#debian-ubuntu-and-fedora)).
- **Arch Linux:** [`pila-git`](https://aur.archlinux.org/packages/pila-git) and
  [`pila-desktop`](https://aur.archlinux.org/packages/pila-desktop) in the AUR.
- **Nix and NixOS:** `nix run github:igalmarino/pila`, or as a login session.
- **From source:** `scripts/install`.

Details for each are in [docs/INSTALL.md](https://github.com/igalmarino/pila/blob/main/docs/INSTALL.md).
Pila needs no systemd: logind, elogind or seatd will do.

## Status

Pila is **alpha**. Its author uses it as their only desktop, but it has been developed on one
machine: an AMD laptop with an external monitor, on Arch Linux. Testers report it working on
Intel GPUs, on Debian, NixOS, and on one NVIDIA card (a GTX 1060).

Known gaps: touchscreens and drawing tablets. Monitors wired to a second GPU are off by
default and have never been tried on real hardware. What comes next is in the
[roadmap](https://github.com/igalmarino/pila/blob/main/docs/ROADMAP.md).

## Help test Pila

The biggest unknown is other people's hardware: NVIDIA (newer cards especially), Intel,
laptops with two GPUs, systems without systemd, high refresh rates, 4K, mixed scales.

1. [Try it nested](#try-it) first.
2. On your real hardware, run `pila-check` on a spare TTY (Ctrl+Alt+F3, log in). It runs real
   programs for about a minute and prints PASS or FAIL per check, with frame times.
3. Send a [test report](https://github.com/igalmarino/pila/issues/new?template=test_report.yml),
   even if everything works, or a [bug report](#reporting-bugs).

## Documentation

- [User manual](https://github.com/igalmarino/pila/blob/main/docs/MANUAL.md) (also `man pila`): using Pila and setting it up.
- [`config.example.toml`](https://github.com/igalmarino/pila/blob/main/config.example.toml): every option and the default key bindings.
- [What sets Pila apart](https://github.com/igalmarino/pila/blob/main/docs/WHY-PILA.md): the full feature list, the design and how Pila compares with other compositors.
- [`pila msg`](https://github.com/igalmarino/pila/blob/main/docs/IPC.md): scripting.
- [CHANGELOG](https://github.com/igalmarino/pila/blob/main/CHANGELOG.md).

## Reporting bugs

Open an [issue](https://github.com/igalmarino/pila/issues/new/choose) and attach
`pila-report.txt`, which `pila-report` writes. It holds the version, GPU, monitors and the end of
the log, without your user name or host name, and sends nothing anywhere. Look through it before
posting. More in the manual's [Troubleshooting](https://github.com/igalmarino/pila/blob/main/docs/MANUAL.md#troubleshooting).

Questions and ideas go to [Discussions](https://github.com/igalmarino/pila/discussions).

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

See [CONTRIBUTING.md](https://github.com/igalmarino/pila/blob/main/CONTRIBUTING.md).

## License

GPL-3.0-or-later. See [LICENSE](https://github.com/igalmarino/pila/blob/main/LICENSE).

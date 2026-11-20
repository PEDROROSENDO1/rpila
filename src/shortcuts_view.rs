//! The shortcuts cheatsheet drawn by Pila itself: a panel in the middle of
//! an output listing the common bindings as they're bound now (Mod resolved,
//! a rebound key as the user wrote it, an unbound action left out). Shown on
//! the first run, when Pila has just written the config from the example
//! (the welcome screen), and by `show-shortcuts` (Mod+? by default).
//!
//! Any key that isn't a modifier, or a click, hides it: a key that's a
//! binding still runs it (Mod+Return hides it and opens a terminal), any
//! other key and a click on the panel go nowhere else (`input.rs`). Its
//! texts are drawn on a thread into one image the size of the panel
//! (`Pila::draw_shortcuts_text`), a single texture, and the panel shows once
//! it's there; all of it goes when it's hidden.

use std::{cell::RefCell, rc::Rc};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                solid::{SolidColorBuffer, SolidColorRenderElement},
                Kind,
            },
            gles::GlesRenderer,
        },
    },
    input::keyboard::{xkb, Keysym},
    reexports::calloop,
    utils::{Logical, Physical, Point, Rectangle, Scale, Size, Transform},
};

use crate::{
    config::{Action, Colors, Config, KeyCombo, ModKey, Trigger, WorkspaceTarget},
    render::PilaRenderElement,
    Pila,
};

/// Around the content, and between the heading, the rows and the notes.
const PADDING: i32 = 16;
const SECTION_GAP: i32 = 8;
/// A line of text.
const ROW: i32 = 24;
/// The keys' column, the gap after it, and what they do.
const KEYS: i32 = 200;
const COLUMN_GAP: i32 = 16;
const WHAT: i32 = 280;
/// The panel's border.
const BORDER: i32 = 1;

/// What the panel is shown as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heading {
    /// The first run: Pila just wrote the config from the example.
    Welcome,
    /// Asked for with `show-shortcuts`.
    Shortcuts,
}

impl Heading {
    fn text(self) -> &'static str {
        match self {
            Heading::Welcome => "Welcome to Pila",
            Heading::Shortcuts => "Shortcuts",
        }
    }
}

/// The cheatsheet while it's shown.
#[derive(Default)]
pub struct ShortcutsView {
    shown: Option<Shown>,
    /// Where the panel was last drawn, redrawn when it goes.
    drawn: Option<Rectangle<i32, Logical>>,
    /// The texts being drawn on a thread (`Pila::draw_shortcuts_text`).
    pending: Option<TextKey>,
    buffers: Rc<RefCell<Buffers>>,
}

struct Shown {
    heading: Heading,
    /// The output it's on, by name: the one under the pointer when it was
    /// shown (`None` before there was one: the first).
    output: Option<String>,
}

/// Kept between frames while it's shown: an unchanged panel adds no damage.
#[derive(Default)]
struct Buffers {
    background: Option<SolidColorBuffer>,
    border: Option<SolidColorBuffer>,
    heading: Option<SolidColorBuffer>,
    /// Every text of the panel, in one image the size of the panel: one
    /// texture, made on a thread (`Pila::draw_shortcuts_text`).
    text: Option<TextImage>,
}

/// A line of the panel's text: where it goes in the panel (logical, from
/// its top-left), how wide it may be, and its color (as bits, to compare).
#[derive(Debug, Clone, PartialEq)]
struct Line {
    text: String,
    at: (i32, i32),
    width: i32,
    color: [u32; 3],
}

/// What a text image shows: its lines, the panel's size and the buffer
/// scale it's drawn at.
#[derive(Debug, Clone, PartialEq)]
struct TextKey {
    lines: Vec<Line>,
    size: (i32, i32),
    scale: i32,
}

/// The panel's texts drawn into one premultiplied ARGB8888 image.
struct TextImage {
    key: TextKey,
    pixels: Vec<u8>,
    /// Made from `pixels` by the first frame that shows it.
    buffer: Option<MemoryRenderBuffer>,
}

/// The lines of a panel of `size` with `heading`, `rows` and `notes`, in
/// `colors`.
fn lines(
    heading: Heading,
    rows: &[(String, &'static str)],
    notes: &[String],
    colors: &Colors,
) -> Vec<Line> {
    let bits = |color: [f32; 3]| color.map(f32::to_bits);
    let inner = KEYS + COLUMN_GAP + WHAT;
    let mut lines = vec![Line {
        text: heading.text().into(),
        at: (PADDING, PADDING),
        width: inner,
        color: bits(colors.hover_icon),
    }];
    let mut y = PADDING + ROW + SECTION_GAP;
    for (keys, what) in rows {
        lines.push(Line {
            text: keys.clone(),
            at: (PADDING, y),
            width: KEYS,
            color: bits(colors.hover_icon),
        });
        lines.push(Line {
            text: (*what).into(),
            at: (PADDING + KEYS + COLUMN_GAP, y),
            width: WHAT,
            color: bits(colors.icon),
        });
        y += ROW;
    }
    y += SECTION_GAP;
    for note in notes {
        lines.push(Line {
            text: note.clone(),
            at: (PADDING, y),
            width: inner,
            color: bits(colors.icon_inactive),
        });
        y += ROW;
    }
    lines
}

/// `key`'s lines drawn into one image (`None` until the fonts have
/// loaded). On a thread: shaping and rasterizing them all takes a few
/// milliseconds.
fn draw_text(key: &TextKey) -> Option<Vec<u8>> {
    let scale = key.scale;
    let (width, height) = (key.size.0 * scale, key.size.1 * scale);
    let stride = width as usize * 4;
    let mut pixels = vec![0u8; stride * height as usize];
    for line in &key.lines {
        let color = line.color.map(f32::from_bits);
        let text = crate::text::render_title(&line.text, color, line.width, ROW, scale)?;
        let text_stride = text.width as usize * 4;
        let (x, y) = (line.at.0 * scale, line.at.1 * scale);
        // Clipped to the image: a line never reaches past it as laid out.
        let columns = (text.width).min(width - x).max(0) as usize * 4;
        for row in 0..(ROW * scale).min(height - y).max(0) {
            let from = row as usize * text_stride;
            let to = (y + row) as usize * stride + x as usize * 4;
            pixels[to..to + columns].copy_from_slice(&text.pixels[from..from + columns]);
        }
    }
    Some(pixels)
}

/// What `Pila::scene` gives the renderer of the panel shown.
#[derive(Clone)]
pub struct SceneShortcuts {
    /// The output it's on, by name.
    pub output: String,
    pub rect: Rectangle<i32, Logical>,
    pub heading: Heading,
    /// Each row's keys and what they do.
    pub rows: Vec<(String, &'static str)>,
    /// The lines under the rows.
    pub notes: Vec<String>,
    /// Its corners, the windows' (`corner-radius`, `rounded-corners`);
    /// `None`: square.
    pub rounding: Option<(f64, crate::rounded::Corners)>,
    buffers: Rc<RefCell<Buffers>>,
}

/// Whether a binding's action counts as a row's.
type Matches = fn(&Action) -> bool;

/// The rows the cheatsheet can show, in its order: what they do, and the
/// bindings that count as it. A row whose action has no binding is left out.
const ROWS: &[(&str, Matches)] = &[
    ("Open a terminal", |a| *a == Action::Terminal),
    ("Open the launcher", |a| *a == Action::Launcher),
    ("Switch windows", |a| *a == Action::CycleFocus),
    ("Close the window", |a| *a == Action::CloseFocused),
    ("Maximize", |a| *a == Action::ToggleMaximize),
    ("Fullscreen", |a| *a == Action::ToggleFullscreen),
    ("Minimize", |a| *a == Action::Minimize),
    ("Snap to halves and quarters", |a| {
        matches!(a, Action::SnapToward(_))
    }),
    ("Go to a workspace", |a| {
        matches!(a, Action::Workspace(WorkspaceTarget::Index(_)))
    }),
    ("Move the window there", |a| {
        matches!(a, Action::MoveToWorkspace(WorkspaceTarget::Index(_)))
    }),
    ("Show the desktop", |a| *a == Action::ShowDesktop),
    ("Show these shortcuts", |a| *a == Action::ShowShortcuts),
    ("Quit Pila", |a| *a == Action::Quit),
];

/// The two most a row names: one key and another way, at most.
const KEYS_PER_ROW: usize = 2;

/// The modifier `mod_key` is, as the cheatsheet names it.
fn mod_name(mod_key: ModKey) -> &'static str {
    match mod_key {
        ModKey::Alt => "Alt",
        ModKey::Super => "Super",
        ModKey::Ctrl => "Ctrl",
    }
}

/// A key as the cheatsheet names it: its character where it has one
/// (`slash` is "/", `d` is "D"), arrows as arrows, else its xkb name.
fn key_name(sym: Keysym) -> String {
    match sym {
        Keysym::Left => return "←".into(),
        Keysym::Right => return "→".into(),
        Keysym::Up => return "↑".into(),
        Keysym::Down => return "↓".into(),
        _ => {}
    }
    let name = xkb::keysym_get_name(sym);
    match sym.key_char() {
        Some(c) if c.is_alphabetic() => c.to_uppercase().collect(),
        Some(c) if c.is_ascii_graphic() => c.to_string(),
        _ => name,
    }
}

/// `combo` as the cheatsheet shows it: "Super+Shift+Q", "Super+/", a tap of
/// a modifier as "Super (tap)", a wheel's notch as "Super+wheel ↓".
pub fn combo_label(combo: &KeyCombo) -> String {
    let mods = combo.mods;
    if let Trigger::Key(sym) = combo.trigger {
        if crate::config::tap_sym(sym).is_some() {
            let name = if mods.logo {
                "Super"
            } else if mods.alt {
                "Alt"
            } else {
                "Ctrl"
            };
            return format!("{name} (tap)");
        }
    }
    let mut parts: Vec<String> = Vec::new();
    for (on, name) in [
        (mods.logo, "Super"),
        (mods.ctrl, "Ctrl"),
        (mods.alt, "Alt"),
        (mods.shift, "Shift"),
    ] {
        if on {
            parts.push(name.into());
        }
    }
    parts.push(match combo.trigger {
        Trigger::Key(sym) => key_name(sym),
        Trigger::Wheel(direction) => {
            use crate::config::WheelDirection;
            match direction {
                WheelDirection::Up => "wheel ↑",
                WheelDirection::Down => "wheel ↓",
                WheelDirection::Left => "wheel ←",
                WheelDirection::Right => "wheel →",
            }
            .into()
        }
    });
    parts.join("+")
}

/// Keys of a row bound to a run of numbered targets ("Super+1", "Super+2"
/// ...) as one: "Super+1…9".
fn numbered(labels: &[String]) -> Option<String> {
    let first = labels.first()?;
    let last = labels.last()?;
    let prefix = first.strip_suffix('1')?;
    let end = last.strip_prefix(prefix)?;
    let all_numbered = labels.iter().enumerate().all(|(i, label)| {
        label
            .strip_prefix(prefix)
            .is_some_and(|n| n == (i + 1).to_string())
    });
    (all_numbered && labels.len() > 1).then(|| format!("{prefix}1…{end}"))
}

/// Keys of a row that differ only by their arrow ("Super+←", "Super+→"
/// ...) as one: "Super+←→".
fn arrows(labels: &[String]) -> Option<String> {
    const ORDER: [char; 4] = ['←', '→', '↑', '↓'];
    let split = |label: &str| -> Option<(String, char)> {
        let arrow = label.chars().last().filter(|c| ORDER.contains(c))?;
        Some((label[..label.len() - arrow.len_utf8()].to_string(), arrow))
    };
    let (prefix, _) = split(labels.first()?)?;
    let mut arrows = Vec::new();
    for label in labels {
        let (other, arrow) = split(label)?;
        if other != prefix {
            return None;
        }
        arrows.push(arrow);
    }
    arrows.sort_by_key(|arrow| ORDER.iter().position(|a| a == arrow));
    let arrows: String = arrows.into_iter().collect();
    (labels.len() > 1).then(|| format!("{prefix}{arrows}"))
}

/// The cheatsheet's rows from `config`: each row's keys, as bound now, and
/// what they do. Mod+drag isn't a binding: it's `mod_key`'s.
pub fn rows(config: &Config) -> Vec<(String, &'static str)> {
    let mut rows = Vec::new();
    for (what, matches) in ROWS {
        let mut bound: Vec<&KeyCombo> = config
            .bindings
            .iter()
            .filter(|b| matches(&b.action))
            .map(|b| &b.combo)
            .collect();
        // The ones to name first: keys before a tap of a modifier, named
        // keys (Tab) before letters (J), Mod's before other modifiers'.
        bound.sort_by_key(|combo| {
            let (tap, letter) = match combo.trigger {
                Trigger::Key(sym) => (
                    crate::config::tap_sym(sym).is_some(),
                    sym.key_char().is_some_and(char::is_alphabetic),
                ),
                Trigger::Wheel(_) => (false, false),
            };
            let with_mod = match config.mod_key {
                ModKey::Alt => combo.mods.alt,
                ModKey::Super => combo.mods.logo,
                ModKey::Ctrl => combo.mods.ctrl,
            };
            (tap, letter, !with_mod)
        });
        let labels: Vec<String> = bound.iter().map(|c| combo_label(c)).collect();
        if labels.is_empty() {
            continue;
        }
        let keys = numbered(&labels)
            .or_else(|| arrows(&labels))
            .unwrap_or_else(|| {
                let shown: Vec<&str> = labels
                    .iter()
                    .take(KEYS_PER_ROW)
                    .map(String::as_str)
                    .collect();
                shown.join(", ")
            });
        rows.push((keys, *what));
        if *what == "Snap to halves and quarters" {
            let mod_name = mod_name(config.mod_key);
            rows.push((format!("{mod_name}+drag"), "Move a window"));
            rows.push((format!("{mod_name}+right drag"), "Resize it"));
        }
    }
    rows
}

/// `path` with the home directory as "~".
fn home_relative(path: &std::path::Path) -> String {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty());
    match home.and_then(|home| path.strip_prefix(home).ok().map(|p| p.to_owned())) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// The panel's size for `rows` rows and `notes` lines under them.
fn panel_size(rows: usize, notes: usize) -> Size<i32, Logical> {
    let width = 2 * PADDING + KEYS + COLUMN_GAP + WHAT;
    let height =
        2 * PADDING + ROW + SECTION_GAP + rows as i32 * ROW + SECTION_GAP + notes as i32 * ROW;
    (width, height).into()
}

impl Pila {
    /// Show the cheatsheet as `heading`, on the output under the pointer;
    /// never while locked.
    pub fn show_shortcuts(&mut self, heading: Heading) {
        if self.is_locked() {
            return;
        }
        self.redraw_shortcuts_view();
        let output = self
            .space
            .output_under(self.pointer_location())
            .next()
            .map(|output| output.name());
        self.shortcuts_view.shown = Some(Shown { heading, output });
        *self.shortcuts_view.buffers.borrow_mut() = Buffers::default();
        tracing::info!(?heading, "showing the shortcuts");
        self.redraw_shortcuts_view();
        self.draw_shortcuts_text();
    }

    /// Draw the panel's texts again if what they say changed (shown, the
    /// fonts loaded, the config reloaded): on a thread, into one image that
    /// the panel shows once it's back (it's drawn only with its text). The
    /// output's scale is the one now; a panel whose output changes scale
    /// while it's shown has its text scaled.
    pub(crate) fn draw_shortcuts_text(&mut self) {
        let Some(view) = self.scene_shortcuts() else {
            return;
        };
        let scale = self
            .space
            .outputs()
            .find(|o| o.name() == view.output)
            .map(|o| crate::scale::buffer_scale(Scale::from(o.current_scale().fractional_scale())))
            .unwrap_or(1);
        let key = TextKey {
            lines: lines(view.heading, &view.rows, &view.notes, &self.config.colors),
            size: (view.rect.size.w, view.rect.size.h),
            scale,
        };
        let drawn = view.buffers.borrow().text.as_ref().map(|t| &t.key) == Some(&key);
        if drawn || self.shortcuts_view.pending.as_ref() == Some(&key) {
            return;
        }
        self.shortcuts_view.pending = Some(key.clone());
        let (sender, channel) = calloop::channel::channel();
        let job = key.clone();
        let spawned = std::thread::Builder::new()
            .name("pila-shortcuts".into())
            .spawn(move || {
                let _ = sender.send(draw_text(&job));
            });
        if let Err(err) = spawned {
            tracing::warn!(?err, "can't draw the shortcuts' text");
            self.shortcuts_view.pending = None;
            return;
        }
        // One message, then the channel closes and its source goes.
        let inserted = self
            .loop_handle
            .insert_source(channel, move |event, _, state| {
                let calloop::channel::Event::Msg(pixels) = event else {
                    return;
                };
                if state.shortcuts_view.pending.as_ref() != Some(&key) {
                    return;
                }
                state.shortcuts_view.pending = None;
                // `None`: no fonts yet; their arrival asks again.
                let Some(pixels) = pixels else {
                    return;
                };
                state.shortcuts_view.buffers.borrow_mut().text = Some(TextImage {
                    key: key.clone(),
                    pixels,
                    buffer: None,
                });
                state.redraw_shortcuts_view();
            });
        if let Err(err) = inserted {
            tracing::warn!(?err, "can't wait for the shortcuts' text");
            self.shortcuts_view.pending = None;
        }
    }

    /// Hide the cheatsheet, if it's shown.
    pub fn hide_shortcuts(&mut self) {
        if self.shortcuts_view.shown.take().is_none() {
            return;
        }
        self.shortcuts_view.pending = None;
        *self.shortcuts_view.buffers.borrow_mut() = Buffers::default();
        self.redraw_shortcuts_view();
    }

    /// `show-shortcuts`: show it, or hide it if it's shown.
    pub fn toggle_shortcuts(&mut self) {
        if self.shortcuts_shown() {
            self.hide_shortcuts();
        } else {
            self.show_shortcuts(Heading::Shortcuts);
        }
    }

    pub fn shortcuts_shown(&self) -> bool {
        self.shortcuts_view.shown.is_some()
    }

    /// Redraw where the panel was and where it is now.
    fn redraw_shortcuts_view(&mut self) {
        if let Some(rect) = self.shortcuts_view.drawn.take() {
            self.queue_redraw_rect(rect);
        }
        if let Some((rect, ..)) = self.shortcuts_layout() {
            self.shortcuts_view.drawn = Some(rect);
            self.queue_redraw_rect(rect);
        }
    }

    /// The panel, its rows, notes and output, while it's shown.
    #[allow(clippy::type_complexity)]
    fn shortcuts_layout(
        &self,
    ) -> Option<(
        Rectangle<i32, Logical>,
        Vec<(String, &'static str)>,
        Vec<String>,
        String,
    )> {
        let shown = self.shortcuts_view.shown.as_ref()?;
        let output = shown
            .output
            .as_ref()
            .and_then(|name| self.space.outputs().find(|o| &o.name() == name))
            .or_else(|| self.space.outputs().next())?
            .clone();
        let area = self.space.output_geometry(&output)?;
        let rows = rows(&self.config);
        let mut notes = Vec::new();
        if shown.heading == Heading::Welcome {
            if let Some(path) = &self.config_path {
                notes.push(format!("Settings: {}", home_relative(path)));
            }
        }
        notes.push("Any key or click closes this".into());
        let size = panel_size(rows.len(), notes.len());
        let loc = area.loc + Point::from(((area.size.w - size.w) / 2, (area.size.h - size.h) / 2));
        Some((Rectangle::new(loc, size), rows, notes, output.name()))
    }

    /// The cheatsheet for `Pila::scene`, while it's shown.
    pub(crate) fn scene_shortcuts(&self) -> Option<SceneShortcuts> {
        let heading = self.shortcuts_view.shown.as_ref()?.heading;
        let (rect, rows, notes, output) = self.shortcuts_layout()?;
        Some(SceneShortcuts {
            output,
            rect,
            heading,
            rows,
            notes,
            rounding: (self.config.corner_radius > 0).then_some((
                f64::from(self.config.corner_radius),
                self.config.rounded_corners,
            )),
            buffers: self.shortcuts_view.buffers.clone(),
        })
    }

    /// Whether the cheatsheet is shown at `location` (global): nothing under
    /// it gets the pointer.
    pub fn shortcuts_view_covers(&self, location: Point<f64, Logical>) -> bool {
        self.shortcuts_layout()
            .is_some_and(|(rect, ..)| rect.to_f64().contains(location))
    }

    /// For the state dump: whether it's shown, as what, where and its rows.
    pub fn shortcuts_view_json(&self) -> serde_json::Value {
        match self.scene_shortcuts() {
            None => serde_json::json!({ "shown": false }),
            Some(view) => {
                let r = view.rect;
                serde_json::json!({
                    "shown": true,
                    "heading": view.heading.text(),
                    "output": view.output,
                    "rect": { "x": r.loc.x, "y": r.loc.y, "w": r.size.w, "h": r.size.h },
                    "rows": view.rows.iter().map(|(keys, what)| serde_json::json!([keys, what])).collect::<Vec<_>>(),
                    "notes": view.notes,
                })
            }
        }
    }
}

/// The cheatsheet's elements, front to back, relative to `origin`.
pub fn elements(
    renderer: &mut GlesRenderer,
    view: &SceneShortcuts,
    colors: &Colors,
    origin: Point<i32, Logical>,
    scale: Scale<f64>,
) -> Vec<PilaRenderElement> {
    let mut buffers = view.buffers.borrow_mut();
    let buffers = &mut *buffers;
    let physical = |loc: Point<i32, Logical>| -> Point<i32, Physical> {
        (loc - origin).to_physical_precise_round(scale)
    };
    let mut elements = Vec::new();
    // Drawn only with its text, which comes from a thread
    // (`Pila::draw_shortcuts_text`): never an empty panel.
    let Some(text) = buffers.text.as_mut() else {
        return elements;
    };
    let (width, height) = text.key.size;
    let text_scale = text.key.scale;
    let pixels = &text.pixels;
    let buffer = text.buffer.get_or_insert_with(|| {
        MemoryRenderBuffer::from_slice(
            pixels,
            Fourcc::Argb8888,
            (width * text_scale, height * text_scale),
            text_scale,
            Transform::Normal,
            None,
        )
    });
    let loc = (view.rect.loc - origin).to_f64().to_physical(scale);
    if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
        renderer,
        loc,
        buffer,
        None,
        None,
        None,
        Kind::Unspecified,
    ) {
        elements.push(PilaRenderElement::Memory(element));
    }
    let heading_rect = Rectangle::new(
        Point::from((view.rect.loc.x, view.rect.loc.y + PADDING)),
        Size::from((view.rect.size.w, ROW)),
    );

    // The windows' corners, if their shaders compiled; else square.
    let shaders = view
        .rounding
        .and_then(|_| crate::rounded::Shaders::get(renderer));
    let mut solid = |slot: &mut Option<SolidColorBuffer>,
                     rect: Rectangle<i32, Logical>,
                     color: [f32; 4],
                     rounding: Option<(f64, crate::rounded::Corners)>| {
        let buffer = slot.get_or_insert_with(|| SolidColorBuffer::new(rect.size, color));
        buffer.update(rect.size, color);
        let element = SolidColorRenderElement::from_buffer(
            buffer,
            physical(rect.loc),
            scale,
            1.0,
            Kind::Unspecified,
        );
        elements.push(match (rounding, &shaders) {
            (Some((radius, corners)), Some(shaders)) => PilaRenderElement::RoundedSolid(
                crate::rounded::RoundedSolid::new(element, shaders, radius, corners, scale),
            ),
            _ => PilaRenderElement::Solid(element),
        });
    };
    // Away from the corners: square.
    solid(
        &mut buffers.heading,
        heading_rect,
        crate::switcher_view::opaque(colors.snap_preview),
        None,
    );
    solid(
        &mut buffers.background,
        view.rect,
        crate::switcher_view::opaque(colors.bar_inactive),
        view.rounding,
    );
    // One pixel around it, its corners a pixel wider to follow the curve.
    let outer = Rectangle::new(
        view.rect.loc - Point::from((BORDER, BORDER)),
        view.rect.size + Size::from((2 * BORDER, 2 * BORDER)),
    );
    let [r, g, b] = colors.hover;
    solid(
        &mut buffers.border,
        outer,
        [r, g, b, 1.0],
        view.rounding
            .map(|(radius, corners)| (radius + f64::from(BORDER), corners)),
    );
    elements
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn config(text: &str, mod_key: ModKey) -> Config {
        Config::parse(text, &|_| None, mod_key).unwrap()
    }

    fn label(text: &str, mod_key: ModKey) -> String {
        let config = config(
            &format!("[bindings]\n\"{text}\" = \"show-shortcuts\"\n"),
            mod_key,
        );
        let binding = config
            .bindings
            .iter()
            .rev()
            .find(|b| b.action == Action::ShowShortcuts)
            .unwrap();
        combo_label(&binding.combo)
    }

    #[test]
    fn keys_are_named_as_they_are_pressed() {
        assert_eq!(label("Mod+Shift+q", ModKey::Super), "Super+Shift+Q");
        assert_eq!(label("Mod+Shift+q", ModKey::Alt), "Alt+Shift+Q");
        assert_eq!(
            label("Shift+Ctrl+Mod+slash", ModKey::Super),
            "Super+Ctrl+Shift+/"
        );
        assert_eq!(label("Mod+Return", ModKey::Super), "Super+Return");
        assert_eq!(label("Mod+Left", ModKey::Super), "Super+←");
        assert_eq!(label("Mod+3", ModKey::Super), "Super+3");
        assert_eq!(label("Mod", ModKey::Super), "Super (tap)");
        assert_eq!(label("Mod+WheelScrollDown", ModKey::Alt), "Alt+wheel ↓");
        assert_eq!(label("XF86AudioMute", ModKey::Super), "XF86AudioMute");
    }

    #[test]
    fn the_defaults_make_the_cheatsheet() {
        let rows = rows(&config("", ModKey::Super));
        let find = |what: &str| {
            rows.iter()
                .find(|(_, w)| *w == what)
                .map(|(keys, _)| keys.as_str())
        };
        assert_eq!(find("Open a terminal"), Some("Super+Return"));
        assert_eq!(find("Switch windows"), Some("Super+Tab, Alt+Tab"));
        assert_eq!(find("Open the launcher"), Some("Super+D, Super (tap)"));
        assert_eq!(find("Snap to halves and quarters"), Some("Super+←→↑↓"));
        assert_eq!(find("Move a window"), Some("Super+drag"));
        assert_eq!(find("Go to a workspace"), Some("Super+1…4"));
        assert_eq!(find("Move the window there"), Some("Super+Shift+1…4"));
        assert_eq!(find("Show these shortcuts"), Some("Super+Shift+/"));
        assert_eq!(find("Quit Pila"), Some("Super+Shift+E"));
    }

    #[test]
    fn the_cheatsheet_follows_the_bindings() {
        let rows = rows(&config(
            "[bindings]\n\"Mod+Return\" = \"none\"\n\"Mod+Shift+F12\" = \"quit\"\n\"Mod+Shift+E\" = \"none\"\n",
            ModKey::Alt,
        ));
        assert!(rows.iter().all(|(_, what)| *what != "Open a terminal"));
        assert!(rows.contains(&("Alt+Shift+F12".into(), "Quit Pila")));
    }

    #[test]
    fn a_run_of_numbers_is_one_range() {
        let labels = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            numbered(&labels(&["Super+1", "Super+2", "Super+3"])).as_deref(),
            Some("Super+1…3")
        );
        assert_eq!(numbered(&labels(&["Super+1"])), None);
        assert_eq!(numbered(&labels(&["Super+1", "Super+3"])), None);
        assert_eq!(numbered(&labels(&["Super+D"])), None);
    }

    #[test]
    fn the_panel_fits_its_rows() {
        let size = panel_size(10, 2);
        assert_eq!(size.w, 2 * PADDING + KEYS + COLUMN_GAP + WHAT);
        assert_eq!(size.h, 2 * PADDING + ROW + 2 * SECTION_GAP + 12 * ROW);
    }
}

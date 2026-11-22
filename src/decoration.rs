//! Server-side decorations: a title bar above the window (drag to move,
//! maximize and close buttons) and an invisible resize border around it.
//!
//! Only windows that agreed to server-side decorations get these (see
//! `XdgDecorationHandler` in `handlers/xdg_shell.rs`); fullscreen windows get
//! neither, maximized windows get no resize border. The window's position in
//! the `Space` stays its geometry origin; the title bar sits directly above it.
//!
//! Layout decisions (placement, maximize) use the *pending* decoration mode,
//! which pila decides; drawing and hit testing use the *committed* state, so a
//! bar only appears once the client has agreed to stop drawing its own.

use std::cell::RefCell;

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
    desktop::Window,
    reexports::wayland_protocols::xdg::{
        decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode,
        shell::server::xdg_toplevel,
    },
    utils::{Logical, Physical, Point, Rectangle, Scale, Transform},
};
use tracing::warn;

use crate::{
    config::Colors,
    grabs::resize_grab::ResizeEdge,
    render::PilaRenderElement,
    rounded::{self, Corners, RoundedSolid},
};

pub const TITLE_BAR_HEIGHT: i32 = 28;
/// Width of each button cell (clickable and hover-highlighted), at the right
/// end of the bar. Cells are the full bar height.
const BUTTON_WIDTH: i32 = 26;
/// Radius of each circular title bar button ("bolinha"), in logical pixels.
const BUTTON_RADIUS: f64 = 6.5;
/// Minimize, maximize and close, from left to right at the bar's right end.
const BUTTONS: i32 = 3;
/// Space between the bar's left end and the title, and between the title and
/// the buttons.
const TITLE_PADDING: i32 = 10;
/// Narrower than this, the title isn't drawn at all.
const MIN_TITLE_WIDTH: i32 = 24;
/// The widest title texture: more than any screen shows. A client can make
/// its window far wider (a subsurface stretches the geometry's bounds), and
/// the texture is width x height x 4 bytes.
const MAX_TITLE_WIDTH: i32 = 8192;
/// Width of the invisible resize border outside the window and title bar.
const BORDER: i32 = 6;
/// Distance from a corner, along each edge, that resizes diagonally.
const CORNER: i32 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleBarPart {
    Drag,
    Minimize,
    Maximize,
    Close,
}

/// A pila-drawn (or pila-handled) part of a window, as opposed to a client surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decoration {
    TitleBar(TitleBarPart),
    Border(ResizeEdge),
}

/// Whether pila has chosen server-side decorations for this window. For
/// layout; may be ahead of what the client has acked.
pub fn is_ssd(window: &Window) -> bool {
    // X11 windows get pila's decorations unless they draw their own
    // (Motif hints) or are menus/tooltips.
    if let Some(x11) = window.x11_surface() {
        return !x11.is_decorated() && !x11.is_override_redirect();
    }
    window.toplevel().is_some_and(|t| {
        t.with_pending_state(|state| state.decoration_mode == Some(DecorationMode::ServerSide))
    })
}

/// Whether a window rule hides pila's title bar (`title-bar = false`): the
/// window stays server-side decorated, so the client draws none either.
fn bar_hidden(window: &Window) -> bool {
    crate::rules::with_window_props(window, |props| props.title_bar == Some(false))
}

/// Whether a title bar is drawn: the client acked server-side decorations,
/// isn't fullscreen and no rule hides it.
pub fn has_title_bar(window: &Window) -> bool {
    if bar_hidden(window) {
        return false;
    }
    if let Some(x11) = window.x11_surface() {
        return is_ssd(window) && !x11.is_fullscreen();
    }
    window.toplevel().is_some_and(|t| {
        t.with_committed_state(|state| {
            state.is_some_and(|state| {
                state.decoration_mode == Some(DecorationMode::ServerSide)
                    && !state.states.contains(xdg_toplevel::State::Fullscreen)
            })
        })
    })
}

/// Height taken above a window at `loc` (its geometry origin) by its title bar,
/// for layout: 0 unless the window is server-side decorated, not fullscreen
/// and its bar isn't hidden by a rule.
pub fn layout_offset(window: &Window, fullscreen: bool) -> i32 {
    if is_ssd(window) && !fullscreen && !bar_hidden(window) {
        TITLE_BAR_HEIGHT
    } else {
        0
    }
}

/// The title bar of a window whose geometry origin is at `loc`, if it has one.
pub fn title_bar_rect(
    window: &Window,
    loc: Point<i32, Logical>,
) -> Option<Rectangle<i32, Logical>> {
    has_title_bar(window).then(|| {
        Rectangle::new(
            (loc.x, loc.y - TITLE_BAR_HEIGHT).into(),
            (window.geometry().size.w, TITLE_BAR_HEIGHT).into(),
        )
    })
}

/// Which part of a title bar `pos` is on. `pos` must be inside `bar`.
pub fn title_bar_part(bar: Rectangle<i32, Logical>, pos: Point<f64, Logical>) -> TitleBarPart {
    let from_right = (bar.loc.x + bar.size.w) as f64 - pos.x;
    if from_right <= BUTTON_WIDTH as f64 {
        TitleBarPart::Close
    } else if from_right <= 2.0 * BUTTON_WIDTH as f64 {
        TitleBarPart::Maximize
    } else if from_right <= 3.0 * BUTTON_WIDTH as f64 {
        TitleBarPart::Minimize
    } else {
        TitleBarPart::Drag
    }
}

/// The resize edge(s) for `pos` if it's on the invisible border around a
/// decorated, non-maximized window whose geometry origin is at `loc`.
pub fn border_edges(
    window: &Window,
    loc: Point<i32, Logical>,
    pos: Point<f64, Logical>,
) -> Option<ResizeEdge> {
    let bar = title_bar_rect(window, loc)?;
    if is_maximized(window) {
        return None;
    }
    // The window plus its title bar.
    let frame = Rectangle::new(
        bar.loc,
        (bar.size.w, bar.size.h + window.geometry().size.h).into(),
    )
    .to_f64();
    let outer = Rectangle::new(
        frame.loc - Point::from((BORDER as f64, BORDER as f64)),
        frame.size + (2.0 * BORDER as f64, 2.0 * BORDER as f64).into(),
    );
    if !outer.contains(pos) || frame.contains(pos) {
        return None;
    }

    // The band the pointer is in gives one edge; being within CORNER of a
    // corner along that band adds the other.
    Some(
        band_edges(
            pos.x,
            frame.loc.x,
            frame.size.w,
            ResizeEdge::LEFT,
            ResizeEdge::RIGHT,
        ) | band_edges(
            pos.y,
            frame.loc.y,
            frame.size.h,
            ResizeEdge::TOP,
            ResizeEdge::BOTTOM,
        ),
    )
}

/// One axis of `border_edges`: `low` if `p` is before the frame (or within
/// CORNER of its start), `high` if past it (or within CORNER of its end).
fn band_edges(p: f64, start: f64, len: f64, low: ResizeEdge, high: ResizeEdge) -> ResizeEdge {
    let end = start + len;
    let corner = CORNER as f64;
    if p < start + corner {
        low
    } else if p >= end - corner {
        high
    } else {
        ResizeEdge::empty()
    }
}

/// Which glyph a button shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Icon {
    Close,
    Minimize,
    Maximize,
}

/// A rasterized button and what it was drawn for, so it's only redrawn
/// (and re-uploaded) when the icon or its colors change.
struct ButtonTexture {
    /// Icon, circle fill color, optional hover glyph color, buffer scale and
    /// the rounded corner it's cut by (in buffer pixels; the close button's,
    /// at the bar's top-right).
    key: (Icon, [f32; 3], Option<[f32; 3]>, i32, i32),
    buffer: MemoryRenderBuffer,
}

/// One window's title bar buffers, kept in its user data so the damage
/// tracker sees the same elements from frame to frame. Buffers can't be
/// shared between windows: element ids must be unique within a frame.
#[derive(Default)]
struct TitleBarBuffers {
    bar: SolidColorBuffer,
    /// The rasterized parts, per buffer scale, the one drawn last at the end:
    /// a window drawn on outputs of different scales keeps each's, so drawing
    /// one doesn't replace (and re-rasterize) another's.
    scaled: Vec<(i32, ScaledParts)>,
}

/// How many scales' parts a window keeps (`TitleBarBuffers::scaled`): more
/// than outputs of different scales one window spans.
const SCALES_KEPT: usize = 3;

/// A title bar's rasterized parts at one buffer scale.
#[derive(Default)]
struct ScaledParts {
    minimize: Vec<ButtonTexture>,
    maximize: Vec<ButtonTexture>,
    close: Vec<ButtonTexture>,
    title: Vec<TitleTexture>,
}

/// How many looks of a button a window keeps (`variant`): focused,
/// unfocused, hovered. Focus going back and forth (Mod+Tab) and the pointer
/// passing over a button then rasterize nothing again.
const BUTTON_LOOKS: usize = 3;

/// How many titles a window keeps: focused and unfocused.
const TITLE_LOOKS: usize = 2;

/// The texture in `slot` that `fits`, else the one `make` makes (if it
/// can), kept with the last used at the end, at most `keep` of them.
fn variant<T>(
    slot: &mut Vec<T>,
    fits: impl Fn(&T) -> bool,
    keep: usize,
    make: impl FnOnce() -> Option<T>,
) -> Option<&T> {
    match slot.iter().position(fits) {
        Some(index) => {
            let texture = slot.remove(index);
            slot.push(texture);
        }
        None => {
            let texture = make()?;
            if slot.len() >= keep {
                slot.remove(0);
            }
            slot.push(texture);
        }
    }
    slot.last()
}

/// The rendered title and what it was rendered for (text, width, color,
/// buffer scale), so it's only re-shaped when one of those changes.
struct TitleTexture {
    /// The text, its color and the buffer scale.
    key: (String, [f32; 3], i32),
    /// The room it was drawn in (logical).
    max_width: i32,
    /// The whole text fit (`text::Title::whole`).
    whole: bool,
    buffer: MemoryRenderBuffer,
    /// Width of the rendered text in logical pixels, for centering.
    width: i32,
}

impl TitleTexture {
    /// Whether it's what `key` drawn in `max_width` would be: drawn so, or
    /// the whole text, which any room it fits in leaves as it is (a resize
    /// changes the room every frame).
    fn shows(&self, key: &(String, [f32; 3], i32), max_width: i32) -> bool {
        self.key == *key && (self.max_width == max_width || (self.whole && self.width <= max_width))
    }
}

/// Where a title `text_width` wide starts in a bar at `bar_x`, `bar_width`
/// wide: centered on the bar, but slid left as needed to clear the buttons,
/// and never left of the padding.
fn title_x(bar_x: i32, bar_width: i32, text_width: i32) -> i32 {
    let centered = bar_x + (bar_width - text_width) / 2;
    let rightmost = bar_x + bar_width - BUTTONS * BUTTON_WIDTH - TITLE_PADDING - text_width;
    centered.min(rightmost).max(bar_x + TITLE_PADDING)
}

/// Whether the window is drawn maximized (its committed xdg state, or X11's
/// `_NET_WM_STATE`).
fn is_maximized(window: &Window) -> bool {
    if let Some(x11) = window.x11_surface() {
        return x11.is_maximized();
    }
    window.toplevel().is_some_and(|t| {
        t.with_committed_state(|state| {
            state.is_some_and(|s| s.states.contains(xdg_toplevel::State::Maximized))
        })
    })
}

/// Render elements for a title bar at `rect` (relative to the output origin),
/// front to back, drawn at `scale`. The textures are rasterized at that
/// scale rounded up (`scale::buffer_scale`), so they stay sharp on HiDPI
/// outputs; a window spanning outputs of different scales re-rasterizes
/// them for each.
#[allow(clippy::too_many_arguments)]
pub fn title_bar_elements(
    renderer: &mut GlesRenderer,
    window: &Window,
    rect: Rectangle<i32, Logical>,
    focused: bool,
    hover: Option<TitleBarPart>,
    colors: &Colors,
    radius: Option<f64>,
    alpha: f32,
    scale: Scale<f64>,
) -> Vec<PilaRenderElement> {
    // Rounded top corners, if the shaders for them work (`rounded.rs`).
    let shaders = radius.and_then(|_| rounded::Shaders::get(renderer));
    let radius = radius.filter(|_| shaders.is_some());
    let mut buffers = window
        .user_data()
        .get_or_insert(|| RefCell::new(TitleBarBuffers::default()))
        .borrow_mut();
    let TitleBarBuffers { bar, scaled } = &mut *buffers;

    let buffer_scale = crate::scale::buffer_scale(scale);
    // This scale's parts, taken out while drawing and put back last.
    let mut parts = scaled
        .iter()
        .position(|(s, _)| *s == buffer_scale)
        .map(|i| scaled.remove(i).1)
        .unwrap_or_default();
    let ScaledParts {
        minimize,
        maximize,
        close,
        title,
    } = &mut parts;
    let icon_color = if focused {
        colors.icon
    } else {
        colors.icon_inactive
    };
    // Circle fill color (traffic-light style when focused or hovered, muted when
    // unfocused) and optional dark glyph color shown on hover.
    let button_colors = |part| {
        let hovered = hover == Some(part);
        let active = match part {
            TitleBarPart::Close => [1.0, 0.373, 0.341],
            TitleBarPart::Minimize => [0.996, 0.737, 0.180],
            TitleBarPart::Maximize => [0.157, 0.784, 0.251],
            TitleBarPart::Drag => unreachable!(),
        };
        let fill = if hovered || focused {
            active
        } else {
            [0.33, 0.35, 0.39]
        };
        let glyph = hovered.then_some([0.12, 0.12, 0.12]);
        (fill, glyph)
    };
    let close_loc = Point::from((rect.loc.x + rect.size.w - BUTTON_WIDTH, rect.loc.y));
    let maximize_loc = close_loc - Point::from((BUTTON_WIDTH, 0));
    let minimize_loc = maximize_loc - Point::from((BUTTON_WIDTH, 0));

    let mut elements = Vec::with_capacity(4);
    let upload =
        |renderer: &mut GlesRenderer, buffer: &MemoryRenderBuffer, loc: Point<i32, Logical>| {
            let loc: Point<f64, Physical> = loc.to_f64().to_physical(scale);
            MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                loc,
                buffer,
                (alpha < 1.0).then_some(alpha),
                None,
                None,
                Kind::Unspecified,
            )
            .map_err(|err| warn!(?err, "failed to upload title bar texture"))
            .ok()
        };

    // The title: at most the space between the left padding and the buttons,
    // centered on the bar where it fits (see `title_x`).
    let title_width =
        (rect.size.w - BUTTONS * BUTTON_WIDTH - 2 * TITLE_PADDING).min(MAX_TITLE_WIDTH);
    let text = crate::foreign_toplevel::title(window);
    if title_width >= MIN_TITLE_WIDTH && !text.is_empty() {
        let key = (text, icon_color, buffer_scale);
        let fits = |texture: &TitleTexture| texture.shows(&key, title_width);
        // None until the fonts have loaded; then the next frame draws it.
        let make = || {
            let drawn = crate::text::render_title(
                &key.0,
                icon_color,
                title_width,
                TITLE_BAR_HEIGHT,
                buffer_scale,
            )?;
            Some(TitleTexture {
                buffer: MemoryRenderBuffer::from_slice(
                    &drawn.pixels,
                    Fourcc::Argb8888,
                    (drawn.width, TITLE_BAR_HEIGHT * buffer_scale),
                    buffer_scale,
                    Transform::Normal,
                    None,
                ),
                width: drawn.width / buffer_scale,
                max_width: title_width,
                whole: drawn.whole,
                key: key.clone(),
            })
        };
        if let Some(texture) = variant(title, fits, TITLE_LOOKS, make) {
            let x = title_x(rect.loc.x, rect.size.w, texture.width);
            let loc = Point::from((x, rect.loc.y));
            elements.extend(upload(renderer, &texture.buffer, loc).map(Into::into));
        }
    } else {
        title.clear();
    }

    for (slot, icon, part, loc) in [
        (minimize, Icon::Minimize, TitleBarPart::Minimize, minimize_loc),
        (maximize, Icon::Maximize, TitleBarPart::Maximize, maximize_loc),
        (close, Icon::Close, TitleBarPart::Close, close_loc),
    ] {
        let corner = match (part, radius) {
            (TitleBarPart::Close, Some(radius)) => {
                (radius * f64::from(buffer_scale)).round() as i32
            }
            _ => 0,
        };
        let (fill, glyph) = button_colors(part);
        let key = (icon, fill, glyph, buffer_scale, corner);
        let make = || {
            Some(ButtonTexture {
                key,
                buffer: button_buffer(icon, fill, glyph, buffer_scale, corner),
            })
        };
        if let Some(texture) = variant(slot, |texture| texture.key == key, BUTTON_LOOKS, make) {
            elements.extend(upload(renderer, &texture.buffer, loc).map(Into::into));
        }
    }

    bar.update(
        rect.size,
        if focused {
            colors.bar
        } else if crate::urgency::is_urgent(window) {
            colors.bar_urgent
        } else {
            colors.bar_inactive
        },
    );
    let loc: Point<i32, Physical> = rect.loc.to_physical_precise_round(scale);
    let bar = SolidColorRenderElement::from_buffer(bar, loc, scale, alpha, Kind::Unspecified);
    elements.push(match (radius, &shaders) {
        (Some(radius), Some(shaders)) => {
            RoundedSolid::new(bar, shaders, radius, Corners::Top, scale).into()
        }
        _ => bar.into(),
    });
    scaled.push((buffer_scale, parts));
    if scaled.len() > SCALES_KEPT {
        scaled.remove(0);
    }
    elements
}

/// Draw one circular button ("bolinha") centered in a `BUTTON_WIDTH` x
/// `TITLE_BAR_HEIGHT` logical-pixel cell at `scale` physical pixels each.
/// `glyph_color` is `Some` when hovered, drawing a crisp symbol inside the
/// circle. A nonzero `corner` (buffer pixels) clips against the bar's
/// rounded top-right corner.
fn button_buffer(
    icon: Icon,
    fill: [f32; 3],
    glyph_color: Option<[f32; 3]>,
    scale: i32,
    corner: i32,
) -> MemoryRenderBuffer {
    let pixels = rasterize_button(icon, fill, glyph_color, scale, corner);
    MemoryRenderBuffer::from_slice(
        &pixels,
        Fourcc::Argb8888,
        (BUTTON_WIDTH * scale, TITLE_BAR_HEIGHT * scale),
        scale,
        Transform::Normal,
        None,
    )
}

/// Premultiplied ARGB8888 pixels (little-endian: B, G, R, A) for a circular
/// button, `scale` physical pixels per logical one, with the top-right corner
/// rounded by `corner` physical pixels.
fn rasterize_button(
    icon: Icon,
    fill: [f32; 3],
    glyph_color: Option<[f32; 3]>,
    scale: i32,
    corner: i32,
) -> Vec<u8> {
    let radius = f64::from(corner) / f64::from(scale);
    let (w, h) = (BUTTON_WIDTH * scale, TITLE_BAR_HEIGHT * scale);
    let s = scale as f64;
    let (cx, cy) = (BUTTON_WIDTH as f64 / 2.0, TITLE_BAR_HEIGHT as f64 / 2.0);
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let p = ((x as f64 + 0.5) / s, (y as f64 + 0.5) / s);
            let shape = rounded::top_right_coverage(p, BUTTON_WIDTH as f64, radius, s);
            let dist = ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt() - BUTTON_RADIUS;
            let circle = ((-dist * s + 0.5).clamp(0.0, 1.0) * shape) as f32;
            let glyph = match glyph_color {
                Some(gc) => {
                    let g = (glyph_coverage(icon, p, (cx, cy), s) as f32).min(circle);
                    Some((gc, g))
                }
                None => None,
            };
            let (color, alpha) = match glyph {
                Some((gc, g)) if circle > 0.0 => {
                    let t = g / circle;
                    (
                        [
                            fill[0] * (1.0 - t) + gc[0] * t,
                            fill[1] * (1.0 - t) + gc[1] * t,
                            fill[2] * (1.0 - t) + gc[2] * t,
                        ],
                        circle,
                    )
                }
                _ => (fill, circle),
            };
            let byte = |v: f32| (v.clamp(0.0, 1.0) * alpha * 255.0).round() as u8;
            let alpha_byte = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
            pixels.extend([
                byte(color[2]),
                byte(color[1]),
                byte(color[0]),
                alpha_byte,
            ]);
        }
    }
    pixels
}

/// How much of the pixel centered at `p` the glyph inside the circle covers
/// (0..=1). `p` is in logical pixels; a physical pixel is `1 / scale` of one.
fn glyph_coverage(icon: Icon, p: (f64, f64), (cx, cy): (f64, f64), scale: f64) -> f64 {
    // A stroke `width` wide along a shape at distance `dist` from p,
    // anti-aliased over one physical pixel.
    let stroke = |dist: f64, width: f64| ((width / 2.0 - dist) * scale + 0.5).clamp(0.0, 1.0);
    match icon {
        Icon::Close => {
            let r = 2.8;
            let d1 = segment_distance(p, (cx - r, cy - r), (cx + r, cy + r));
            let d2 = segment_distance(p, (cx + r, cy - r), (cx - r, cy + r));
            stroke(d1.min(d2), 1.3)
        }
        Icon::Maximize => {
            // Opposing diagonal arrows pointing to top-right and bottom-left.
            let d = 2.6;
            let h = 0.6;
            let tr1 = segment_distance(p, (cx + h, cy - d), (cx + d, cy - d));
            let tr2 = segment_distance(p, (cx + d, cy - d), (cx + d, cy - h));
            let tr3 = segment_distance(p, (cx + 0.7, cy - 0.7), (cx + d, cy - d));
            let bl1 = segment_distance(p, (cx - h, cy + d), (cx - d, cy + d));
            let bl2 = segment_distance(p, (cx - d, cy + d), (cx - d, cy + h));
            let bl3 = segment_distance(p, (cx - 0.7, cy + 0.7), (cx - d, cy + d));
            stroke(tr1.min(tr2).min(tr3).min(bl1).min(bl2).min(bl3), 1.15)
        }
        Icon::Minimize => stroke(
            segment_distance(p, (cx - 3.0, cy), (cx + 3.0, cy)),
            1.3,
        ),
    }
}

/// Distance from `p` to the segment `a`-`b`.
fn segment_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (abx, aby) = (b.0 - a.0, b.1 - a.1);
    let (apx, apy) = (p.0 - a.0, p.1 - a.1);
    let t = ((apx * abx + apy * aby) / (abx * abx + aby * aby)).clamp(0.0, 1.0);
    let (dx, dy) = (apx - abx * t, apy - aby * t);
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_buttons_looks_are_kept_and_made_again_only_when_gone() {
        let mut slot: Vec<u32> = Vec::new();
        let mut made = 0;
        let mut get = |slot: &mut Vec<u32>, look: u32| {
            *variant(
                slot,
                |t| *t == look,
                3,
                || {
                    made += 1;
                    Some(look)
                },
            )
            .unwrap()
        };
        // Focused, unfocused, focused again (Mod+Tab), hovered: three made.
        for look in [1, 2, 1, 3, 2, 1] {
            assert_eq!(get(&mut slot, look), look);
        }
        assert_eq!(slot, [3, 2, 1]);
        // A fourth look drops the one used longest ago.
        get(&mut slot, 4);
        assert_eq!(slot, [2, 1, 4]);
        assert_eq!(made, 4);
        // One that can't be made yet (no fonts) leaves the others.
        assert!(variant(&mut slot, |t| *t == 5, 3, || None).is_none());
        assert_eq!(slot, [2, 1, 4]);
    }

    #[test]
    fn a_whole_titles_texture_serves_any_room_it_fits_in() {
        let key = ("Files".to_string(), [1.0; 3], 1);
        let texture = |whole| TitleTexture {
            key: key.clone(),
            max_width: 300,
            whole,
            buffer: MemoryRenderBuffer::new(Fourcc::Argb8888, (1, 1), 1, Transform::Normal, None),
            width: 40,
        };
        // A resize: the room changes, the whole text still fits.
        assert!(texture(true).shows(&key, 300));
        assert!(texture(true).shows(&key, 120));
        assert!(texture(true).shows(&key, 40));
        assert!(!texture(true).shows(&key, 39));
        // Cut short ("…"): only the room it was drawn in.
        assert!(!texture(false).shows(&key, 120));
        assert!(texture(false).shows(&key, 300));
        // Another text, color or scale is another texture.
        assert!(!texture(true).shows(&("Edit".to_string(), [1.0; 3], 1), 300));
        assert!(!texture(true).shows(&("Files".to_string(), [0.5; 3], 1), 300));
        assert!(!texture(true).shows(&("Files".to_string(), [1.0; 3], 2), 300));
    }

    fn bar() -> Rectangle<i32, Logical> {
        // A 300px-wide bar at (100, 50).
        Rectangle::new((100, 50).into(), (300, TITLE_BAR_HEIGHT).into())
    }

    #[test]
    fn title_bar_buttons_are_at_the_right_end() {
        let y = 50.0 + TITLE_BAR_HEIGHT as f64 / 2.0;
        let right = 400.0;
        assert_eq!(
            title_bar_part(bar(), (right - 1.0, y).into()),
            TitleBarPart::Close
        );
        assert_eq!(
            title_bar_part(bar(), (right - BUTTON_WIDTH as f64 - 1.0, y).into()),
            TitleBarPart::Maximize
        );
        assert_eq!(
            title_bar_part(bar(), (right - 2.0 * BUTTON_WIDTH as f64 - 1.0, y).into()),
            TitleBarPart::Minimize
        );
        assert_eq!(
            title_bar_part(bar(), (right - 3.0 * BUTTON_WIDTH as f64 - 1.0, y).into()),
            TitleBarPart::Drag
        );
        assert_eq!(title_bar_part(bar(), (101.0, y).into()), TitleBarPart::Drag);
    }

    fn alpha_at(pixels: &[u8], x: i32, y: i32) -> u8 {
        pixels[((y * BUTTON_WIDTH + x) * 4 + 3) as usize]
    }

    fn alpha_at_2x(pixels: &[u8], x: i32, y: i32) -> u8 {
        pixels[((y * BUTTON_WIDTH * 2 + x) * 4 + 3) as usize]
    }

    #[test]
    fn button_circles_are_centered_and_crisp() {
        let px = rasterize_button(Icon::Close, [1.0, 0.37, 0.34], None, 1, 0);
        // Center of the circle is opaque; cell corners stay transparent.
        assert_eq!(alpha_at(&px, BUTTON_WIDTH / 2, TITLE_BAR_HEIGHT / 2), 255);
        assert_eq!(alpha_at(&px, 0, 0), 0);
        assert_eq!(alpha_at(&px, BUTTON_WIDTH - 1, TITLE_BAR_HEIGHT - 1), 0);

        // Hovered button draws a dark glyph inside the opaque circle.
        let px_hover = rasterize_button(
            Icon::Close,
            [1.0, 0.37, 0.34],
            Some([0.12, 0.12, 0.12]),
            1,
            0,
        );
        let center_idx = ((TITLE_BAR_HEIGHT / 2 * BUTTON_WIDTH + BUTTON_WIDTH / 2) * 4) as usize;
        assert_eq!(px_hover[center_idx + 3], 255);
        // Red channel (index 2 in BGRA) is darker at the center cross than in the unhovered fill.
        assert!(px_hover[center_idx + 2] < 80);
    }

    #[test]
    fn buttons_at_scale_2_have_crisp_circles() {
        let px = rasterize_button(Icon::Maximize, [0.16, 0.78, 0.25], None, 2, 0);
        assert_eq!(px.len() as i32, BUTTON_WIDTH * 2 * TITLE_BAR_HEIGHT * 2 * 4);
        assert_eq!(alpha_at_2x(&px, BUTTON_WIDTH, TITLE_BAR_HEIGHT), 255);
        assert_eq!(alpha_at_2x(&px, 0, 0), 0);
    }

    #[test]
    fn titles_center_on_the_bar_but_clear_the_buttons() {
        let buttons = BUTTONS * BUTTON_WIDTH;
        // Short title on a wide bar: centered on the whole bar.
        assert_eq!(title_x(100, 600, 80), 100 + (600 - 80) / 2);
        // Wider title: centering would overlap the buttons, so it slides left
        // until it ends TITLE_PADDING before them.
        let x = title_x(100, 300, 140);
        assert_eq!(x + 140, 100 + 300 - buttons - TITLE_PADDING);
        assert!(x < 100 + (300 - 140) / 2);
        // A title filling all the space starts at the left padding.
        let full = 300 - buttons - 2 * TITLE_PADDING;
        assert_eq!(title_x(100, 300, full), 100 + TITLE_PADDING);
    }

    #[test]
    fn band_edges_pick_side_and_corners() {
        let (low, high) = (ResizeEdge::LEFT, ResizeEdge::RIGHT);
        // Frame spans 100..400 on this axis.
        assert_eq!(band_edges(97.0, 100.0, 300.0, low, high), low); // in the border, before the frame
        assert_eq!(band_edges(403.0, 100.0, 300.0, low, high), high); // past the frame
        assert_eq!(band_edges(110.0, 100.0, 300.0, low, high), low); // within CORNER of the start
        assert_eq!(band_edges(390.0, 100.0, 300.0, low, high), high); // within CORNER of the end
        assert_eq!(
            band_edges(250.0, 100.0, 300.0, low, high),
            ResizeEdge::empty()
        ); // middle
    }
}

//! The overview (`overview`, Mod+W by default): the active workspace's
//! windows spread out on their outputs, none over another, live, over the
//! dimmed wallpaper; panels (upper layer surfaces) stay as they are. A click
//! on a window, or the arrows and Enter, picks it (focused and raised as it
//! closes); Escape, a click beside the windows or any other binding closes
//! it.
//!
//! Nothing is drawn anew: each window is drawn as an animating one is, its
//! frame mapped onto its place in the overview (`animation::FrameMap`), so
//! its title bar, corners, shadow and live content come along and its frames
//! are paced by the output it's shown on (`frame::send_frames`). The places
//! are laid out from the windows as they are whenever they're asked for
//! (`Pila::overview_slots`, `arrange`): a window opening, closing or
//! resizing meanwhile takes its place at once. This module draws only what's
//! behind them: the selected one's outline and the dimming
//! (`overview_elements`).
//!
//! Opening, the windows glide from where they are to their places and the
//! wallpaper dims (`[animations] overview`); closing, they glide back from
//! the places they had then (`Overview::closing`) while it's already closed
//! to input. A glide is an animation like the others (`step_animations`
//! ends it, `animating` keeps frames coming).

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use smithay::{
    backend::renderer::{
        element::{
            solid::{SolidColorBuffer, SolidColorRenderElement},
            Kind,
        },
        gles::GlesRenderer,
    },
    desktop::Window,
    input::keyboard::Keysym,
    output::Output,
    utils::{Logical, Point, Rectangle, Scale, Size},
};

use crate::{
    animation::{lerp_rect, Animation, FrameMap, Transform, WindowEffect},
    config::{Action, AnimationKind, Colors, OverviewAction},
    render::{PilaRenderElement, SceneWindow},
    snap::Direction,
    Pila,
};

/// Around the windows, inside the part of the output panels leave.
const MARGIN: i32 = 48;
/// Between two windows, across and down.
const GAP: i32 = 32;
/// The selected window's outline, around its frame.
const OUTLINE: i32 = 4;
/// What the wallpaper is dimmed with (premultiplied black).
const DIM: [f32; 4] = [0.0, 0.0, 0.0, 0.45];

/// The overview while it's open, or gliding closed.
#[derive(Default)]
pub struct Overview {
    open: Option<Open>,
    /// How far it's shown (0: closed, 1: open) while it opens or closes.
    glide: Option<Animation>,
    /// Closing: the places the windows glide back from.
    closing: Option<Vec<Slot>>,
    buffers: Rc<RefCell<Buffers>>,
}

#[derive(Default)]
struct Open {
    /// The window Enter picks: the focused one at first, then the one the
    /// arrows or the pointer go to.
    selected: Option<Window>,
    /// The places last laid out, to redraw the outputs when they change.
    laid_out: Vec<(Window, Rectangle<i32, Logical>)>,
}

/// Kept between frames while it's open: an unchanged overview adds no
/// damage.
#[derive(Default)]
struct Buffers {
    /// Each output's dimming, by name.
    dim: HashMap<String, SolidColorBuffer>,
    outline: Option<SolidColorBuffer>,
}

/// A window's place in the overview.
#[derive(Clone)]
pub struct Slot {
    pub window: Window,
    pub output: Output,
    /// Its frame (geometry and title bar) where it is.
    pub frame: Rectangle<i32, Logical>,
    /// Where its frame is shown in the overview.
    pub rect: Rectangle<i32, Logical>,
}

/// What `Pila::scene` gives the renderer of the overview open.
#[derive(Clone)]
pub struct SceneOverview {
    /// The outputs it dims, by name, and where they are.
    pub outputs: Vec<(String, Rectangle<i32, Logical>)>,
    /// The selected window's place.
    pub selected: Option<Rectangle<i32, Logical>>,
    /// Its outline's corners: the windows' radius and corners (`None`:
    /// square).
    pub rounding: Option<(f64, crate::rounded::Corners)>,
    /// How far it's shown (0 to 1): the dimming and outline fade with it.
    pub shown: f32,
    buffers: Rc<RefCell<Buffers>>,
}

/// Places for frames of `sizes` in `area`, in rows, none over another: each
/// shrunk by the same scale (never grown), the order kept from row to row,
/// rows and windows centered, `GAP` between them. The number of rows is the
/// one that shows them largest (the fewest of equal ones).
pub fn arrange(
    area: Rectangle<i32, Logical>,
    sizes: &[Size<i32, Logical>],
) -> Vec<Rectangle<i32, Logical>> {
    let n = sizes.len();
    if n == 0 {
        return Vec::new();
    }
    let size = |i: usize| {
        let s = sizes[i];
        (f64::from(s.w.max(1)), f64::from(s.h.max(1)))
    };
    let (avail_w, avail_h) = (f64::from(area.size.w), f64::from(area.size.h));
    let gap = f64::from(GAP);
    // The rows of `rows` rows: as even as can be, the first ones longer.
    let split = |rows: usize| -> Vec<std::ops::Range<usize>> {
        let (per, extra) = (n / rows, n % rows);
        let mut start = 0;
        (0..rows)
            .map(|row| {
                let len = per + usize::from(row < extra);
                let range = start..start + len;
                start += len;
                range
            })
            .collect()
    };
    let scale_for = |rows: &[std::ops::Range<usize>]| -> f64 {
        let mut scale: f64 = 1.0;
        let mut heights = 0.0;
        for row in rows {
            let width: f64 = row.clone().map(|i| size(i).0).sum();
            let gaps = gap * (row.len() as f64 - 1.0);
            scale = scale.min((avail_w - gaps).max(1.0) / width);
            heights += row.clone().map(|i| size(i).1).fold(0.0, f64::max);
        }
        let gaps = gap * (rows.len() as f64 - 1.0);
        scale.min((avail_h - gaps).max(1.0) / heights)
    };
    let (rows, scale) = (1..=n)
        .map(|count| {
            let rows = split(count);
            let scale = scale_for(&rows);
            (rows, scale)
        })
        .fold(None::<(Vec<_>, f64)>, |best, (rows, scale)| match best {
            Some((_, best_scale)) if best_scale >= scale => best,
            _ => Some((rows, scale)),
        })
        .unwrap_or_default();
    let row_height =
        |row: &std::ops::Range<usize>| row.clone().map(|i| size(i).1 * scale).fold(0.0, f64::max);
    let total_h: f64 = rows.iter().map(row_height).sum::<f64>() + gap * (rows.len() as f64 - 1.0);
    let mut places = vec![Rectangle::default(); n];
    let mut y = f64::from(area.loc.y) + (avail_h - total_h) / 2.0;
    for row in &rows {
        let height = row_height(row);
        let width: f64 =
            row.clone().map(|i| size(i).0 * scale).sum::<f64>() + gap * (row.len() as f64 - 1.0);
        let mut x = f64::from(area.loc.x) + (avail_w - width) / 2.0;
        for i in row.clone() {
            let (w, h) = (size(i).0 * scale, size(i).1 * scale);
            places[i] = Rectangle::new(
                (x.round() as i32, (y + (height - h) / 2.0).round() as i32).into(),
                ((w.round() as i32).max(1), (h.round() as i32).max(1)).into(),
            );
            x += w + gap;
        }
        y += height + gap;
    }
    places
}

/// The place one step from `from` toward `direction` among `places`: the
/// nearest whose center lies that way, a sideways distance counting double.
pub fn step(
    places: &[Rectangle<i32, Logical>],
    from: usize,
    direction: Direction,
) -> Option<usize> {
    let center = |r: &Rectangle<i32, Logical>| {
        let r = r.to_f64();
        (r.loc.x + r.size.w / 2.0, r.loc.y + r.size.h / 2.0)
    };
    let (fx, fy) = center(places.get(from)?);
    places
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != from)
        .filter_map(|(i, place)| {
            let (x, y) = center(place);
            let (ahead, aside) = match direction {
                Direction::Left => (fx - x, y - fy),
                Direction::Right => (x - fx, y - fy),
                Direction::Up => (fy - y, x - fx),
                Direction::Down => (y - fy, x - fx),
            };
            (ahead > 0.0).then_some((i, ahead + 2.0 * aside.abs()))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

impl Action {
    /// Whether the overview stays open for it: going to a workspace or
    /// sending a window there, switching layouts and the overview's own.
    /// Any other action closes it first (`Pila::run_action`).
    pub(crate) fn keeps_overview(&self) -> bool {
        matches!(
            self,
            Action::Overview(_)
                | Action::Workspace(_)
                | Action::MoveToWorkspace(_)
                | Action::SwitchLayout(_)
        )
    }
}

impl Pila {
    pub fn overview_open(&self) -> bool {
        self.overview.open.is_some()
    }

    /// The `overview` action.
    pub(crate) fn overview_action(&mut self, action: OverviewAction) {
        match action {
            OverviewAction::Open => self.open_overview(),
            OverviewAction::Close => self.close_overview(),
            OverviewAction::Toggle if self.overview_open() => self.close_overview(),
            OverviewAction::Toggle => self.open_overview(),
        }
    }

    /// Open it, the focused window selected. Not while locked.
    pub fn open_overview(&mut self) {
        if self.overview_open() || self.is_locked() {
            return;
        }
        tracing::debug!("opening the overview");
        self.end_switch();
        self.close_window_menu();
        self.hide_shortcuts();
        self.dismiss_popup_grab();
        // From where a closing one is, if it's on its way back.
        let now = self.animation_time();
        let from = self.overview_shown(now);
        self.overview.closing = None;
        self.overview.glide = self
            .animation_duration(AnimationKind::Overview)
            .map(|duration| Animation::new(now, duration.mul_f64(1.0 - from), from, 1.0));
        if self.overview.glide.is_some() {
            crate::animation::log_overview_glide(true);
        }
        self.overview.open = Some(Open {
            selected: self.focused_window(),
            laid_out: Vec::new(),
        });
        self.overview_laid_out();
        // Every output: windows move, and a fullscreen window's panels show.
        self.queue_redraw();
        self.refresh_pointer_focus();
        let pointer_grabbed = self.seat.get_pointer().is_some_and(|p| p.is_grabbed());
        if !pointer_grabbed {
            self.set_cursor(smithay::input::pointer::CursorImageStatus::default_named());
        }
    }

    /// Close it, the windows gliding back where they are.
    pub fn close_overview(&mut self) {
        if !self.overview_open() {
            return;
        }
        tracing::debug!("closing the overview");
        let slots = self.overview_slots();
        let now = self.animation_time();
        let from = self.overview_shown(now);
        self.overview.open = None;
        self.overview.glide = self
            .animation_duration(AnimationKind::Overview)
            .map(|duration| Animation::new(now, duration.mul_f64(from), from, 0.0));
        if self.overview.glide.is_some() {
            crate::animation::log_overview_glide(false);
            self.overview.closing = Some(slots);
        } else {
            self.overview_closed(&slots);
        }
        self.queue_redraw();
        self.refresh_pointer_focus();
    }

    /// Done closing: what was kept for it goes.
    fn overview_closed(&mut self, slots: &[Slot]) {
        *self.overview.buffers.borrow_mut() = Buffers::default();
        for slot in slots {
            if !self.window_animates(&slot.window) {
                crate::animation::clear_textures(slot.window.user_data());
            }
        }
    }

    /// How far it's shown at `now`: 0 closed, 1 open, between while it
    /// glides.
    pub(crate) fn overview_shown(&self, now: std::time::Duration) -> f64 {
        match &self.overview.glide {
            Some(glide) => glide.value(now),
            None if self.overview_open() => 1.0,
            None => 0.0,
        }
    }

    /// Whether it opens or closes, though its time may be up
    /// (`animations_held`): every output is redrawn for it.
    pub(crate) fn overview_moving(&self) -> bool {
        self.overview.glide.is_some()
    }

    /// Whether it's still gliding at `now` (`animating`).
    pub(crate) fn overview_gliding(&self, now: std::time::Duration) -> bool {
        self.overview.glide.as_ref().is_some_and(|g| !g.done(now))
    }

    /// End a glide that's done (`step_animations`).
    pub(crate) fn end_finished_overview(&mut self, now: std::time::Duration) {
        if self.overview.glide.as_ref().is_some_and(|g| g.done(now)) {
            self.overview.glide = None;
            if let Some(slots) = self.overview.closing.take() {
                self.overview_closed(&slots);
            }
        }
    }

    /// Stop gliding at once (the session locked, `drop_animations`).
    pub(crate) fn stop_overview_glide(&mut self) {
        self.overview.glide = None;
        if let Some(slots) = self.overview.closing.take() {
            self.overview_closed(&slots);
            self.queue_redraw();
        }
    }

    /// Close it on `window`, focused and raised (brought back if minimized).
    fn pick_in_overview(&mut self, window: &Window) {
        self.close_overview();
        self.activate_toplevel(window);
    }

    /// A key pressed while it's open that's no binding: Escape closes it,
    /// the arrows go from window to window, Enter picks the selected one.
    pub(crate) fn overview_key(&mut self, sym: Keysym) {
        let direction = match sym {
            Keysym::Escape => return self.close_overview(),
            Keysym::Return | Keysym::KP_Enter | Keysym::space => {
                let selected = self.overview.open.as_ref().and_then(|o| o.selected.clone());
                match selected {
                    Some(window) => self.pick_in_overview(&window),
                    None => self.close_overview(),
                }
                return;
            }
            Keysym::Left => Direction::Left,
            Keysym::Right => Direction::Right,
            Keysym::Up => Direction::Up,
            Keysym::Down => Direction::Down,
            _ => return,
        };
        let slots = self.overview_slots();
        let places: Vec<_> = slots.iter().map(|s| s.rect).collect();
        let selected = self
            .overview
            .open
            .as_ref()
            .and_then(|o| o.selected.as_ref());
        let next = match selected.and_then(|w| slots.iter().position(|s| &s.window == w)) {
            Some(from) => step(&places, from, direction),
            // Nothing selected yet: the first one.
            None => (!slots.is_empty()).then_some(0),
        };
        if let Some(next) = next {
            self.select_in_overview(Some(slots[next].window.clone()));
        }
    }

    /// Select `window` (the arrows, the pointer over it), redrawing the
    /// outline where it was and where it is.
    fn select_in_overview(&mut self, window: Option<Window>) {
        let Some(open) = self.overview.open.as_mut() else {
            return;
        };
        if open.selected == window {
            return;
        }
        let before = std::mem::replace(&mut open.selected, window.clone());
        for slot in self.overview_slots() {
            if Some(&slot.window) == before.as_ref() || Some(&slot.window) == window.as_ref() {
                self.queue_redraw_rect(outline_rect(slot.rect));
            }
        }
    }

    /// The pointer moved to `location` while it's open: the window under it
    /// is selected.
    pub(crate) fn overview_pointer_moved(&mut self, location: Point<f64, Logical>) {
        if !self.overview_open() {
            return;
        }
        if let Some(window) = self.overview_window_at(location) {
            self.select_in_overview(Some(window));
        }
    }

    /// A press while it's open: on a window it picks that one; beside the
    /// windows (not on a panel) it closes it. `true` if it took the press,
    /// which then goes nowhere else (its release neither).
    pub(crate) fn overview_press(&mut self, location: Point<f64, Logical>) -> bool {
        if !self.overview_open() {
            return false;
        }
        if let Some(window) = self.overview_window_at(location) {
            self.pick_in_overview(&window);
            return true;
        }
        // Panels above it still take their clicks.
        if self.under(location).is_some() {
            return false;
        }
        self.close_overview();
        true
    }

    /// The window shown at `location` (global), while it's open.
    pub fn overview_window_at(&self, location: Point<f64, Logical>) -> Option<Window> {
        if !self.overview_open() {
            return None;
        }
        self.overview_slots()
            .into_iter()
            .find(|slot| slot.rect.to_f64().contains(location))
            .map(|slot| slot.window)
    }

    /// A window committed while it's open: what it shows changed, and its
    /// size may move the others on its output.
    pub(crate) fn overview_committed(&mut self, window: &Window) {
        if !self.overview_open() {
            return;
        }
        let output = self
            .overview_slots()
            .into_iter()
            .find(|slot| &slot.window == window)
            .map(|slot| slot.output);
        if let Some(output) = output {
            self.queue_redraw_output(&output);
        }
    }

    /// After each turn of the loop while it's open: an output whose places
    /// changed (a window opened, closed, moved or was minimized) is redrawn,
    /// and a selected window that's gone leaves nothing selected.
    pub(crate) fn overview_laid_out(&mut self) {
        let Some(open) = self.overview.open.as_ref() else {
            return;
        };
        let slots = self.overview_slots();
        let laid_out: Vec<_> = slots.iter().map(|s| (s.window.clone(), s.rect)).collect();
        if laid_out == open.laid_out {
            return;
        }
        let changed: Vec<Output> = slots
            .iter()
            .filter(|slot| {
                !open
                    .laid_out
                    .iter()
                    .any(|(w, rect)| *w == slot.window && *rect == slot.rect)
            })
            .map(|slot| slot.output.clone())
            .collect();
        let gone = open
            .laid_out
            .iter()
            .any(|(w, _)| !laid_out.iter().any(|(other, _)| other == w));
        let selected_gone = open
            .selected
            .as_ref()
            .is_some_and(|w| !laid_out.iter().any(|(other, _)| other == w));
        if let Some(open) = self.overview.open.as_mut() {
            open.laid_out = laid_out;
            if selected_gone {
                open.selected = None;
            }
        }
        self.defer_redraws(|state| {
            if gone {
                state.queue_redraw();
            } else {
                for output in &changed {
                    state.queue_redraw_output(output);
                }
            }
        });
    }

    /// The windows it shows, each with its place: per output, the active
    /// workspace's windows on it (the minimized ones too), laid out in the
    /// part panels leave, in the order they're on screen.
    pub fn overview_slots(&self) -> Vec<Slot> {
        if !self.overview_open() {
            return Vec::new();
        }
        let active = self.workspaces.active;
        let first_output = self.space.outputs().next().cloned();
        let mapped = self
            .space
            .elements()
            .filter(|w| !crate::workspaces::is_override_redirect(w))
            .filter_map(|w| {
                let frame = frame_at(w, self.space.element_location(w)?);
                Some((w.clone(), self.window_output(w)?, frame))
            });
        let minimized = self
            .windows
            .minimized
            .iter()
            .filter(|w| self.workspace_of(w) == active)
            .filter_map(|w| {
                let frame = frame_at(w, crate::workspaces::space_location(w));
                let output = self
                    .output_overlapping(frame)
                    .or_else(|| first_output.clone())?;
                Some((w.clone(), output, frame))
            });
        let windows: Vec<(Window, Output, Rectangle<i32, Logical>)> =
            mapped.chain(minimized).collect();
        let mut slots = Vec::new();
        for output in self.space.outputs() {
            let Some(area) = self.output_area(output, true) else {
                continue;
            };
            let mut here: Vec<&(Window, Output, Rectangle<i32, Logical>)> =
                windows.iter().filter(|(_, o, _)| o == output).collect();
            // As they're on screen: top to bottom, then left to right.
            here.sort_by_key(|(_, _, frame)| {
                let center = crate::animation::center(*frame);
                (center.y as i32, center.x as i32)
            });
            let inner = Rectangle::new(
                area.loc + Point::from((MARGIN, MARGIN)),
                (area.size.w - 2 * MARGIN, area.size.h - 2 * MARGIN).into(),
            );
            let sizes: Vec<_> = here.iter().map(|(_, _, frame)| frame.size).collect();
            let places = arrange(inner, &sizes);
            slots.extend(
                here.into_iter()
                    .zip(places)
                    .map(|((window, output, frame), rect)| Slot {
                        window: window.clone(),
                        output: output.clone(),
                        frame: *frame,
                        rect,
                    }),
            );
        }
        slots
    }

    /// The scene's windows as it shows them: each on its place, `shown` of
    /// the way there from where it is (minimized ones added behind, fading
    /// with it); while it's open, those it doesn't show are left out.
    pub(crate) fn overview_scene_windows(
        &self,
        windows: Vec<SceneWindow>,
        focused: Option<&Window>,
    ) -> Vec<SceneWindow> {
        let now = self.animation_time();
        let shown = self.overview_shown(now);
        let open = self.overview_open();
        let slots = match &self.overview.closing {
            Some(slots) if !open => slots.clone(),
            _ => self.overview_slots(),
        };
        let effect = |slot: &Slot, frame: Rectangle<i32, Logical>, alpha: f32| {
            let to = lerp_rect(frame, slot.rect, shown);
            WindowEffect {
                transform: Transform {
                    alpha,
                    scale: 1.0,
                    offset: Point::default(),
                },
                center: to.loc + to.size.downscale(2.0).to_point(),
                output: Some(slot.output.clone()),
                frame: Some(FrameMap {
                    from: frame.to_f64(),
                    to,
                }),
            }
        };
        let mut drawn: Vec<SceneWindow> = windows
            .into_iter()
            .filter_map(|mut entry| {
                let Some(slot) = slots.iter().find(|s| s.window == entry.window) else {
                    return (!open).then_some(entry);
                };
                let frame = self
                    .space
                    .element_location(&entry.window)
                    .map_or(slot.frame, |loc| frame_at(&entry.window, loc));
                entry.effect = Some(effect(slot, frame, 1.0));
                Some(entry)
            })
            .collect();
        for slot in slots.iter().filter(|s| self.is_minimized(&s.window)) {
            let loc = crate::workspaces::space_location(&slot.window);
            if let Some(mut entry) = self.scene_window_at(&slot.window, loc, focused, None, None) {
                entry.effect = Some(effect(slot, slot.frame, shown as f32));
                drawn.push(entry);
            }
        }
        drawn
    }

    /// The overview for `Pila::scene`, while it's open or gliding closed.
    pub(crate) fn scene_overview(&self) -> Option<SceneOverview> {
        if !self.overview_open() && self.overview.closing.is_none() {
            return None;
        }
        let selected = self.overview.open.as_ref().and_then(|open| {
            let window = open.selected.as_ref()?;
            self.overview_slots()
                .into_iter()
                .find(|slot| &slot.window == window)
                .map(|slot| slot.rect)
        });
        Some(SceneOverview {
            outputs: self
                .space
                .outputs()
                .filter_map(|o| Some((o.name(), self.space.output_geometry(o)?)))
                .collect(),
            selected,
            rounding: (self.config.corner_radius > 0).then_some((
                f64::from(self.config.corner_radius + OUTLINE as u32),
                self.config.rounded_corners,
            )),
            shown: self.overview_shown(self.animation_time()) as f32,
            buffers: self.overview.buffers.clone(),
        })
    }

    /// For the state dump: whether it's open, the selected window and each
    /// window's place.
    pub fn overview_json(&self) -> serde_json::Value {
        let shown = self.overview_shown(self.animation_time());
        let Some(open) = self.overview.open.as_ref() else {
            return serde_json::json!({ "open": false, "shown": shown });
        };
        let rect = |r: Rectangle<i32, Logical>| serde_json::json!({ "x": r.loc.x, "y": r.loc.y, "w": r.size.w, "h": r.size.h });
        serde_json::json!({
            "open": true,
            "shown": shown,
            "selected": open.selected.as_ref().map(crate::foreign_toplevel::app_id),
            "slots": self.overview_slots().iter().map(|slot| serde_json::json!({
                "app_id": crate::foreign_toplevel::app_id(&slot.window),
                "output": slot.output.name(),
                "rect": rect(slot.rect),
            })).collect::<Vec<_>>(),
        })
    }
}

/// `window`'s frame (geometry and title bar) with its geometry at `loc`.
fn frame_at(window: &Window, loc: Point<i32, Logical>) -> Rectangle<i32, Logical> {
    let geo = Rectangle::new(loc, window.geometry().size);
    crate::decoration::title_bar_rect(window, loc).map_or(geo, |bar| bar.merge(geo))
}

/// The selected window's outline around its place.
fn outline_rect(rect: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    Rectangle::new(
        rect.loc - Point::from((OUTLINE, OUTLINE)),
        rect.size + Size::from((2 * OUTLINE, 2 * OUTLINE)),
    )
}

/// What it draws behind the windows on the area `visible` (an output's;
/// everywhere if `None`), front to back, relative to `origin`: the selected
/// window's outline, then the dimming over what's below (the wallpaper).
pub fn overview_elements(
    renderer: &mut GlesRenderer,
    view: &SceneOverview,
    colors: &Colors,
    visible: Option<Rectangle<i32, Logical>>,
    origin: Point<i32, Logical>,
    scale: Scale<f64>,
) -> Vec<PilaRenderElement> {
    let mut buffers = view.buffers.borrow_mut();
    let buffers = &mut *buffers;
    let shows = |rect: Rectangle<i32, Logical>| visible.is_none_or(|area| area.overlaps(rect));
    let element = |buffer: &SolidColorBuffer, loc: Point<i32, Logical>| {
        SolidColorRenderElement::from_buffer(
            buffer,
            (loc - origin).to_physical_precise_round(scale),
            scale,
            view.shown,
            Kind::Unspecified,
        )
    };
    let mut elements = Vec::new();
    if let Some(rect) = view.selected.map(outline_rect).filter(|r| shows(*r)) {
        let color = crate::switcher_view::opaque(colors.snap_preview);
        let buffer = buffers
            .outline
            .get_or_insert_with(|| SolidColorBuffer::new(rect.size, color));
        buffer.update(rect.size, color);
        let solid = element(buffer, rect.loc);
        let shaders = view
            .rounding
            .and_then(|_| crate::rounded::Shaders::get(renderer));
        elements.push(match (view.rounding, shaders) {
            (Some((radius, corners)), Some(shaders)) => PilaRenderElement::RoundedSolid(
                crate::rounded::RoundedSolid::new(solid, &shaders, radius, corners, scale),
            ),
            _ => PilaRenderElement::Solid(solid),
        });
    }
    for (name, geo) in view.outputs.iter().filter(|(_, geo)| shows(*geo)) {
        let buffer = buffers
            .dim
            .entry(name.clone())
            .or_insert_with(|| SolidColorBuffer::new(geo.size, DIM));
        buffer.update(geo.size, DIM);
        elements.push(PilaRenderElement::Solid(element(buffer, geo.loc)));
    }
    elements
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    fn size(w: i32, h: i32) -> Size<i32, Logical> {
        (w, h).into()
    }

    #[test]
    fn places_never_overlap_stay_inside_keep_the_shape_and_never_grow() {
        let area = rect(10, 20, 1800, 1000);
        let shapes = [
            size(800, 600),
            size(1920, 1080),
            size(300, 900),
            size(640, 480),
            size(1200, 400),
        ];
        for n in 1..=20 {
            let sizes: Vec<_> = (0..n).map(|i| shapes[i % shapes.len()]).collect();
            let places = arrange(area, &sizes);
            assert_eq!(places.len(), n);
            for (i, (place, size)) in places.iter().zip(&sizes).enumerate() {
                assert!(area.contains_rect(*place), "{n}: {place:?} out of {area:?}");
                assert!(place.size.w <= size.w && place.size.h <= size.h, "grown");
                let (a, b) = (
                    f64::from(place.size.w) / f64::from(place.size.h),
                    f64::from(size.w) / f64::from(size.h),
                );
                assert!((a - b).abs() / b < 0.05, "{n}: shape {a} for {b}");
                for other in &places[i + 1..] {
                    assert!(!place.overlaps(*other), "{n}: {place:?} over {other:?}");
                }
            }
        }
    }

    #[test]
    fn small_windows_keep_their_size_centered() {
        let places = arrange(rect(0, 0, 1000, 800), &[size(200, 100), size(300, 100)]);
        assert_eq!(places[0].size, size(200, 100));
        assert_eq!(places[1].size, size(300, 100));
        // One row, centered: 200 + 32 + 300 = 532 wide.
        assert_eq!(places[0].loc, (234, 350).into());
        assert_eq!(places[1].loc, (234 + 200 + GAP, 350).into());
    }

    #[test]
    fn rows_are_chosen_to_show_windows_largest() {
        // Four screen-sized windows on a screen: two rows of two.
        let sizes = vec![size(1920, 1080); 4];
        let places = arrange(rect(0, 0, 1920, 1080), &sizes);
        assert_eq!(places[0].loc.y, places[1].loc.y);
        assert!(places[2].loc.y > places[0].loc.y);
        assert_eq!(places[2].loc.y, places[3].loc.y);
    }

    #[test]
    fn a_step_goes_to_the_nearest_place_that_way() {
        // A grid of two rows:  0 1 2
        //                        3 4
        let places = [
            rect(0, 0, 100, 100),
            rect(200, 0, 100, 100),
            rect(400, 0, 100, 100),
            rect(100, 200, 100, 100),
            rect(300, 200, 100, 100),
        ];
        assert_eq!(step(&places, 0, Direction::Right), Some(1));
        assert_eq!(step(&places, 1, Direction::Left), Some(0));
        assert_eq!(step(&places, 0, Direction::Left), None);
        assert_eq!(step(&places, 2, Direction::Down), Some(4));
        assert_eq!(step(&places, 3, Direction::Up), Some(0));
        assert_eq!(step(&places, 3, Direction::Right), Some(4));
        assert_eq!(step(&places, 4, Direction::Down), None);
    }
}

//! Input methods (IME): text-input-v3 for apps, input-method-v2 for the
//! input method itself (fcitx5 and the like), both Smithay's.
//!
//! Smithay follows the keyboard focus: a surface given it gets the text
//! input's `enter`, which activates the input method once the app enables
//! it, and losing it deactivates the input method. Pila adds:
//!
//! - The candidate window (the input method's popup surface) is tracked by
//!   the `PopupManager` as `PopupKind::InputMethod`, so it's drawn, hit and
//!   redrawn with its window or layer surface, and placed by `place_popup`:
//!   below the text cursor, above it if there's no room, slid sideways to
//!   stay on the output (Smithay puts it on the cursor, over the text).
//! - The input method's keyboard grab sees keys after Pila's bindings; the
//!   keys it hands back through a virtual keyboard of its own go straight
//!   to the focus (`virtual_keyboard.rs`), and so do all keys while the
//!   session is locked (`input.rs`). A locked session never activates it
//!   (`focus.rs`).
//! - Keyboard grabs aren't only menus' any more: a menu opened while the
//!   input method holds the keyboard takes the pointer only
//!   (`handlers/xdg_shell.rs`), and Pila only ends a menu's keyboard grab
//!   (`Pila::keyboard_popup_grab`).

use std::cell::Cell;

use smithay::{
    desktop::{
        find_popup_root_surface, get_popup_toplevel_coords, layer_map_for_output,
        utils::bbox_from_surface_tree, PopupKind, PopupManager,
    },
    output::Output,
    reexports::wayland_server::{protocol::wl_surface::WlSurface, DisplayHandle},
    utils::{Logical, Point, Rectangle, Size},
    wayland::{
        compositor::with_states,
        input_method::{InputMethodHandler, InputMethodManagerState, PopupSurface},
        seat::WaylandFocus,
        text_input::{TextInputActivation, TextInputManagerState},
    },
};
use tracing::warn;

use crate::{redraw::Content, state::is_privileged, Pila};

/// Create the globals: text-input for every client (sandboxed apps type
/// too), input-method only for privileged ones (it reads what's typed
/// everywhere).
pub fn init_globals(dh: &DisplayHandle) {
    TextInputManagerState::new::<Pila>(dh);
    InputMethodManagerState::new::<Pila, _>(dh, is_privileged);
}

/// Where a candidate window whose surface tree's box is `size`, at `offset`
/// from its origin, goes for the text cursor `cursor`, both in the parent
/// surface's coordinates, inside `bounds` (the room on the output, in the
/// same coordinates): below the cursor, else above it (never past the top),
/// and as far left as it needs to fit, never past the left. Returns the
/// popup's origin.
pub fn place_popup(
    cursor: Rectangle<i32, Logical>,
    offset: Point<i32, Logical>,
    size: Size<i32, Logical>,
    bounds: Rectangle<i32, Logical>,
) -> Point<i32, Logical> {
    let right = bounds.loc.x + bounds.size.w;
    let bottom = bounds.loc.y + bounds.size.h;
    // The box's top-left corner, then the origin from it.
    let mut x = cursor.loc.x;
    x -= (x + size.w - right).max(0);
    x = x.max(bounds.loc.x);
    let below = cursor.loc.y + cursor.size.h;
    let y = if below + size.h <= bottom {
        below
    } else {
        // Fitting neither way, from the top edge.
        (cursor.loc.y - size.h).max(bounds.loc.y)
    };
    Point::from((x - offset.x, y - offset.y))
}

impl Pila {
    /// Where a candidate window's parent surface starts (global), and the
    /// output its root is on. The parent is the surface with the keyboard:
    /// a window's or a layer surface's, or a popup of either (a popover with
    /// a text field).
    fn popup_parent_origin(
        &self,
        popup: &PopupSurface,
    ) -> Option<(Point<i32, Logical>, Option<Output>)> {
        let parent = popup.get_parent()?;
        let kind = PopupKind::from(popup.clone());
        let root = find_popup_root_surface(&kind).ok()?;
        // Popups are placed from their root's geometry.
        let (root_origin, output) = if let Some(window) = self.window_for_surface(&root) {
            (
                self.space.element_location(&window)?,
                self.window_output(&window),
            )
        } else {
            let (output, layer) = self.layer_for_surface(&root)?;
            let layer_loc = layer_map_for_output(&output).layer_geometry(&layer)?.loc;
            let output_loc = self.space.output_geometry(&output)?.loc;
            (output_loc + layer_loc, Some(output))
        };
        // Down the popups between the root and the parent, then from the
        // parent's geometry to its surface (`parent_geometry`).
        let origin = root_origin + get_popup_toplevel_coords(&kind) - parent.location.loc;
        Some((origin, output))
    }

    /// The room a candidate window has, in its parent's coordinates: the
    /// output its text cursor is on (a window may span two), without the
    /// panels drawn above windows, which would cover it; all of it for a
    /// layer surface's, drawn with its layer.
    fn popup_bounds(
        &self,
        popup: &PopupSurface,
        cursor: Rectangle<i32, Logical>,
    ) -> Option<Rectangle<i32, Logical>> {
        let (origin, root_output) = self.popup_parent_origin(popup)?;
        let output = self
            .space
            .output_under((origin + cursor.loc).to_f64())
            .next()
            .cloned()
            .or(root_output)?;
        let root = find_popup_root_surface(&PopupKind::from(popup.clone())).ok()?;
        let usable = self.layer_for_surface(&root).is_none() && !self.top_layer_covered(&output);
        let mut bounds = self.output_area(&output, usable)?;
        bounds.loc -= origin;
        Some(bounds)
    }

    /// Put a candidate window where `place_popup` says and, if that moves
    /// it from where Pila last put it (`PlacedAt`), redraw where it was and
    /// where it is: Smithay puts it on the text cursor whenever the app moves
    /// that, and a new one may have a buffer already (Smithay makes it again
    /// when the input method activates). A commit of its own is redrawn by
    /// the commit handler.
    pub fn place_input_method_popup(&mut self, popup: &PopupSurface) {
        let Some(parent) = popup.get_parent().map(|parent| parent.surface.clone()) else {
            return;
        };
        let cursor = popup.text_input_rectangle();
        let Some(bounds) = self.popup_bounds(popup, cursor) else {
            return;
        };
        let tree = bbox_from_surface_tree(popup.wl_surface(), (0, 0));
        let at = place_popup(cursor, tree.loc, tree.size, bounds);
        let placed = with_states(popup.wl_surface(), |states| {
            states
                .data_map
                .get_or_insert(PlacedAt::default)
                .0
                .replace(Some(at))
        });
        if placed == Some(at) {
            popup.set_location(at);
            return;
        }
        let before = placed.and_then(|placed| {
            popup.set_location(placed);
            self.surface_outputs(&parent)
        });
        popup.set_location(at);
        self.redraw_parent(&parent, before);
    }

    /// A change of Pila's own to a candidate window of `parent`: the outputs
    /// it showed on (`before`) and shows on now, at once.
    fn redraw_parent(&mut self, parent: &WlSurface, before: Option<Vec<Output>>) {
        let mut outputs = before.unwrap_or_default();
        for output in self.surface_outputs(parent).unwrap_or_default() {
            if !outputs.contains(&output) {
                outputs.push(output);
            }
        }
        for output in outputs {
            self.request_redraw_as(&output, Content::Sure);
        }
    }

    /// The candidate windows shown, for tests (`Pila::state_json`): their
    /// parent's name and their surface tree's box, global, as drawn.
    pub fn input_method_popups(&self) -> Vec<serde_json::Value> {
        use smithay::wayland::shell::wlr_layer::Layer;
        let windows = self.space.elements().filter_map(|window| {
            let surface = window.wl_surface()?.into_owned();
            let loc = self.space.element_location(window)?;
            let name = crate::foreign_toplevel::title_and_app_id(window).1;
            // Popups are drawn from the window's geometry (`render.rs`).
            Some((surface, loc, name))
        });
        let layers = self
            .layer_surfaces(&[Layer::Overlay, Layer::Top, Layer::Bottom, Layer::Background])
            .into_iter()
            .map(|(layer, loc)| {
                let name = layer.namespace().to_string();
                (layer.wl_surface().clone(), loc, name)
            });
        let mut popups = Vec::new();
        for (surface, origin, name) in windows.collect::<Vec<_>>().into_iter().chain(layers) {
            for (popup, offset) in PopupManager::popups_for_surface(&surface) {
                if !matches!(popup, PopupKind::InputMethod(_)) {
                    continue;
                }
                let at = origin + offset - popup.geometry().loc;
                let rect = bbox_from_surface_tree(popup.wl_surface(), at);
                popups.push(serde_json::json!({
                    "parent": name,
                    "geometry": [rect.loc.x, rect.loc.y, rect.size.w, rect.size.h],
                }));
            }
        }
        popups
    }
}

/// Where Pila last put a candidate window (in its parent's coordinates), in
/// its surface's data: Smithay moves it before telling Pila.
#[derive(Default)]
struct PlacedAt(Cell<Option<Point<i32, Logical>>>);

impl InputMethodHandler for Pila {
    fn new_popup(&mut self, surface: PopupSurface) {
        if let Err(err) = self.popups.track_popup(PopupKind::from(surface.clone())) {
            warn!(?err, "can't track an input method's popup");
            return;
        }
        // Its first buffer at its output's scale, as a window's.
        if let Some(parent) = surface.get_parent().map(|parent| parent.surface.clone()) {
            if let Some(output) = self.output_for_surface(&parent) {
                crate::scale::send_scale_to_tree(surface.wl_surface(), &output);
            }
        }
        self.place_input_method_popup(&surface);
    }

    fn popup_repositioned(&mut self, surface: PopupSurface) {
        self.place_input_method_popup(&surface);
    }

    fn dismiss_popup(&mut self, surface: PopupSurface) {
        let Some(parent) = surface.get_parent().map(|parent| parent.surface.clone()) else {
            return;
        };
        let before = self.surface_outputs(&parent);
        // Shown again, it's placed and redrawn anew: Smithay dismisses it
        // again when the input method activates, but by then without a
        // parent, which is nothing to Pila.
        with_states(surface.wl_surface(), |states| {
            if let Some(placed) = states.data_map.get::<PlacedAt>() {
                placed.0.set(None);
            }
        });
        // The tree of popups is its root's, which for a text field in a
        // popup isn't the parent.
        let kind = PopupKind::from(surface);
        let root = find_popup_root_surface(&kind).unwrap_or_else(|_| parent.clone());
        let _ = PopupManager::dismiss_popup(&root, &kind);
        self.redraw_parent(&parent, before);
    }

    /// The parent's geometry in its surface: popups are drawn from their
    /// root's geometry, at their place less their own `geometry().loc`
    /// (`render.rs`), and an input method's `geometry()` is this rectangle,
    /// so its location is in the parent surface's coordinates, as the text
    /// cursor's.
    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, Logical> {
        if let Some(window) = self.window_for_surface(parent) {
            return window.geometry();
        }
        // A popup's own geometry (a popover's shadow is outside it); a layer
        // surface has none.
        match self.popups.find_popup(parent) {
            Some(popup @ PopupKind::Xdg(_)) => popup.geometry(),
            _ => Rectangle::default(),
        }
    }
}

impl TextInputActivation for Pila {}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    const OUTPUT: (i32, i32, i32, i32) = (0, 0, 1000, 800);

    fn place(cursor: Rectangle<i32, Logical>, size: (i32, i32)) -> (i32, i32) {
        let (x, y, w, h) = OUTPUT;
        let p = place_popup(cursor, (0, 0).into(), size.into(), rect(x, y, w, h));
        (p.x, p.y)
    }

    #[test]
    fn a_candidate_window_goes_below_the_text_cursor() {
        assert_eq!(place(rect(100, 200, 2, 20), (300, 40)), (100, 220));
    }

    #[test]
    fn a_candidate_window_goes_above_the_text_cursor_with_no_room_below() {
        assert_eq!(place(rect(100, 770, 2, 20), (300, 40)), (100, 730));
    }

    #[test]
    fn a_candidate_window_fitting_neither_way_starts_at_the_top() {
        assert_eq!(place(rect(100, 400, 2, 20), (300, 700)), (100, 0));
    }

    #[test]
    fn a_candidate_window_slides_left_to_stay_on_the_output() {
        assert_eq!(place(rect(900, 200, 2, 20), (300, 40)), (700, 220));
    }

    #[test]
    fn a_candidate_window_wider_than_the_output_starts_at_its_left_edge() {
        assert_eq!(place(rect(900, 200, 2, 20), (1200, 40)), (0, 220));
    }

    #[test]
    fn a_candidate_window_is_placed_in_its_parents_coordinates() {
        // The parent's surface starts at (-50, -60) of the output's.
        let bounds = rect(-50, -60, 1000, 800);
        let p = place_popup(rect(10, 10, 2, 20), (0, 0).into(), (100, 40).into(), bounds);
        assert_eq!((p.x, p.y), (10, 30));
        let p = place_popup(
            rect(10, 720, 2, 20),
            (0, 0).into(),
            (100, 40).into(),
            bounds,
        );
        assert_eq!((p.x, p.y), (10, 680));
    }

    #[test]
    fn a_candidate_windows_shadow_offset_is_kept() {
        // Its tree's box starts 10 px left of and above its origin (a
        // shadow): the box, not the origin, goes below the cursor.
        let p = place_popup(
            rect(100, 200, 2, 20),
            (-10, -10).into(),
            (320, 60).into(),
            rect(0, 0, 1000, 800),
        );
        assert_eq!((p.x, p.y), (110, 230));
    }
}

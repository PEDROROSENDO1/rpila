//! wp_pointer_warp_v1: a client moves the pointer within its own surface
//! (games and SDL 3 recentering it, apps putting it on a default button).
//!
//! Only the client the pointer is on, with the serial of the pointer's
//! latest enter, onto a place where the pointer stays on that surface (or a
//! subsurface of its tree): never onto another client, nor while the screen
//! is locked. The move is `Pila::warp_pointer`'s, which leaves a pointer a
//! constraint holds where it is (a locked client says where with the
//! constraint's cursor hint instead) and tells clients as any motion; it
//! isn't the user's activity.

use smithay::{
    reexports::wayland_server::protocol::{wl_pointer::WlPointer, wl_surface::WlSurface},
    utils::{Logical, Point, Serial},
    wayland::pointer_warp::PointerWarpHandler,
};
use tracing::debug;

use crate::{redraw::root_surface, Pila};

impl PointerWarpHandler for Pila {
    fn warp_pointer(
        &mut self,
        surface: WlSurface,
        _pointer: WlPointer,
        pos: Point<f64, Logical>,
        serial: Serial,
    ) {
        if let Some(target) = self.pointer_warp_target(&surface, pos, serial) {
            self.warp_pointer(target);
        } else {
            debug!(?pos, "pointer warp refused");
        }
    }
}

impl Pila {
    /// Where a warp of the pointer to `pos` on `surface` puts it, if the
    /// client may.
    fn pointer_warp_target(
        &self,
        surface: &WlSurface,
        pos: Point<f64, Logical>,
        serial: Serial,
    ) -> Option<Point<f64, Logical>> {
        if self.is_locked() {
            return None;
        }
        let pointer = self.seat.get_pointer()?;
        if pointer.last_enter() != Some(serial) {
            return None;
        }
        // The pointer is on `focused`, at `origin`: `surface` or a surface of
        // its tree.
        let (focused, origin) = self.surface_under(pointer.current_location())?;
        let origin = if focused == *surface {
            origin
        } else if root_surface(&focused) == *surface {
            self.surface_origin(surface)?.0
        } else {
            return None;
        };
        let target = origin + pos;
        let (under, _) = self.surface_under(target)?;
        (root_surface(&under) == root_surface(surface)).then_some(target)
    }
}

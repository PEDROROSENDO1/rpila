//! wp_tearing_control_v1: a client says a surface's frames may tear (shown
//! at once, mid-refresh) rather than wait for the next vblank: games with
//! vsync off, through Mesa's Vulkan and EGL.
//!
//! The hint is double-buffered surface state (`TearingHint`). Pila honors it
//! on bare metal for a monitor whose frame is a fullscreen window that asks
//! for it (`Scene::tears`, `tearing` in the config, never while locked): the
//! frame goes out with an asynchronous page flip. Smithay flips that way only
//! when nothing but the primary plane changed (the pointer moving over the
//! game is a normal flip), and goes back to a normal flip, for as long as the
//! plane stays the same, if the driver refuses one.

use std::sync::Mutex;

use smithay::{
    reexports::{
        wayland_protocols::wp::tearing_control::v1::server::{
            wp_tearing_control_manager_v1::{self as manager, WpTearingControlManagerV1},
            wp_tearing_control_v1::{self as control, PresentationHint, WpTearingControlV1},
        },
        wayland_server::{
            backend::ClientId, protocol::wl_surface::WlSurface, Client, DataInit, Dispatch,
            DisplayHandle, GlobalDispatch, New, Resource, WEnum,
        },
    },
    wayland::compositor::{with_states, Cacheable},
};

use crate::{sync::Locked, Pila};

/// The surface's presentation hint, double-buffered with the surface state:
/// whether its frames may tear.
#[derive(Default, Clone, Copy, Debug)]
pub struct TearingHint(bool);

impl Cacheable for TearingHint {
    fn commit(&mut self, _dh: &DisplayHandle) -> Self {
        *self
    }
    fn merge_into(self, into: &mut Self, _dh: &DisplayHandle) {
        *into = self;
    }
}

/// The surface's tearing control object, if it has one (one at a time).
#[derive(Default)]
struct TearingControl(Mutex<Option<WpTearingControlV1>>);

/// Create the global. Called once from `protocols::init_globals`.
pub fn init_global(dh: &DisplayHandle) {
    dh.create_global::<Pila, WpTearingControlManagerV1, ()>(1, ());
}

/// Whether `surface`'s committed state says its frames may tear.
pub fn wants_async(surface: &WlSurface) -> bool {
    with_states(surface, |states| {
        states.cached_state.get::<TearingHint>().current().0
    })
}

impl GlobalDispatch<WpTearingControlManagerV1, ()> for Pila {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<WpTearingControlManagerV1>,
        _: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<WpTearingControlManagerV1, ()> for Pila {
    fn request(
        _: &mut Self,
        _: &Client,
        resource: &WpTearingControlManagerV1,
        request: manager::Request,
        _: &(),
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        let manager::Request::GetTearingControl { id, surface } = request else {
            return;
        };
        let exists = with_states(&surface, |states| {
            let slot = states
                .data_map
                .get_or_insert_threadsafe(TearingControl::default);
            slot.0.locked().as_ref().is_some_and(|c| c.is_alive())
        });
        if exists {
            resource.post_error(
                manager::Error::TearingControlExists,
                "the surface already has a tearing control object",
            );
            return;
        }
        let control = data_init.init(id, surface.clone());
        with_states(&surface, |states| {
            if let Some(slot) = states.data_map.get::<TearingControl>() {
                *slot.0.locked() = Some(control);
            }
        });
    }
}

impl Dispatch<WpTearingControlV1, WlSurface> for Pila {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WpTearingControlV1,
        request: control::Request,
        surface: &WlSurface,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        let control::Request::SetPresentationHint { hint } = request else {
            return;
        };
        // "If the associated wl_surface is destroyed, this object becomes
        // inert."
        if !surface.is_alive() {
            return;
        }
        let tears = matches!(hint, WEnum::Value(PresentationHint::Async));
        with_states(surface, |states| {
            *states.cached_state.get::<TearingHint>().pending() = TearingHint(tears);
        });
    }

    fn destroyed(_: &mut Self, _: ClientId, _: &WpTearingControlV1, surface: &WlSurface) {
        if !surface.is_alive() {
            return;
        }
        // "Revert the presentation hint to vsync. The change will be applied
        // on the next wl_surface.commit."
        with_states(surface, |states| {
            *states.cached_state.get::<TearingHint>().pending() = TearingHint(false);
            if let Some(slot) = states.data_map.get::<TearingControl>() {
                *slot.0.locked() = None;
            }
        });
    }
}

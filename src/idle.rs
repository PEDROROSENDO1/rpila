//! ext-idle-notify and idle-inhibit: tell clients such as swayidle when the
//! user has been idle, unless something visible (a video player) inhibits it.
//!
//! Every input event counts as activity (`Pila::process_input_event`), told
//! the notifier once per event-loop iteration (`Pila::after_frames`).
//! Smithay runs the per-notification timers on `Pila::loop_handle`. An idle
//! inhibitor only counts while its surface is visible: part of a mapped window
//! or layer surface, and the session not locked. `update_idle_inhibit`
//! re-checks that after every frame.
//!
//! Pila's own idle action, `[idle] monitors-off`: the monitors turn off after
//! that long without input, unless an inhibitor counts
//! (`Pila::power_off_monitors`; any input wakes them, `Pila::wake_monitors`).
//! One timer, armed at startup and by input once it has fired, moved only
//! when it fires (`monitors_off_due`): no work per input event but the
//! time it came.

use std::{collections::HashSet, time::Duration, time::Instant};

use smithay::{
    reexports::{
        calloop::{
            timer::{TimeoutAction, Timer},
            RegistrationToken,
        },
        wayland_server::{protocol::wl_surface::WlSurface, Resource},
    },
    wayland::{
        idle_inhibit::IdleInhibitHandler,
        idle_notify::{IdleNotifierHandler, IdleNotifierState},
    },
};

use crate::Pila;

/// Surfaces with an idle inhibitor, visible or not, and whether one counts
/// now.
#[derive(Default)]
pub struct IdleInhibitors {
    surfaces: HashSet<WlSurface>,
    inhibited: bool,
}

/// `[idle] monitors-off`'s timer.
pub struct MonitorsOff {
    /// The last input (or inhibitor gone, or start).
    last_activity: Instant,
    timer: Option<RegistrationToken>,
}

impl Default for MonitorsOff {
    fn default() -> Self {
        MonitorsOff {
            last_activity: Instant::now(),
            timer: None,
        }
    }
}

impl Pila {
    /// Count user input as activity: idle notifications resume and restart
    /// their timeouts, and so does `[idle] monitors-off`'s.
    pub fn notify_activity(&mut self) {
        let seat = self.seat.clone();
        self.globals.idle_notifier.notify_activity(&seat);
        self.monitors_off.last_activity = Instant::now();
        if self.monitors_off.timer.is_none() {
            self.arm_monitors_off();
        }
    }

    /// Recompute whether a visible surface inhibits idle. Call after each frame.
    pub fn update_idle_inhibit(&mut self) {
        self.idle_inhibitors
            .surfaces
            .retain(|surface| surface.is_alive());
        let inhibited = !self.is_locked()
            && self
                .idle_inhibitors
                .surfaces
                .iter()
                .any(|surface| self.is_visible(surface));
        // A video that stops counts as activity: the monitors don't go off
        // the moment it ends.
        if self.idle_inhibitors.inhibited && !inhibited {
            self.monitors_off.last_activity = Instant::now();
        }
        self.idle_inhibitors.inhibited = inhibited;
        // A no-op unless it changed.
        self.globals.idle_notifier.set_is_inhibited(inhibited);
    }

    /// `[idle]` set or changed (and at startup): the timer again, from now.
    pub fn idle_config_changed(&mut self) {
        if let Some(token) = self.monitors_off.timer.take() {
            self.loop_handle.remove(token);
        }
        self.monitors_off.last_activity = Instant::now();
        self.arm_monitors_off();
    }

    /// The timer, due `monitors-off` after the last activity; none if
    /// `monitors-off = false`.
    fn arm_monitors_off(&mut self) {
        let Some(seconds) = self.config.idle.monitors_off else {
            return;
        };
        let due = self.monitors_off.last_activity + Duration::from_secs(seconds.into());
        let timer = Timer::from_deadline(due);
        self.monitors_off.timer = self
            .loop_handle
            .insert_source(timer, |_, _, state| state.monitors_off_due())
            .ok();
    }

    /// The timer fired: later if there was input since or an inhibitor
    /// counts, else the monitors off until the next input (which arms it
    /// again).
    fn monitors_off_due(&mut self) -> TimeoutAction {
        let Some(seconds) = self.config.idle.monitors_off else {
            self.monitors_off.timer = None;
            return TimeoutAction::Drop;
        };
        let timeout = Duration::from_secs(seconds.into());
        if self.idle_inhibitors.inhibited {
            return TimeoutAction::ToDuration(timeout);
        }
        let now = Instant::now();
        let due = self.monitors_off.last_activity + timeout;
        if now < due {
            return TimeoutAction::ToDuration(due - now);
        }
        tracing::info!(seconds, "no input: turning the monitors off");
        self.power_off_monitors();
        self.monitors_off.timer = None;
        TimeoutAction::Drop
    }

    /// Whether `surface` belongs to a mapped window or a layer surface.
    fn is_visible(&self, surface: &WlSurface) -> bool {
        let root = crate::redraw::root_surface(surface);
        let mapped_window = self
            .space
            .elements()
            .any(|w| w.toplevel().is_some_and(|t| *t.wl_surface() == root));
        mapped_window || self.layer_for_surface(&root).is_some()
    }
}

// ext_idle_notifier_v1
impl IdleNotifierHandler for Pila {
    fn idle_notifier_state(&mut self) -> &mut IdleNotifierState<Self> {
        &mut self.globals.idle_notifier
    }
}

// zwp_idle_inhibit_manager_v1
impl IdleInhibitHandler for Pila {
    fn inhibit(&mut self, surface: WlSurface) {
        self.idle_inhibitors.surfaces.insert(surface);
        self.update_idle_inhibit();
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.idle_inhibitors.surfaces.remove(&surface);
        self.update_idle_inhibit();
    }
}

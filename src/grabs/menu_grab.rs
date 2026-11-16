//! A menu's pointer grab: Smithay's `PopupPointerGrab`, which gives the
//! pointer only to the menu's client and dismisses the menu on a press
//! anywhere else, except over an input method's candidate window. A text
//! field in a popover (a GTK search field) types through the input method,
//! whose candidate window is another client's surface: it gets the pointer,
//! hover and clicks, and the popover stays open.

use smithay::{
    desktop::{PopupKind, PopupPointerGrab},
    input::pointer::{
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
        GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent,
        GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData as PointerGrabStartData,
        MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
    },
    utils::{Logical, Point},
    wayland::seat::WaylandFocus,
};

use crate::{focus::PointerFocus, Pila};

pub struct MenuPointerGrab {
    inner: PopupPointerGrab<Pila>,
}

impl MenuPointerGrab {
    pub fn new(inner: PopupPointerGrab<Pila>) -> Self {
        Self { inner }
    }
}

/// Whether `focus` is an input method's candidate window.
fn is_candidate_window(data: &Pila, focus: &PointerFocus) -> bool {
    focus.wl_surface().is_some_and(|surface| {
        matches!(
            data.popups.find_popup(&surface),
            Some(PopupKind::InputMethod(_))
        )
    })
}

impl PointerGrab<Pila> for MenuPointerGrab {
    fn motion(
        &mut self,
        data: &mut Pila,
        handle: &mut PointerInnerHandle<'_, Pila>,
        focus: Option<(PointerFocus, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        if focus
            .as_ref()
            .is_some_and(|(focus, _)| is_candidate_window(data, focus))
        {
            handle.motion(data, focus, event);
        } else {
            self.inner.motion(data, handle, focus, event);
        }
    }

    fn relative_motion(
        &mut self,
        data: &mut Pila,
        handle: &mut PointerInnerHandle<'_, Pila>,
        focus: Option<(PointerFocus, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        self.inner.relative_motion(data, handle, focus, event);
    }

    fn button(
        &mut self,
        data: &mut Pila,
        handle: &mut PointerInnerHandle<'_, Pila>,
        event: &ButtonEvent,
    ) {
        if handle
            .current_focus()
            .is_some_and(|(focus, _)| is_candidate_window(data, &focus))
        {
            handle.button(data, event);
        } else {
            self.inner.button(data, handle, event);
        }
    }

    fn axis(
        &mut self,
        data: &mut Pila,
        handle: &mut PointerInnerHandle<'_, Pila>,
        details: AxisFrame,
    ) {
        self.inner.axis(data, handle, details);
    }

    fn frame(&mut self, data: &mut Pila, handle: &mut PointerInnerHandle<'_, Pila>) {
        self.inner.frame(data, handle);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut Pila,
        handle: &mut PointerInnerHandle<'_, Pila>,
        event: &GestureSwipeBeginEvent,
    ) {
        self.inner.gesture_swipe_begin(data, handle, event);
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut Pila,
        handle: &mut PointerInnerHandle<'_, Pila>,
        event: &GestureSwipeUpdateEvent,
    ) {
        self.inner.gesture_swipe_update(data, handle, event);
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut Pila,
        handle: &mut PointerInnerHandle<'_, Pila>,
        event: &GestureSwipeEndEvent,
    ) {
        self.inner.gesture_swipe_end(data, handle, event);
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut Pila,
        handle: &mut PointerInnerHandle<'_, Pila>,
        event: &GesturePinchBeginEvent,
    ) {
        self.inner.gesture_pinch_begin(data, handle, event);
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut Pila,
        handle: &mut PointerInnerHandle<'_, Pila>,
        event: &GesturePinchUpdateEvent,
    ) {
        self.inner.gesture_pinch_update(data, handle, event);
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut Pila,
        handle: &mut PointerInnerHandle<'_, Pila>,
        event: &GesturePinchEndEvent,
    ) {
        self.inner.gesture_pinch_end(data, handle, event);
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut Pila,
        handle: &mut PointerInnerHandle<'_, Pila>,
        event: &GestureHoldBeginEvent,
    ) {
        self.inner.gesture_hold_begin(data, handle, event);
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut Pila,
        handle: &mut PointerInnerHandle<'_, Pila>,
        event: &GestureHoldEndEvent,
    ) {
        self.inner.gesture_hold_end(data, handle, event);
    }

    fn start_data(&self) -> &PointerGrabStartData<Pila> {
        self.inner.start_data()
    }

    fn unset(&mut self, data: &mut Pila) {
        self.inner.unset(data);
    }
}

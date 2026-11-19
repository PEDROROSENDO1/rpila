//! A picture on the clipboard: an ext-data-control source offering
//! `image/png`, set as the seat's selection, whose bytes are written to
//! whoever pastes them, until something else takes the clipboard. Like
//! wl-copy, the process stays for that long: a Wayland clipboard holds
//! offers, not data.

use std::{fs::File, io::Write};

use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_registry::WlRegistry, wl_seat::WlSeat},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
    ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_offer_v1::ExtDataControlOfferV1,
    ext_data_control_source_v1::{self, ExtDataControlSourceV1},
};

pub const MIME: &str = "image/png";

struct State {
    png: Vec<u8>,
    cancelled: bool,
}

/// Put `png` on the clipboard and serve it until it's replaced; `Err` if
/// it couldn't be put there. `offered` runs once it is.
pub fn serve(png: Vec<u8>, offered: impl FnOnce()) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|err| format!("no Wayland: {err}"))?;
    let (globals, mut queue) =
        registry_queue_init::<State>(&conn).map_err(|err| err.to_string())?;
    let qh = queue.handle();
    let missing = |what: &str| format!("the compositor has no {what}");
    let manager: ExtDataControlManagerV1 = globals
        .bind(&qh, 1..=1, ())
        .map_err(|_| missing("ext-data-control"))?;
    let seat: WlSeat = globals.bind(&qh, 1..=1, ()).map_err(|_| missing("seat"))?;
    let device = manager.get_data_device(&seat, &qh, ());
    let source = manager.create_data_source(&qh, ());
    source.offer(MIME.to_string());
    device.set_selection(Some(&source));
    let mut state = State {
        png,
        cancelled: false,
    };
    queue.roundtrip(&mut state).map_err(|err| err.to_string())?;
    if state.cancelled {
        return Err("the clipboard was taken at once".into());
    }
    offered();
    while !state.cancelled {
        queue
            .blocking_dispatch(&mut state)
            .map_err(|err| err.to_string())?;
    }
    Ok(())
}

impl Dispatch<ExtDataControlSourceV1, ()> for State {
    fn event(
        state: &mut Self,
        source: &ExtDataControlSourceV1,
        event: ext_data_control_source_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_source_v1::Event::Send { mime_type, fd } => {
                if mime_type == MIME {
                    // A reader that goes away early is its business.
                    let _ = File::from(fd).write_all(&state.png);
                }
            }
            ext_data_control_source_v1::Event::Cancelled => {
                source.destroy();
                state.cancelled = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtDataControlDeviceV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Others' offers aren't read: dropped as they come.
        if let ext_data_control_device_v1::Event::DataOffer { id } = event {
            id.destroy();
        }
    }

    wayland_client::event_created_child!(State, ExtDataControlDeviceV1, [
        ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ExtDataControlOfferV1, ()),
    ]);
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// Objects with no events this client needs.
macro_rules! no_events {
    ($($proxy:ty),*) => {$(
        impl Dispatch<$proxy, ()> for State {
            fn event(
                _: &mut Self,
                _: &$proxy,
                _: <$proxy as wayland_client::Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )*};
}

no_events!(WlSeat, ExtDataControlManagerV1, ExtDataControlOfferV1);

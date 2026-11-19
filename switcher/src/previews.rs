//! The windows' previews: a still of each, captured once when a switch
//! starts (ext-image-copy-capture of an ext-foreign-toplevel-list window,
//! whose identifier is the IPC's window id), shrunk to a thumbnail.
//!
//! On a thread of its own and a Wayland connection of its own, opened for
//! that switch and closed after: nothing is held between switches, and Pila
//! copies one frame per window, one window at a time (a full-size buffer at
//! a time, however many windows), the selected one first. A window that
//! can't be captured within `TIMEOUT` keeps its icon.

use std::{
    os::unix::fs::FileExt,
    time::{Duration, Instant},
};

use crate::shm::{dispatch_until, shm_buffer};
use iced::widget::image;
use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_buffer::WlBuffer, wl_registry::WlRegistry, wl_shm, wl_shm_pool::WlShmPool},
    Connection, Dispatch, EventQueue, QueueHandle, WEnum,
};

use wayland_protocols::ext::{
    foreign_toplevel_list::v1::client::{
        ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
        ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
    },
    image_capture_source::v1::client::{
        ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1,
        ext_image_capture_source_v1::ExtImageCaptureSourceV1,
    },
    image_copy_capture::v1::client::{
        ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
        ext_image_copy_capture_manager_v1::{self, ExtImageCopyCaptureManagerV1},
        ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
    },
};

/// The largest thumbnail, in pixels: twice a tile's preview, sharp at scale 2.
pub const THUMBNAIL: (u32, u32) = (2 * 184, 2 * 115);
/// How long one window's capture may take.
const TIMEOUT: Duration = Duration::from_secs(1);
/// The most windows captured for one switch.
const MAX_WINDOWS: usize = 24;

/// A window's preview, by its id.
pub type Preview = (String, image::Handle);

/// The previews of the windows `ids`, captured in that order on a thread,
/// each as it's ready. None where Pila (or another compositor) doesn't let
/// this client capture windows.
pub fn capture(ids: Vec<String>) -> impl iced::futures::Stream<Item = Preview> {
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let spawned = std::thread::Builder::new()
        .name("pila-switcher-previews".into())
        .spawn(move || {
            if let Err(err) = capture_all(&ids, &mut |preview| {
                let _ = sender.unbounded_send(preview);
            }) {
                eprintln!("pila-switcher: no window previews: {err}");
            }
        });
    if let Err(err) = spawned {
        eprintln!("pila-switcher: no window previews: {err}");
    }
    receiver
}

#[derive(Default)]
struct State {
    /// The windows Pila lists, with their identifiers once told.
    toplevels: Vec<(ExtForeignToplevelHandleV1, Option<String>)>,
    session: SessionState,
    frame: FrameState,
}

/// What the current capture session said of its buffers.
#[derive(Default)]
struct SessionState {
    size: Option<(u32, u32)>,
    formats: Vec<wl_shm::Format>,
    done: bool,
    stopped: bool,
}

#[derive(Default, PartialEq, Eq)]
enum FrameState {
    #[default]
    Waiting,
    Ready,
    Failed,
}

fn capture_all(ids: &[String], send: &mut dyn FnMut(Preview)) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|err| err.to_string())?;
    let (globals, mut queue) =
        registry_queue_init::<State>(&conn).map_err(|err| err.to_string())?;
    let qh = queue.handle();
    let bind_err = |what: &str| format!("the compositor has no {what}");
    let shm: wl_shm::WlShm = globals
        .bind(&qh, 1..=1, ())
        .map_err(|_| bind_err("wl_shm"))?;
    let _list: ExtForeignToplevelListV1 = globals
        .bind(&qh, 1..=1, ())
        .map_err(|_| bind_err("ext-foreign-toplevel-list"))?;
    let sources: ExtForeignToplevelImageCaptureSourceManagerV1 = globals
        .bind(&qh, 1..=1, ())
        .map_err(|_| bind_err("window capture sources"))?;
    let copies: ExtImageCopyCaptureManagerV1 = globals
        .bind(&qh, 1..=1, ())
        .map_err(|_| bind_err("ext-image-copy-capture"))?;
    let mut state = State::default();
    // The windows and their identifiers.
    queue.roundtrip(&mut state).map_err(|err| err.to_string())?;
    for id in ids.iter().take(MAX_WINDOWS) {
        let Some(handle) = state
            .toplevels
            .iter()
            .find(|(_, identifier)| identifier.as_deref() == Some(id.as_str()))
            .map(|(handle, _)| handle.clone())
        else {
            continue;
        };
        if let Some(thumbnail) = capture_one(
            &conn, &mut queue, &mut state, &qh, &shm, &sources, &copies, &handle,
        ) {
            send((id.clone(), thumbnail));
        }
    }
    Ok(())
}

/// One window's still, as a thumbnail; `None` if it can't be had in time.
#[allow(clippy::too_many_arguments)]
fn capture_one(
    conn: &Connection,
    queue: &mut EventQueue<State>,
    state: &mut State,
    qh: &QueueHandle<State>,
    shm: &wl_shm::WlShm,
    sources: &ExtForeignToplevelImageCaptureSourceManagerV1,
    copies: &ExtImageCopyCaptureManagerV1,
    handle: &ExtForeignToplevelHandleV1,
) -> Option<image::Handle> {
    let deadline = Instant::now() + TIMEOUT;
    let source = sources.create_source(handle, qh, ());
    state.session = SessionState::default();
    let session = copies.create_session(
        &source,
        ext_image_copy_capture_manager_v1::Options::empty(),
        qh,
        (),
    );
    let thumbnail = (|| {
        dispatch_until(conn, queue, state, deadline, |s| {
            s.session.done || s.session.stopped
        })?;
        let (width, height) = state.session.size.filter(|_| !state.session.stopped)?;
        // Pila's are little-endian BGRA, XRGB first.
        let format = [wl_shm::Format::Xrgb8888, wl_shm::Format::Argb8888]
            .into_iter()
            .find(|f| state.session.formats.contains(f))?;
        let (buffer, file) = shm_buffer(shm, qh, width, height, format)?;
        state.frame = FrameState::Waiting;
        let frame = session.create_frame(qh, ());
        frame.attach_buffer(&buffer);
        frame.damage_buffer(0, 0, width as i32, height as i32);
        frame.capture();
        let ready = dispatch_until(conn, queue, state, deadline, |s| {
            s.frame != FrameState::Waiting
        })
        .is_some()
            && state.frame == FrameState::Ready;
        frame.destroy();
        buffer.destroy();
        if !ready {
            return None;
        }
        let mut pixels = vec![0; (width * height * 4) as usize];
        file.read_exact_at(&mut pixels, 0).ok()?;
        Some(thumbnail_of(pixels, width, height))
    })();
    session.destroy();
    source.destroy();
    let _ = conn.flush();
    thumbnail
}

/// Little-endian BGRA (XRGB or ARGB) pixels as an opaque RGBA thumbnail no
/// larger than `THUMBNAIL`, keeping their aspect.
pub fn thumbnail_of(mut pixels: Vec<u8>, width: u32, height: u32) -> image::Handle {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    let (w, h) = fit((width, height), THUMBNAIL);
    let full = ::image::RgbaImage::from_raw(width, height, pixels);
    match full {
        Some(full) if (w, h) != (width, height) => {
            let small =
                ::image::imageops::resize(&full, w, h, ::image::imageops::FilterType::Triangle);
            image::Handle::from_rgba(w, h, small.into_raw())
        }
        Some(full) => image::Handle::from_rgba(width, height, full.into_raw()),
        None => image::Handle::from_rgba(1, 1, vec![0, 0, 0, 255]),
    }
}

/// `size` shrunk to fit in `bounds`, keeping its aspect; never grown, never
/// below a pixel.
pub fn fit(size: (u32, u32), bounds: (u32, u32)) -> (u32, u32) {
    let (w, h) = (size.0.max(1) as f64, size.1.max(1) as f64);
    let scale = (bounds.0 as f64 / w).min(bounds.1 as f64 / h).min(1.0);
    (
        ((w * scale).round() as u32).max(1),
        ((h * scale).round() as u32).max(1),
    )
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

impl Dispatch<ExtForeignToplevelListV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            state.toplevels.push((toplevel, None));
        }
    }

    wayland_client::event_created_child!(State, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_foreign_toplevel_handle_v1::Event::Identifier { identifier } => {
                if let Some(entry) = state.toplevels.iter_mut().find(|(h, _)| h == handle) {
                    entry.1 = Some(identifier);
                }
            }
            ext_foreign_toplevel_handle_v1::Event::Closed => {
                state.toplevels.retain(|(h, _)| h != handle);
                handle.destroy();
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_session_v1::Event;
        match event {
            Event::BufferSize { width, height } => state.session.size = Some((width, height)),
            Event::ShmFormat {
                format: WEnum::Value(format),
            } => state.session.formats.push(format),
            Event::Done => state.session.done = true,
            Event::Stopped => state.session.stopped = true,
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_frame_v1::Event::Ready => state.frame = FrameState::Ready,
            ext_image_copy_capture_frame_v1::Event::Failed { .. } => {
                state.frame = FrameState::Failed
            }
            _ => {}
        }
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

no_events!(
    wl_shm::WlShm,
    WlShmPool,
    WlBuffer,
    ExtForeignToplevelImageCaptureSourceManagerV1,
    ExtImageCaptureSourceV1,
    ExtImageCopyCaptureManagerV1
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thumbnail_keeps_the_aspect_and_never_grows() {
        assert_eq!(fit((1920, 1080), THUMBNAIL), (368, 207));
        assert_eq!(fit((800, 1200), THUMBNAIL), (153, 230));
        assert_eq!(fit((100, 50), THUMBNAIL), (100, 50));
        assert_eq!(fit((0, 0), THUMBNAIL), (1, 1));
        assert_eq!(fit((10000, 1), THUMBNAIL), (368, 1));
    }

    #[test]
    fn bgra_becomes_opaque_rgba() {
        // One pixel: blue 1, green 2, red 3, alpha 0 (XRGB's unused byte).
        let handle = thumbnail_of(vec![1, 2, 3, 0], 1, 1);
        match handle {
            image::Handle::Rgba { pixels, .. } => assert_eq!(&pixels[..], [3, 2, 1, 255]),
            other => panic!("not RGBA: {other:?}"),
        }
    }
}

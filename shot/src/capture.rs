//! The monitors as they are now: ext-image-copy-capture of each output
//! (ext-output-image-capture-source), without the cursor, on a Wayland
//! connection of its own that's closed after. Pila gives an output upright,
//! its transform applied, at its full resolution.

use std::{
    os::unix::fs::FileExt,
    time::{Duration, Instant},
};

use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{
        wl_buffer::WlBuffer,
        wl_output::{self, WlOutput},
        wl_registry::WlRegistry,
        wl_shm,
        wl_shm_pool::WlShmPool,
    },
    Connection, Dispatch, EventQueue, QueueHandle, WEnum,
};
use wayland_protocols::ext::{
    image_capture_source::v1::client::{
        ext_image_capture_source_v1::ExtImageCaptureSourceV1,
        ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
    },
    image_copy_capture::v1::client::{
        ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
        ext_image_copy_capture_manager_v1::{self, ExtImageCopyCaptureManagerV1},
        ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
    },
};

use crate::shm::{dispatch_until, shm_buffer};

/// How long one output's capture may take.
const TIMEOUT: Duration = Duration::from_secs(3);

/// A monitor's picture: its output's name, and its pixels, opaque RGBA,
/// row after row.
#[derive(Debug, Clone)]
pub struct Shot {
    pub output: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Default)]
struct State {
    /// The outputs, with their names once told.
    outputs: Vec<(WlOutput, Option<String>)>,
    session: SessionState,
    frame: FrameState,
}

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

/// The outputs named `only` (every one if empty), in the compositor's order.
pub fn outputs(only: &[String]) -> Result<Vec<Shot>, String> {
    let conn = Connection::connect_to_env().map_err(|err| format!("no Wayland: {err}"))?;
    let (globals, mut queue) =
        registry_queue_init::<State>(&conn).map_err(|err| err.to_string())?;
    let qh = queue.handle();
    let missing = |what: &str| format!("the compositor has no {what}");
    let shm: wl_shm::WlShm = globals
        .bind(&qh, 1..=1, ())
        .map_err(|_| missing("wl_shm"))?;
    let sources: ExtOutputImageCaptureSourceManagerV1 = globals
        .bind(&qh, 1..=1, ())
        .map_err(|_| missing("output capture sources (ext-image-capture-source)"))?;
    let copies: ExtImageCopyCaptureManagerV1 = globals
        .bind(&qh, 1..=1, ())
        .map_err(|_| missing("ext-image-copy-capture"))?;
    let mut state = State::default();
    for global in globals.contents().clone_list() {
        if global.interface == "wl_output" && global.version >= 4 {
            let output: WlOutput = globals.registry().bind(global.name, 4, &qh, ());
            state.outputs.push((output, None));
        }
    }
    // Their names.
    queue.roundtrip(&mut state).map_err(|err| err.to_string())?;
    let mut shots = Vec::new();
    for (output, name) in state.outputs.clone() {
        let Some(name) = name else { continue };
        if !only.is_empty() && !only.contains(&name) {
            continue;
        }
        let shot = capture_one(
            &conn, &mut queue, &mut state, &qh, &shm, &sources, &copies, &output,
        )
        .ok_or_else(|| format!("couldn't capture {name}"))?;
        let (width, height, rgba) = shot;
        shots.push(Shot {
            output: name,
            width,
            height,
            rgba,
        });
    }
    if let Some(name) = only.iter().find(|n| !shots.iter().any(|s| &s.output == *n)) {
        return Err(format!("no output named {name}"));
    }
    if shots.is_empty() {
        return Err("no output to capture".into());
    }
    Ok(shots)
}

/// One output's picture: its size and opaque RGBA pixels; `None` if it
/// can't be had in time.
#[allow(clippy::too_many_arguments)]
fn capture_one(
    conn: &Connection,
    queue: &mut EventQueue<State>,
    state: &mut State,
    qh: &QueueHandle<State>,
    shm: &wl_shm::WlShm,
    sources: &ExtOutputImageCaptureSourceManagerV1,
    copies: &ExtImageCopyCaptureManagerV1,
    output: &WlOutput,
) -> Option<(u32, u32, Vec<u8>)> {
    let deadline = Instant::now() + TIMEOUT;
    let source = sources.create_source(output, qh, ());
    state.session = SessionState::default();
    // No cursor: the options' paint_cursors left out.
    let session = copies.create_session(
        &source,
        ext_image_copy_capture_manager_v1::Options::empty(),
        qh,
        (),
    );
    let picture = (|| {
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
        bgra_to_rgba(&mut pixels);
        Some((width, height, pixels))
    })();
    session.destroy();
    source.destroy();
    let _ = conn.flush();
    picture
}

/// Little-endian BGRA (XRGB or ARGB) pixels as opaque RGBA, in place.
pub fn bgra_to_rgba(pixels: &mut [u8]) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
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

impl Dispatch<WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        output: &WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            if let Some(entry) = state.outputs.iter_mut().find(|(o, _)| o == output) {
                entry.1 = Some(name);
            }
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
    ExtOutputImageCaptureSourceManagerV1,
    ExtImageCaptureSourceV1,
    ExtImageCopyCaptureManagerV1
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_becomes_opaque_rgba() {
        // Blue 1, green 2, red 3, alpha 0 (XRGB's unused byte); twice.
        let mut pixels = vec![1, 2, 3, 0, 4, 5, 6, 7];
        bgra_to_rgba(&mut pixels);
        assert_eq!(pixels, [3, 2, 1, 255, 6, 5, 4, 255]);
    }
}

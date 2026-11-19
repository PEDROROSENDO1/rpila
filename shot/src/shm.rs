//! What capturing over a Wayland connection of one's own needs: waiting for
//! events with a deadline, and shm buffers to copy into and read back
//! (`capture.rs`).

use std::{
    fs::File,
    os::fd::{AsFd, AsRawFd, FromRawFd},
    time::Instant,
};

use wayland_client::{
    protocol::{wl_buffer::WlBuffer, wl_shm, wl_shm_pool::WlShmPool},
    Connection, Dispatch, EventQueue, QueueHandle,
};

/// Read and dispatch events until `done` holds; `None` past the deadline or
/// if the connection fails.
pub fn dispatch_until<S>(
    conn: &Connection,
    queue: &mut EventQueue<S>,
    state: &mut S,
    deadline: Instant,
    done: impl Fn(&S) -> bool,
) -> Option<()> {
    loop {
        queue.dispatch_pending(state).ok()?;
        if done(state) {
            return Some(());
        }
        conn.flush().ok()?;
        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        let left = deadline.checked_duration_since(Instant::now())?;
        let mut fd = libc::pollfd {
            fd: guard.connection_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let millis = left.as_millis().clamp(1, i32::MAX as u128) as i32;
        // SAFETY: one valid pollfd, for the call's length.
        let ready = unsafe { libc::poll(&mut fd, 1, millis) };
        if ready <= 0 {
            return None;
        }
        guard.read().ok()?;
    }
}

/// An shm buffer of `width` x `height` in `format`, and its file to read it
/// back from.
pub fn shm_buffer<S>(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<S>,
    width: u32,
    height: u32,
    format: wl_shm::Format,
) -> Option<(WlBuffer, File)>
where
    S: Dispatch<WlShmPool, ()> + Dispatch<WlBuffer, ()> + 'static,
{
    let len = width.checked_mul(height)?.checked_mul(4)?;
    // SAFETY: a new fd, owned by the File from here on.
    let fd = unsafe { libc::memfd_create(c"pila-capture".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return None;
    }
    let file = unsafe { File::from_raw_fd(fd) };
    file.set_len(len.into()).ok()?;
    let pool = shm.create_pool(file.as_fd(), i32::try_from(len).ok()?, qh, ());
    let buffer = pool.create_buffer(
        0,
        width as i32,
        height as i32,
        (width * 4) as i32,
        format,
        qh,
        (),
    );
    pool.destroy();
    Some((buffer, file))
}

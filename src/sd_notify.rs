//! Tells systemd that Pila is ready, when it runs as `pila.service`
//! (`resources/session/pila.service`, `Type=notify`): systemd reaches
//! `graphical-session.target`, and starts what hangs from it, once Pila has
//! exported its environment (`Pila::start_session`). The protocol is one
//! datagram, `READY=1`, to the socket `NOTIFY_SOCKET` names (a path, or an
//! abstract name written with a leading `@`), so there's no libsystemd.
//! Without `NOTIFY_SOCKET` (any other way of starting Pila) it does nothing.
//! What Pila starts doesn't inherit the variable (`Pila::session_env`).

use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::sync::Once;

use tracing::{info, warn};

/// The variable systemd sets for a `Type=notify` service.
pub const NOTIFY_SOCKET: &str = "NOTIFY_SOCKET";

static READY: Once = Once::new();

/// The address `NOTIFY_SOCKET`'s value names: a path, or with a leading `@`
/// an abstract name.
pub fn address(value: &str) -> std::io::Result<SocketAddr> {
    match value.strip_prefix('@') {
        Some(name) => {
            use std::os::linux::net::SocketAddrExt;
            SocketAddr::from_abstract_name(name)
        }
        None => SocketAddr::from_pathname(value),
    }
}

/// Send `READY=1`, the first time it's called. Never blocks (a full socket
/// loses the message, logged).
pub fn ready() {
    READY.call_once(|| {
        let Some(value) = std::env::var_os(NOTIFY_SOCKET) else {
            return;
        };
        let sent = value
            .to_str()
            .ok_or_else(|| std::io::Error::other("not UTF-8"))
            .and_then(address)
            .and_then(|addr| {
                let socket = UnixDatagram::unbound()?;
                socket.set_nonblocking(true)?;
                socket.send_to_addr(b"READY=1", &addr)
            });
        match sent {
            Ok(_) => info!("told systemd the session is ready"),
            Err(err) => warn!(?err, ?value, "can't tell systemd the session is ready"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::address;

    #[test]
    fn notify_socket_is_a_path_or_an_abstract_name() {
        let path = address("/run/user/1000/systemd/notify").unwrap();
        assert_eq!(
            path.as_pathname(),
            Some(std::path::Path::new("/run/user/1000/systemd/notify"))
        );
        use std::os::linux::net::SocketAddrExt;
        let abstract_name = address("@/org/freedesktop/systemd1/notify/123").unwrap();
        assert_eq!(
            abstract_name.as_abstract_name(),
            Some(&b"/org/freedesktop/systemd1/notify/123"[..])
        );
        assert!(abstract_name.as_pathname().is_none());
    }
}

//! The cookie X11 apps need to reach Pila's Xwayland. Xwayland runs with
//! `-auth` and a file holding one MIT-MAGIC-COOKIE-1, made anew for each
//! display Pila reserves; what Pila starts gets the file as `XAUTHORITY`.
//! A process that can reach the X socket without the cookie (another user,
//! a sandbox given no X11 permission) is refused, as the socket file's
//! permissions alone would leave to chance.
//!
//! The file is in `$XDG_RUNTIME_DIR`, the user's alone (mode 0600), with an
//! entry for any host (`FamilyWild`, as Flatpak writes them for its
//! sandboxes), so a change of hostname doesn't lock apps out.

use std::{
    io::{self, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use x11rb::rust_connection::{DefaultStream, RustConnection};

/// The only kind of cookie Pila makes.
pub const COOKIE_NAME: &[u8] = b"MIT-MAGIC-COOKIE-1";
/// `FamilyWild`: an entry for any address.
const FAMILY_WILD: u16 = 0xffff;

/// The Xauthority file of a display, removed when it's dropped.
#[derive(Debug)]
pub struct Xauthority {
    path: PathBuf,
    cookie: [u8; 16],
}

impl Xauthority {
    /// A new cookie for `display`, written to its file in `runtime_dir`.
    pub fn create(runtime_dir: &Path, display: u32) -> io::Result<Self> {
        let mut cookie = [0; 16];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut cookie)?;
        let path = runtime_dir.join(format!("pila-xauthority-{display}"));
        // A file left by a Pila that crashed may have other permissions,
        // which truncating it would keep.
        match std::fs::remove_file(&path) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err),
            _ => {}
        }
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?
            .write_all(&entry(display, &cookie))?;
        Ok(Self { path, cookie })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn cookie(&self) -> [u8; 16] {
        self.cookie
    }
}

impl Drop for Xauthority {
    fn drop(&mut self) {
        if let Err(err) = std::fs::remove_file(&self.path) {
            tracing::warn!(?err, path = %self.path.display(), "can't remove the Xauthority file");
        }
    }
}

/// An Xauthority entry for `display` on any host: family, address, display
/// number, cookie name and cookie, each but the family preceded by its
/// length, all big-endian.
fn entry(display: u32, cookie: &[u8]) -> Vec<u8> {
    let mut bytes = FAMILY_WILD.to_be_bytes().to_vec();
    let number = display.to_string();
    for field in [&b""[..], number.as_bytes(), COOKIE_NAME, cookie] {
        let len = u16::try_from(field.len()).expect("a field of a few bytes");
        bytes.extend(len.to_be_bytes());
        bytes.extend(field);
    }
    bytes
}

/// The first MIT-MAGIC-COOKIE-1 in the Xauthority file at `path`.
pub fn read_cookie(path: &Path) -> io::Result<Vec<u8>> {
    let bytes = std::fs::read(path)?;
    let mut rest = &bytes[..];
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "not an Xauthority file");
    let field = |rest: &mut &[u8]| -> io::Result<Vec<u8>> {
        let (len, tail) = rest.split_first_chunk::<2>().ok_or_else(invalid)?;
        let len = usize::from(u16::from_be_bytes(*len));
        let value = tail.get(..len).ok_or_else(invalid)?.to_vec();
        *rest = &tail[len..];
        Ok(value)
    };
    while !rest.is_empty() {
        rest = rest.get(2..).ok_or_else(invalid)?; // the family
        let _address = field(&mut rest)?;
        let _number = field(&mut rest)?;
        let name = field(&mut rest)?;
        let data = field(&mut rest)?;
        if name == COOKIE_NAME {
            return Ok(data);
        }
    }
    Err(invalid())
}

/// Connect to the X display `display` through its socket file with
/// `cookie` (empty: none). Pila's own connection that asks X for its
/// clients uses it, since Pila's environment has no `XAUTHORITY`.
pub fn connect(display: u32, cookie: &[u8]) -> Result<(RustConnection, usize), String> {
    let socket = format!("/tmp/.X11-unix/X{display}");
    let stream = std::os::unix::net::UnixStream::connect(&socket)
        .map_err(|err| format!("{socket}: {err}"))?;
    let (stream, _) = DefaultStream::from_unix_stream(stream).map_err(|err| err.to_string())?;
    let name = if cookie.is_empty() {
        Vec::new()
    } else {
        COOKIE_NAME.to_vec()
    };
    let conn = RustConnection::connect_to_stream_with_auth_info(stream, 0, name, cookie.to_vec())
        .map_err(|err| err.to_string())?;
    Ok((conn, 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pila-xauth-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_cookie_is_written_for_the_user_alone_and_read_back() {
        let dir = dir();
        let auth = Xauthority::create(&dir, 7).unwrap();
        let mode = std::fs::metadata(auth.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(read_cookie(auth.path()).unwrap(), auth.cookie());
        let path = auth.path().to_path_buf();
        drop(auth);
        assert!(!path.exists(), "removed with it");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_file_left_behind_is_replaced_with_the_users_alone() {
        let dir = dir();
        let path = dir.join("pila-xauthority-3");
        std::fs::write(&path, b"old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let auth = Xauthority::create(&dir, 3).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(read_cookie(&path).unwrap(), auth.cookie());
        drop(auth);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn each_display_gets_a_new_cookie() {
        let dir = dir();
        let a = Xauthority::create(&dir, 1).unwrap();
        let b = Xauthority::create(&dir, 2).unwrap();
        assert_ne!(a.cookie(), b.cookie());
        drop((a, b));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_entry_is_as_libxau_writes_one() {
        let bytes = entry(12, &[0xab; 16]);
        let mut expected = vec![0xff, 0xff, 0, 0, 0, 2, b'1', b'2', 0, 18];
        expected.extend(COOKIE_NAME);
        expected.extend([0, 16]);
        expected.extend([0xab; 16]);
        assert_eq!(bytes, expected);
    }
}

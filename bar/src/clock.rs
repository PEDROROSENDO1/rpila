//! The clock: the local time as strftime(3) writes it, in the language of
//! `LC_TIME` (`main` sets the locale), kept on a thread of its own that
//! sends the text only when it changes.

use std::{
    ffi::CString,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

extern "C" {
    // POSIX; the libc crate has it only for Windows.
    fn tzset();
}

/// The clock's text now: `format` in local time. Empty for an empty or
/// unusable format.
pub fn now(format: &str) -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let time = secs as libc::time_t;
    // SAFETY: tzset only re-reads TZ and /etc/localtime (a change of time
    // zone shows at the next tick), and `tm` is a plain struct written by
    // localtime_r, which returns null if it couldn't.
    unsafe {
        tzset();
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&time, &mut tm).is_null() {
            return String::new();
        }
        format_tm(format, &tm)
    }
}

/// `tm` as `format` says, through strftime.
fn format_tm(format: &str, tm: &libc::tm) -> String {
    let Ok(c_format) = CString::new(format) else {
        return String::new();
    };
    if format.is_empty() {
        return String::new();
    }
    // strftime returns 0 both when the text doesn't fit and when it's
    // empty (a lone `%p` in some locales): a bigger buffer, up to a limit.
    let mut size = 64;
    while size <= 4096 {
        let mut buffer = vec![0u8; size];
        // SAFETY: the buffer is `size` bytes, the format a C string and
        // `tm` a valid struct.
        let written =
            unsafe { libc::strftime(buffer.as_mut_ptr().cast(), size, c_format.as_ptr(), tm) };
        if written > 0 {
            buffer.truncate(written);
            return String::from_utf8_lossy(&buffer).into_owned();
        }
        size *= 4;
    }
    String::new()
}

/// How long until just past the next second, from `subsec_nanos` into
/// this one: each tick reads the clock once its second has begun.
pub fn until_next_second(subsec_nanos: u32) -> Duration {
    let left = 1_000_000_000u32.saturating_sub(subsec_nanos.min(999_999_999));
    Duration::from_nanos(u64::from(left)) + Duration::from_millis(5)
}

/// The clock's text each time it changes, from a thread of its own. It
/// looks every second, so the time is right again at once after a suspend
/// or a change of time zone; only a change wakes the bar.
pub fn ticks(format: String) -> impl iced::futures::Stream<Item = String> {
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let spawned = std::thread::Builder::new()
        .name("pila-bar-clock".into())
        .spawn(move || {
            let mut shown = None;
            loop {
                let text = now(&format);
                if shown.as_ref() != Some(&text) {
                    if sender.unbounded_send(text.clone()).is_err() {
                        break;
                    }
                    shown = Some(text);
                }
                let into = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.subsec_nanos())
                    .unwrap_or_default();
                std::thread::sleep(until_next_second(into));
            }
        });
    if let Err(err) = spawned {
        eprintln!("pila-bar: no clock: {err}");
    }
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(secs: i64) -> libc::tm {
        let time = secs as libc::time_t;
        // SAFETY: as in `now`.
        unsafe {
            let mut tm: libc::tm = std::mem::zeroed();
            assert!(!libc::gmtime_r(&time, &mut tm).is_null());
            tm
        }
    }

    #[test]
    fn formats_through_strftime() {
        // 2026-11-17 20:35:14 UTC, a Tuesday.
        let tm = utc(1_794_947_714);
        assert_eq!(format_tm("%Y-%m-%d %H:%M:%S", &tm), "2026-11-17 20:35:14");
        assert_eq!(format_tm("100%%", &tm), "100%");
        assert_eq!(format_tm("", &tm), "");
        assert_eq!(format_tm("a\0b", &tm), "");
        // Longer than the first buffer.
        let long = "%Y ".repeat(40);
        assert_eq!(format_tm(&long, &tm), "2026 ".repeat(40));
        // Longer than any buffer: nothing rather than a part.
        assert_eq!(format_tm(&"%Y".repeat(3000), &tm), "");
    }

    #[test]
    fn ticks_land_just_past_each_second() {
        assert_eq!(until_next_second(0), Duration::from_millis(1005));
        assert_eq!(until_next_second(250_000_000), Duration::from_millis(755));
        assert_eq!(
            until_next_second(999_999_999),
            Duration::from_nanos(5_000_001)
        );
        assert_eq!(
            until_next_second(2_000_000_000),
            Duration::from_nanos(5_000_001)
        );
    }

    #[test]
    fn now_reads_the_local_time() {
        let year = now("%Y");
        assert_eq!(year.len(), 4, "{year:?}");
        assert!(year.starts_with("20"), "{year:?}");
    }
}

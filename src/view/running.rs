//! A viewer that's already running, so starting another on its port can
//! take its place (it may be an older version) instead of failing. Each
//! run's key is random (see the module docs above), so the running viewer
//! records its port, key and process in the data folder (`view-<port>.json`,
//! readable only by its owner), and a second `view` reads it. It only
//! trusts the record once the viewer on that port has accepted the key and
//! says it reads the same events: a stale record (from a viewer that's
//! gone) or another program on the port is refused, and so never stopped.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where the viewer reading `events_dir` on `port` records its key: beside
/// the events folder, in the data folder.
fn record_path(events_dir: &Path, port: u16) -> PathBuf {
    events_dir
        .parent()
        .unwrap_or(events_dir)
        .join(format!("view-{port}.json"))
}

/// Records the viewer on `port`, reading `events_dir`, and its `key`.
pub fn record(events_dir: &Path, port: u16, key: &str) -> std::io::Result<()> {
    let path = record_path(events_dir, port);
    let body = serde_json::json!({
        "port": port,
        "key": key,
        "pid": std::process::id(),
    });
    // Written whole, then moved into place, so a reader never sees half of
    // it; and created readable only by this account, since the key is in it.
    let tmp = path.with_extension(format!("json.{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options
        .open(&tmp)
        .and_then(|mut f| f.write_all(body.to_string().as_bytes()))
        .and_then(|()| std::fs::rename(&tmp, &path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// The viewer running on `port` and reading `events_dir`, as recorded.
#[derive(Debug, PartialEq)]
pub struct Running {
    pub key: String,
    /// Its process: the one that wrote the record, with the key it took.
    pub pid: Option<u32>,
}

/// The viewer running on `port` and reading `events_dir`, if there is one
/// and it takes the key recorded for it.
pub fn find(events_dir: &Path, port: u16) -> Option<Running> {
    let text = std::fs::read_to_string(record_path(events_dir, port)).ok()?;
    let record: serde_json::Value = serde_json::from_str(&text).ok()?;
    let key = record.get("key")?.as_str()?;
    // It goes into a request, so it must be what a key looks like.
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    if reads(port, key)? != events_dir.display().to_string() {
        return None;
    }
    // Keys are made fresh by each run, so the process on the port that took
    // this one is the one that recorded it, with this pid.
    let pid = record
        .get("pid")
        .and_then(|p| p.as_u64())
        .and_then(|p| u32::try_from(p).ok())
        .filter(|&p| p != 0 && p != std::process::id());
    Some(Running {
        key: key.to_string(),
        pid,
    })
}

/// Stops process `pid` (a viewer `find` found): asks it to on Unix, or,
/// with `force`, makes it. Whether it's gone is for the caller to see, by
/// its port coming free (a stopped process can linger until its parent
/// collects it).
pub fn stop(pid: u32, force: bool) -> Result<(), String> {
    imp::stop(pid, force)
}

#[cfg(unix)]
mod imp {
    pub fn stop(pid: u32, force: bool) -> Result<(), String> {
        let pid = libc::pid_t::try_from(pid).map_err(|_| format!("no process {pid}"))?;
        let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
        // SAFETY: kill only sends a signal; pid is a positive process id.
        if unsafe { libc::kill(pid, signal) } == 0 {
            return Ok(());
        }
        let err = std::io::Error::last_os_error();
        // Gone already.
        if err.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        Err(format!("couldn't stop process {pid}: {err}"))
    }
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};

    /// Windows has no asking: it's made to either way.
    pub fn stop(pid: u32, _force: bool) -> Result<(), String> {
        // SAFETY: the handle is checked, then closed.
        unsafe {
            let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if handle.is_null() {
                // Gone already (or not ours to stop: then its port stays taken).
                return Ok(());
            }
            let stopped = TerminateProcess(handle, 1) != 0;
            CloseHandle(handle);
            if stopped {
                Ok(())
            } else {
                Err(format!("couldn't stop process {pid}"))
            }
        }
    }
}

/// Asks the viewer on `port` which events it reads, with `key`. `None` if
/// nothing answers, or it doesn't take the key.
fn reads(port: u16, key: &str) -> Option<String> {
    let wait = Duration::from_secs(2);
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&address, wait).ok()?;
    stream.set_read_timeout(Some(wait)).ok()?;
    stream.set_write_timeout(Some(wait)).ok()?;
    write!(
        stream,
        "GET /api/info HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Agent-Graph-Key: {key}\r\n\
         Connection: close\r\n\r\n"
    )
    .ok()?;
    let mut reply = Vec::new();
    stream.take(64 * 1024).read_to_end(&mut reply).ok()?;
    let reply = String::from_utf8_lossy(&reply);
    let (head, body) = reply.split_once("\r\n\r\n")?;
    if !head.starts_with("HTTP/1.1 200") {
        return None;
    }
    let info: serde_json::Value = serde_json::from_str(body).ok()?;
    Some(info.get("events_dir")?.as_str()?.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_running_viewer_is_found_by_its_record() {
        let root = tempfile::tempdir().unwrap();
        let events = root.path().join("events");
        std::fs::create_dir(&events).unwrap();
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let key = super::super::start_with(&events, listener, Duration::from_secs(600), |_| Ok(()))
            .unwrap();

        assert_eq!(find(&events, port), None, "no record yet");
        record(&events, port, &key).unwrap();
        // Its pid is this test's own, which is never one to stop.
        assert_eq!(
            find(&events, port),
            Some(Running {
                key: key.clone(),
                pid: None
            })
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(record_path(&events, port))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "only its owner can read the key");
        }

        // Another data folder's viewer isn't this one's.
        let other = tempfile::tempdir().unwrap();
        let other_events = other.path().join("events");
        record(&other_events, port, &key).unwrap();
        assert_eq!(find(&other_events, port), None);

        // A key it doesn't take (a stale record), or one that isn't a key.
        record(&events, port, "0123456789abcdef").unwrap();
        assert_eq!(find(&events, port), None);
        record(&events, port, "k\r\nHost: evil").unwrap();
        assert_eq!(find(&events, port), None);
    }

    #[test]
    fn nothing_listening_is_no_viewer() {
        let root = tempfile::tempdir().unwrap();
        let events = root.path().join("events");
        let port = std::net::TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port(); // Closed again at once.
        record(&events, port, "abc123").unwrap();
        assert_eq!(find(&events, port), None);
    }
}

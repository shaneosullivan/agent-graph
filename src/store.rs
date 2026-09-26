//! Reading and writing the event files.
//!
//! Each session appends to its own `.jsonl` file. Every hook invocation writes
//! its lines with a single append, so concurrent hooks don't interleave lines
//! and no locking is needed.

use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

use crate::event::Envelope;

/// Creates `path` and its parents. On Unix they're readable only by the current
/// user; on Windows the user profile's ACLs already do that.
pub fn ensure_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path)
    }
}

/// Appends `bytes` to `path` in one write, creating the file if needed.
pub fn append(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut opts = OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)?.write_all(bytes)
}

/// Writes `bytes` to `path` via a temporary file and a rename, so readers never
/// see a half-written file. The temporary file is always a new one (never a
/// file or link someone left at a predictable name), and the rename replaces
/// `path` itself, even if it's a link. A regular file that was there keeps
/// its permissions; anything else (nothing, or a link, whose target's
/// permissions have nothing to do with this file) makes a new file readable
/// only by the user.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let keep = fs::symlink_metadata(path)
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.permissions());
    write_atomic_with(path, bytes, keep)
}

/// `write_atomic`, giving the file `permissions`, or if `None`, making it
/// readable only by the user. Only Unix modes are copied: on Windows,
/// "permissions" are attributes like read-only, which would stop the file
/// being replaced next time.
pub fn write_atomic_with(
    path: &Path,
    bytes: &[u8],
    permissions: Option<fs::Permissions>,
) -> io::Result<()> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?
        .to_string_lossy();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let mut attempt = 0;
    let (tmp, mut file) = loop {
        let tmp = dir.join(format!(
            ".{name}.{}-{nanos}-{attempt}.tmp",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&tmp) {
            Ok(file) => break (tmp, file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && attempt < 100 => attempt += 1,
            Err(e) => return Err(e),
        }
    };
    let written = file
        .write_all(bytes)
        .and_then(|()| keep_permissions(&file, permissions))
        .and_then(|()| file.sync_all())
        .and_then(|()| {
            drop(file);
            fs::rename(&tmp, path)
        });
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

/// Writes `bytes` to a new file at `path`, failing with `AlreadyExists` if
/// there's anything there already (a file, or a link, which isn't followed).
pub fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let written = file.write_all(bytes);
    if written.is_err() {
        drop(file);
        let _ = fs::remove_file(path);
    }
    written
}

/// Gives `file` the Unix `permissions`, if any. (Not on Windows, where they're
/// attributes like read-only; see `write_atomic_with`.)
#[cfg(unix)]
fn keep_permissions(file: &fs::File, permissions: Option<fs::Permissions>) -> io::Result<()> {
    permissions.map_or(Ok(()), |p| file.set_permissions(p))
}

#[cfg(not(unix))]
fn keep_permissions(_file: &fs::File, _permissions: Option<fs::Permissions>) -> io::Result<()> {
    Ok(())
}

/// Errors if `path`, or a folder between `root` and it, is a symbolic link.
/// For files a project ships, so one can't lead a write out of the project.
pub fn refuse_links(root: &Path, path: &Path) -> io::Result<()> {
    let relative = path.strip_prefix(root).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} isn't in {}", path.display(), root.display()),
        )
    })?;
    let mut at = root.to_path_buf();
    for part in relative.components() {
        at.push(part);
        match fs::symlink_metadata(&at) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(io::Error::other(format!(
                    "{} is a symbolic link",
                    at.display()
                )));
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => break,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[derive(Debug, Default)]
pub struct Loaded {
    pub events: Vec<Envelope>,
    /// Lines that weren't valid events, e.g. a line cut short by a crash.
    pub skipped_lines: usize,
}

/// Reads every `*.jsonl` file in `dir`. A missing directory is just empty.
pub fn load_events(dir: &Path) -> io::Result<Loaded> {
    let mut loaded = Loaded::default();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(loaded),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_none_or(|ext| ext != "jsonl") {
            continue;
        }
        // Lines as bytes: one that isn't UTF-8 (a write torn mid-character)
        // is just a bad line, like any other that doesn't parse.
        for line in BufReader::new(fs::File::open(&path)?).split(b'\n') {
            let line = line?;
            if line.trim_ascii().is_empty() {
                continue;
            }
            match serde_json::from_slice::<Envelope>(&line) {
                Ok(event) => loaded.events.push(event),
                Err(_) => loaded.skipped_lines += 1,
            }
        }
    }
    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_writes_replace_the_file_and_leave_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        write_atomic(&file, b"one").unwrap();
        write_atomic(&file, b"two").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "two");
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["settings.json"], "no temporary files left");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_writes_replace_a_link_rather_than_follow_it() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        fs::write(&outside, "keep me").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        write_atomic(&link, b"new").unwrap();
        assert_eq!(fs::read_to_string(&outside).unwrap(), "keep me");
        assert!(
            !fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_replaced_link_doesnt_take_its_targets_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join("shared");
        fs::create_dir(&shared).unwrap();
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o777)).unwrap();
        let script = dir.path().join("script");
        fs::write(&script, "").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        for target in [&shared, &script] {
            let link = dir.path().join("link");
            let _ = fs::remove_file(&link);
            std::os::unix::fs::symlink(target, &link).unwrap();
            write_atomic(&link, b"{}").unwrap();
            let mode = fs::metadata(&link).unwrap().permissions().mode() & 0o7777;
            assert_eq!(mode, 0o600, "took {}'s permissions", target.display());
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_read_only_original_doesnt_stop_the_next_write() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("settings.json");
        fs::write(&original, "{}").unwrap();
        let mut read_only = fs::metadata(&original).unwrap().permissions();
        read_only.set_readonly(true);
        let backup = dir.path().join("settings.json.bak");
        write_atomic_with(&backup, b"{}", Some(read_only)).unwrap();
        write_atomic(&backup, b"{\"again\": true}").unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn links_inside_a_project_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path();
        fs::create_dir_all(project.join("real")).unwrap();
        std::os::unix::fs::symlink(root.path(), project.join("linked")).unwrap();
        assert!(refuse_links(project, &project.join("real/settings.json")).is_ok());
        assert!(refuse_links(project, &project.join("missing/a/b")).is_ok());
        assert!(refuse_links(project, &project.join("linked/settings.json")).is_err());
        assert!(refuse_links(project, &project.join("linked")).is_err());
        assert!(refuse_links(project, Path::new("/elsewhere")).is_err());
    }

    #[test]
    fn load_skips_bad_lines_and_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let good = r#"{"v":1,"id":"01J8ZK3V7Q9R2M5X4T6W8Y0B1C","ts":"2026-09-25T10:00:00.000Z","type":"status","node":"x:1","data":{"state":"idle"}}"#;
        append(
            &dir.path().join("x-1.jsonl"),
            format!("{good}\n{{\"v\":1,\"id\n").as_bytes(),
        )
        .unwrap();
        fs::write(dir.path().join("notes.txt"), "ignored").unwrap();

        let loaded = load_events(dir.path()).unwrap();
        assert_eq!(loaded.events.len(), 1);
        assert_eq!(loaded.skipped_lines, 1);
    }

    /// R12: a write torn inside a multi-byte character (a crash, a full
    /// disk) leaves bytes that aren't UTF-8. That line is skipped; the rest
    /// of the file, and every other file, still load.
    #[test]
    fn a_line_that_isnt_utf8_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let good = |n: u32| {
            format!(
                r#"{{"v":1,"id":"01J8ZK3V7Q9R2M5X4T6W8Y0B{n:02}","ts":"2026-09-25T10:00:00.000Z","type":"status","node":"x:1","data":{{"state":"idle"}}}}"#
            )
        };
        let mut torn = format!("{}\n", good(1)).into_bytes();
        torn.extend_from_slice(b"{\"v\":1,\"data\":{\"summary\":\"caf\xc3");
        torn.extend_from_slice(format!("{}\n", good(2)).as_bytes());
        torn.extend_from_slice(format!("{}\n", good(3)).as_bytes());
        append(&dir.path().join("x-1.jsonl"), &torn).unwrap();
        append(&dir.path().join("y-1.jsonl"), format!("{}\n", good(4)).as_bytes()).unwrap();

        let loaded = load_events(dir.path()).expect("loads despite the bad bytes");
        assert_eq!(loaded.events.len(), 3, "lines 1 and 3, and the other file");
        assert_eq!(loaded.skipped_lines, 1);
    }

    #[test]
    fn missing_dir_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_events(&dir.path().join("nope")).unwrap();
        assert!(loaded.events.is_empty());
    }
}

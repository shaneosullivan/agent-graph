//! The process tree: each process's parent, name and start time.
//!
//! A session records its agent's process and the ones above it, so the
//! reducer can link a session to the one whose agent started it (design §6,
//! method 3), and a local view can tell when a session's process has gone.
//! Processes are named `<pid>@<start time>`: the start time keeps the name
//! unique after the OS reuses the pid.
//!
//! - Linux: `/proc/<pid>/stat`.
//! - macOS: `proc_pidinfo`.
//! - Windows: a Toolhelp snapshot, and `GetProcessTimes` for the start.
//! - Anywhere else, nothing is known, and sessions link by environment only.

/// How far up the tree to look.
const MAX_DEPTH: usize = 16;

/// Shells, and the like, that run a hook command or an agent's shell
/// commands, rather than being the agent.
const SHELLS: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "dash",
    "fish",
    "ksh",
    "tcsh",
    "csh",
    "nu",
    "cmd",
    "powershell",
    "pwsh",
    "wsl",
    "env",
    "timeout",
];

/// One process: enough to follow the tree up, and to tell it apart from a
/// later process that reuses its pid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    pub parent: u32,
    pub name: String,
    /// When it started, in the OS's own units. Only compared.
    pub start: u64,
}

impl Process {
    /// `<pid>@<start>`, as events record it.
    pub fn id(&self) -> String {
        format!("{}@{}", self.pid, self.start)
    }

    /// Whether this is a shell (or wrapper) rather than a program in its own right.
    pub fn is_shell(&self) -> bool {
        let name = self.name.to_ascii_lowercase();
        let name = name.strip_suffix(".exe").unwrap_or(&name);
        let name = name.strip_prefix('-').unwrap_or(name); // login shells: "-zsh"
        SHELLS.contains(&name)
    }
}

/// `pid` and the processes above it, nearest first. It stops at the top of
/// the tree, at a process it can't read, or at a "parent" that started after
/// its child (the real parent is gone and its pid was reused).
pub fn lineage(pid: u32) -> Vec<Process> {
    let reader = imp::Reader::new();
    let mut out: Vec<Process> = Vec::new();
    let mut next = pid;
    while out.len() < MAX_DEPTH && next > 1 && !out.iter().any(|p| p.pid == next) {
        let Some(process) = reader.get(next) else {
            break;
        };
        if out.last().is_some_and(|child| process.start > child.start) {
            break;
        }
        next = process.parent;
        out.push(process);
    }
    out
}

/// Whether the process named `id` (`<pid>@<start>`) is still running, or
/// `None` where that can't be told.
pub fn alive(id: &str) -> Option<bool> {
    if !imp::SUPPORTED {
        return None;
    }
    let (pid, start) = id.split_once('@')?;
    let (pid, start) = (pid.parse::<u32>().ok()?, start.parse::<u64>().ok()?);
    Some(
        imp::Reader::new()
            .get(pid)
            .is_some_and(|p| p.start == start),
    )
}

/// The command line `pid` was started with, its arguments joined by spaces,
/// where that can be read (Linux and macOS).
pub fn command_line(pid: u32) -> Option<String> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        let line = String::from_utf8_lossy(&raw).replace('\0', " ");
        Some(line.trim().to_string()).filter(|l| !l.is_empty())
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("ps")
            .args(["-ww", "-o", "command=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (out.status.success() && !line.is_empty()).then_some(line)
    }
    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
    {
        let _ = pid;
        None
    }
}

/// The agent that ran this hook (the nearest process above this one that
/// isn't a shell), and the processes above it.
pub fn agent_of_this_hook() -> Option<(Process, Vec<Process>)> {
    let mut chain = lineage(std::process::id());
    if chain.first().is_some_and(|p| p.pid == std::process::id()) {
        chain.remove(0);
    }
    let shells = chain.iter().take_while(|p| p.is_shell()).count();
    let mut rest = chain.split_off(shells);
    if rest.is_empty() {
        return None;
    }
    let agent = rest.remove(0);
    Some((agent, rest))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod imp {
    use super::Process;

    pub const SUPPORTED: bool = true;

    pub struct Reader;

    impl Reader {
        pub fn new() -> Reader {
            Reader
        }

        pub fn get(&self, pid: u32) -> Option<Process> {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            // "pid (name) state ppid …": the name can hold spaces and parens.
            let (open, close) = (stat.find('(')?, stat.rfind(')')?);
            let fields: Vec<&str> = stat.get(close + 1..)?.split_whitespace().collect();
            // fields[0] is field 3 (state), so field n is fields[n - 3]:
            // 4 is the parent, 22 the start time in clock ticks since boot.
            Some(Process {
                pid,
                parent: fields.get(1)?.parse().ok()?,
                name: stat.get(open + 1..close)?.to_string(),
                start: fields.get(19)?.parse().ok()?,
            })
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Process;

    pub const SUPPORTED: bool = true;

    pub struct Reader;

    impl Reader {
        pub fn new() -> Reader {
            Reader
        }

        pub fn get(&self, pid: u32) -> Option<Process> {
            let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
            // SAFETY: proc_bsdinfo is plain data, and proc_pidinfo writes at
            // most `size` bytes into it.
            let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
            let written = unsafe {
                libc::proc_pidinfo(
                    libc::c_int::try_from(pid).ok()?,
                    libc::PROC_PIDTBSDINFO,
                    0,
                    (&raw mut info).cast(),
                    size,
                )
            };
            if written != size {
                return None;
            }
            let name = Some(text(&info.pbi_name))
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| text(&info.pbi_comm));
            Some(Process {
                pid,
                parent: info.pbi_ppid,
                name,
                start: info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec,
            })
        }
    }

    fn text(chars: &[libc::c_char]) -> String {
        let bytes: Vec<u8> = chars
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| c as u8)
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

#[cfg(windows)]
mod imp {
    use std::collections::HashMap;

    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    use super::Process;

    pub const SUPPORTED: bool = true;

    /// Every process's parent and name, from one snapshot.
    pub struct Reader(HashMap<u32, (u32, String)>);

    impl Reader {
        pub fn new() -> Reader {
            let mut table = HashMap::new();
            // SAFETY: the snapshot handle is checked, then closed; the entry
            // is sized as the API requires.
            unsafe {
                let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
                if snapshot != INVALID_HANDLE_VALUE {
                    let mut entry: PROCESSENTRY32W = std::mem::zeroed();
                    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
                    let mut more = Process32FirstW(snapshot, &mut entry) != 0;
                    while more {
                        let len = entry
                            .szExeFile
                            .iter()
                            .position(|&c| c == 0)
                            .unwrap_or(entry.szExeFile.len());
                        let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                        table.insert(entry.th32ProcessID, (entry.th32ParentProcessID, name));
                        more = Process32NextW(snapshot, &mut entry) != 0;
                    }
                    CloseHandle(snapshot);
                }
            }
            Reader(table)
        }

        pub fn get(&self, pid: u32) -> Option<Process> {
            let (parent, name) = self.0.get(&pid)?.clone();
            Some(Process {
                pid,
                parent,
                name,
                start: start_time(pid)?,
            })
        }
    }

    /// When `pid` started, in 100 ns ticks since 1601.
    fn start_time(pid: u32) -> Option<u64> {
        // SAFETY: the handle is checked, then closed; the times are plain data.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return None;
            }
            let mut times: [FILETIME; 4] = std::mem::zeroed();
            let [created, exited, kernel, user] = &mut times;
            let ok = GetProcessTimes(handle, created, exited, kernel, user) != 0;
            CloseHandle(handle);
            ok.then(|| {
                (u64::from(times[0].dwHighDateTime) << 32) | u64::from(times[0].dwLowDateTime)
            })
        }
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    windows
)))]
mod imp {
    use super::Process;

    pub const SUPPORTED: bool = false;

    pub struct Reader;

    impl Reader {
        pub fn new() -> Reader {
            Reader
        }

        pub fn get(&self, _pid: u32) -> Option<Process> {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_and_its_parents_can_be_read() {
        let chain = lineage(std::process::id());
        let me = chain.first().expect("this process");
        assert_eq!(me.pid, std::process::id());
        assert!(!me.name.is_empty());
        assert!(
            chain.len() >= 2,
            "at least the test runner's parent: {chain:?}"
        );
        assert_eq!(chain[1].pid, me.parent);
        assert!(chain.windows(2).all(|w| w[1].start <= w[0].start));
        assert_eq!(alive(&me.id()), Some(true));
    }

    #[test]
    fn a_reused_pid_is_not_the_same_process() {
        let me = &lineage(std::process::id())[0];
        let earlier = format!("{}@{}", me.pid, me.start.saturating_sub(1));
        assert_eq!(alive(&earlier), Some(false));
        assert_eq!(alive("not a process"), None);
    }

    #[test]
    fn shells_are_recognised() {
        let named = |name: &str| Process {
            pid: 2,
            parent: 1,
            name: name.into(),
            start: 0,
        };
        for shell in ["sh", "-zsh", "bash", "cmd.exe", "PowerShell.exe"] {
            assert!(named(shell).is_shell(), "{shell}");
        }
        for program in ["claude", "node", "codex.exe", "agent-graph"] {
            assert!(!named(program).is_shell(), "{program}");
        }
    }
}

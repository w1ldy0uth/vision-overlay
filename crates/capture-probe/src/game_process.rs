//! Observe the game executable; never open game memory or terminate the game.
use std::time::Duration;

pub const GAME_EXE: &str = "GenshinImpact.exe";
const EXIT_GRACE: Duration = Duration::from_secs(3);

#[derive(Debug, PartialEq)]
pub enum Transition {
    Started,
    Exited,
}

#[derive(Default)]
pub struct Watcher {
    seen: bool,
    absent_since: Option<Duration>,
}

impl Watcher {
    /// Unknown observations break the absence streak; they never mean exit.
    pub fn observe(&mut self, present: Option<bool>, now: Duration) -> Option<Transition> {
        match present {
            Some(true) => {
                self.absent_since = None;
                if !self.seen {
                    self.seen = true;
                    return Some(Transition::Started);
                }
            }
            Some(false) if self.seen => {
                let since = *self.absent_since.get_or_insert(now);
                if now.saturating_sub(since) >= EXIT_GRACE {
                    return Some(Transition::Exited);
                }
            }
            _ => self.absent_since = None,
        }
        None
    }
}

#[cfg(windows)]
pub fn is_running() -> std::io::Result<bool> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_NO_MORE_FILES, HANDLE, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
    };
    struct Snapshot(HANDLE);
    impl Drop for Snapshot {
        fn drop(&mut self) {
            // SAFETY: the successful snapshot handle is owned by this guard.
            unsafe { CloseHandle(self.0) };
        }
    }
    // SAFETY: process enumeration takes no pointers or access to game memory.
    let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    let snapshot = Snapshot(handle);
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    // SAFETY: entry has the required size and remains valid during each call.
    let mut ok = unsafe { Process32FirstW(snapshot.0, &mut entry) };
    loop {
        if ok == 0 {
            let error = std::io::Error::last_os_error();
            return if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                Ok(false)
            } else {
                Err(error)
            };
        }
        let length = entry
            .szExeFile
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(entry.szExeFile.len());
        if String::from_utf16_lossy(&entry.szExeFile[..length]).eq_ignore_ascii_case(GAME_EXE) {
            return Ok(true);
        }
        // SAFETY: the guard owns an open snapshot and entry is correctly sized.
        ok = unsafe { Process32NextW(snapshot.0, &mut entry) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_for_launch_then_confirms_exit() {
        let mut watcher = Watcher::default();
        let time = Duration::from_secs;
        assert_eq!(watcher.observe(Some(false), time(0)), None);
        assert_eq!(watcher.observe(Some(false), time(3600)), None);
        assert_eq!(
            watcher.observe(Some(true), time(3601)),
            Some(Transition::Started)
        );
        assert_eq!(watcher.observe(Some(true), time(3602)), None);
        assert_eq!(watcher.observe(Some(false), time(3603)), None);
        assert_eq!(watcher.observe(Some(false), time(3605)), None);
        assert_eq!(
            watcher.observe(Some(false), time(3606)),
            Some(Transition::Exited)
        );
    }

    #[test]
    fn reappearance_and_enumeration_errors_cancel_exit_countdown() {
        let mut watcher = Watcher::default();
        let time = Duration::from_secs;
        watcher.observe(Some(true), time(0));
        watcher.observe(Some(false), time(1));
        assert_eq!(watcher.observe(Some(true), time(3)), None);
        watcher.observe(Some(false), time(4));
        assert_eq!(watcher.observe(None, time(7)), None);
        assert_eq!(watcher.observe(Some(false), time(8)), None);
        assert_eq!(watcher.observe(Some(false), time(10)), None);
        assert_eq!(
            watcher.observe(Some(false), time(11)),
            Some(Transition::Exited)
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_process_enumeration_succeeds() {
        // Works whether or not the user currently has the game open.
        is_running().expect("Windows process list should be readable");
    }
}

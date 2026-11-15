//! A named kernel object's lifetime prevents two conflicting live probe instances.
use anyhow::{Result, bail};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
    System::Threading::CreateMutexW,
};
pub struct CaptureLock(HANDLE);
// The object is a lifetime marker, not an owned thread mutex; no wait/release is used.
unsafe impl Send for CaptureLock {}
impl Drop for CaptureLock {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}
impl CaptureLock {
    pub fn acquire() -> Result<Self> {
        Self::named("Local\\VisionOverlayLiveCapture")
    }
    fn named(name: &str) -> Result<Self> {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        // SAFETY: null security attributes and a valid null-terminated name. We do not
        // acquire thread ownership; holding the handle keeps the name alive.
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide.as_ptr()) };
        let error = unsafe { GetLastError() };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        if error == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(handle) };
            bail!("Another Vision Overlay live capture is already running");
        }
        Ok(Self(handle))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn second_live_instance_is_refused_and_handle_release_allows_restart() {
        let name = format!("Local\\VisionOverlayLockTest{}", std::process::id());
        let first = CaptureLock::named(&name).unwrap();
        assert!(CaptureLock::named(&name).is_err());
        drop(first);
        assert!(CaptureLock::named(&name).is_ok());
    }
}

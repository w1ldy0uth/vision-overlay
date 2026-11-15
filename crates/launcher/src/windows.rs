use super::Layout;
use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    ptr::{null, null_mut},
    sync::atomic::{AtomicU32, Ordering},
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::*},
    UI::{Controls::Dialogs::*, Shell::*, WindowsAndMessaging::*},
};
const OPEN: u32 = 1;
const SETUP: u32 = 2;
const DATA: u32 = 3;
const DISABLE: u32 = 4;
const EXIT: u32 = 5;
const CALLBACK: u32 = WM_APP + 1;
static ACTION: AtomicU32 = AtomicU32::new(0);
fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.as_ref().encode_wide().chain(Some(0)).collect()
}
pub fn error(message: &str) {
    unsafe {
        MessageBoxW(
            null_mut(),
            wide(message).as_ptr(),
            wide("Vision Overlay").as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}
fn ask(message: &str) -> bool {
    unsafe {
        MessageBoxW(
            null_mut(),
            wide(message).as_ptr(),
            wide("Vision Overlay").as_ptr(),
            MB_YESNO | MB_ICONQUESTION,
        ) == IDYES
    }
}
fn info(message: &str) {
    unsafe {
        MessageBoxW(
            null_mut(),
            wide(message).as_ptr(),
            wide("Vision Overlay").as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}
fn open(target: impl AsRef<std::ffi::OsStr>) -> Result<()> {
    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            wide("open").as_ptr(),
            wide(target).as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        )
    };
    ensure!((result as isize) > 32, "Cannot open the browser or folder");
    Ok(())
}
fn choose(title: &str, filter: &str) -> Result<Option<PathBuf>> {
    let mut buffer = vec![0u16; 32768];
    let title = wide(title);
    let filter = wide(filter);
    let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    dialog.lpstrFile = buffer.as_mut_ptr();
    dialog.nMaxFile = buffer.len() as u32;
    dialog.lpstrTitle = title.as_ptr();
    dialog.lpstrFilter = filter.as_ptr();
    dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
    if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
        let code = unsafe { CommDlgExtendedError() };
        ensure!(code == 0, "File dialog failed: {code}");
        return Ok(None);
    }
    use std::os::windows::ffi::OsStringExt;
    let end = buffer.iter().position(|n| *n == 0).unwrap();
    Ok(Some(std::ffi::OsString::from_wide(&buffer[..end]).into()))
}
fn fingerprint(path: &Path, expected: &str) -> Result<()> {
    let mut file = fs::File::open(path).context("Game client file missing")?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    ensure!(
        format!("{:x}", hash.finalize()) == expected,
        "This game build is not supported by the startup packet decoder. A compatible Vision Overlay build is required."
    );
    Ok(())
}
fn configure(layout: &Layout, disable: bool) -> Result<()> {
    let script = layout.components.join("scripts/Configure-AutoCapture.ps1");
    let args = format!(
        "-NoProfile -ExecutionPolicy RemoteSigned -WindowStyle Hidden -File \"{}\"{}",
        script.display(),
        if disable { " -Disable" } else { "" }
    );
    let mut task: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    let verb = wide("runas");
    let program = wide("powershell.exe");
    let args = wide(args);
    let cwd = wide(&layout.root);
    task.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    task.fMask = SEE_MASK_NOCLOSEPROCESS;
    task.lpVerb = verb.as_ptr();
    task.lpFile = program.as_ptr();
    task.lpParameters = args.as_ptr();
    task.lpDirectory = cwd.as_ptr();
    task.nShow = SW_HIDE;
    ensure!(
        unsafe { ShellExecuteExW(&mut task) } != 0,
        "Capture configuration was cancelled or could not start"
    );
    let process = Handle(task.hProcess);
    ensure!(
        unsafe { WaitForSingleObject(process.0, 120000) } == WAIT_OBJECT_0,
        "Capture configuration timed out"
    );
    let mut code = 1;
    unsafe { GetExitCodeProcess(process.0, &mut code) };
    ensure!(
        code == 0,
        "Capture configuration failed; see data/auto-capture logs"
    );
    Ok(())
}
fn setup(layout: &Layout) -> Result<()> {
    info(
        "Настройка захвата: выберите GenshinImpact.exe из папки игры. Для настройки автозахвата Windows запросит права администратора. Необходимые данные декодера уже включены в комплект. Инструкция — в README.txt комплекта.",
    );
    let bytes = layout.bundled_keys()?;
    let Some(game) = choose(
        "Выберите GenshinImpact.exe",
        "Genshin Impact\0GenshinImpact.exe\0\0",
    )?
    else {
        return Ok(());
    };
    ensure!(
        game.file_name().is_some_and(|n| n == "GenshinImpact.exe"),
        "Select GenshinImpact.exe"
    );
    let client = game.parent().context("Game directory missing")?;
    fingerprint(
        &game,
        "b3001fc110b63e0e0ce24a77386e71d0eefaffaae244ea0c72645a659cb5ea6e",
    )?;
    fingerprint(
        &client.join("GenshinImpact_Data/Managed/Metadata/global-metadata.dat"),
        "4ed10538735861ad87b027d30039e2afb92e334f1b4de4f643e17c9c79d19fd7",
    )?;
    if layout.data.join("body-codec.json").exists()
        && !ask("Заменить ранее настроенный путь к игре?")
    {
        return Ok(());
    }
    fs::write(layout.data.join("gi.json"), bytes)?;
    fs::write(
        layout.data.join("body-codec.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"client_directory":client}))?,
    )?;
    configure(layout, false)?;
    info(
        "Автозахват настроен. Если игра уже запущена, полностью закройте и запустите её снова. После перемещения папки приложения повторите настройку.",
    );
    Ok(())
}
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
struct Server {
    child: Child,
    stop: PathBuf,
}
impl Server {
    fn shutdown(&mut self) {
        let _ = fs::write(&self.stop, "stop");
        for _ in 0..150 {
            if self.child.try_wait().ok().flatten().is_some() {
                let _ = fs::remove_file(&self.stop);
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        // Never terminate a writer in the middle of a transaction or backup.
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown();
    }
}
unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if message == CALLBACK {
        if l as u32 == WM_LBUTTONDBLCLK {
            ACTION.store(OPEN, Ordering::Relaxed);
        }
        if l as u32 == WM_RBUTTONUP {
            unsafe {
                let menu = CreatePopupMenu();
                for (id, label) in [
                    (OPEN, "Открыть Vision Overlay"),
                    (SETUP, "Настроить / включить автозахват"),
                    (DISABLE, "Отключить автоматический захват"),
                    (DATA, "Открыть папку данных"),
                    (EXIT, "Закрыть интерфейс и сервер"),
                ] {
                    AppendMenuW(menu, MF_STRING, id as usize, wide(label).as_ptr());
                }
                let mut point: POINT = std::mem::zeroed();
                GetCursorPos(&mut point);
                SetForegroundWindow(hwnd);
                let selected = TrackPopupMenu(
                    menu,
                    TPM_RETURNCMD | TPM_NONOTIFY,
                    point.x,
                    point.y,
                    0,
                    hwnd,
                    null(),
                ) as u32;
                DestroyMenu(menu);
                if selected != 0 {
                    ACTION.store(selected, Ordering::Relaxed);
                }
            }
        }
        return 0;
    }
    unsafe { DefWindowProcW(hwnd, message, w, l) }
}
struct Tray {
    hwnd: HWND,
    icon: NOTIFYICONDATAW,
}
impl Tray {
    fn new() -> Result<Self> {
        unsafe {
            let class = wide("VisionOverlayPortableTray");
            let module = GetModuleHandleW(null());
            let mut wc: WNDCLASSW = std::mem::zeroed();
            wc.lpfnWndProc = Some(window_proc);
            wc.hInstance = module;
            wc.lpszClassName = class.as_ptr();
            ensure!(RegisterClassW(&wc) != 0, "Cannot register launcher window");
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                wide("Vision Overlay").as_ptr(),
                0,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                module,
                null(),
            );
            ensure!(!hwnd.is_null(), "Cannot create launcher window");
            let mut icon: NOTIFYICONDATAW = std::mem::zeroed();
            icon.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            icon.hWnd = hwnd;
            icon.uID = 1;
            icon.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
            icon.uCallbackMessage = CALLBACK;
            icon.hIcon = LoadIconW(null_mut(), IDI_APPLICATION);
            let label = wide("Vision Overlay — двойной клик: открыть; ПКМ: меню");
            icon.szTip[..label.len()].copy_from_slice(&label);
            ensure!(
                Shell_NotifyIconW(NIM_ADD, &icon) != 0,
                "Cannot add notification icon"
            );
            Ok(Self { hwnd, icon })
        }
    }
}
impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &self.icon);
            DestroyWindow(self.hwnd);
        }
    }
}
pub fn run(layout: Layout) -> Result<()> {
    let port = layout.port()?;
    let url = format!("http://127.0.0.1:{port}/");
    let stop = layout.data.join("launcher-stop");
    if std::env::args().any(|a| a == "--stop") {
        fs::write(stop, "stop")?;
        return Ok(());
    }
    let id = format!(
        "Local\\VisionOverlayPortable-{:x}",
        Sha256::digest(layout.root.to_string_lossy().to_lowercase().as_bytes())
    );
    let handle = unsafe { CreateMutexW(null(), 0, wide(id).as_ptr()) };
    let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    ensure!(
        !handle.is_null(),
        "Cannot acquire application instance lock"
    );
    let _instance = Handle(handle);
    if exists {
        if std::env::args().any(|a| a == "--no-browser") {
            return Ok(());
        }
        return open(&url);
    }
    if stop.exists() {
        fs::remove_file(&stop)?;
    }
    // Never silently connect to another installation on an occupied port.
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let reservation = TcpListener::bind(address).context(
        "Application port is occupied. Close the other instance or set port in data/launcher.json",
    )?;
    let server_stop = layout
        .data
        .join(format!("server-stop-{}", uuid::Uuid::new_v4()));
    let stdout = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(layout.data.join("logs/launcher-server.stdout.log"))?;
    let stderr = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(layout.data.join("logs/launcher-server.stderr.log"))?;
    drop(reservation);
    let child = Command::new(layout.components.join("vision-server.exe"))
        .args(["--initialize", "--port", &port.to_string(), "--db"])
        .arg(layout.data.join("vision-overlay.sqlite3"))
        .arg("--shutdown-file")
        .arg(&server_stop)
        .current_dir(&layout.root)
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .context("Cannot start packaged server")?;
    let mut server = Server {
        child,
        stop: server_stop,
    };
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if let Some(status) = server.child.try_wait()? {
            bail!("Server exited ({status}); see data/logs/launcher-server.stderr.log")
        }
        if let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(200)) {
            stream.set_read_timeout(Some(Duration::from_millis(500)))?;
            let _ = write!(
                stream,
                "GET /api/v1/state HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
            );
            let mut bytes = [0; 256];
            if stream
                .read(&mut bytes)
                .is_ok_and(|n| bytes[..n].starts_with(b"HTTP/1.1 200"))
            {
                break;
            }
        }
        ensure!(
            Instant::now() < deadline,
            "Server startup timed out; see data/logs"
        );
        thread::sleep(Duration::from_millis(100));
    }
    let headless = std::env::args().any(|a| a == "--headless");
    let _tray = if headless { None } else { Some(Tray::new()?) };
    if !headless && !std::env::args().any(|a| a == "--no-browser") {
        open(&url)?;
    }
    if !headless
        && !layout.data.join("first-run.json").exists()
        && !std::env::args().any(|a| a == "--no-browser")
    {
        fs::write(layout.data.join("first-run.json"), "{}")?;
        if ask(
            "Vision Overlay работает в трее. Настроить захват игры сейчас? Для просмотра приложения можно выбрать «Нет» и настроить захват позже через меню значка.",
        ) && let Err(e) = setup(&layout)
        {
            error(&format!("{e:#}"));
        }
    }
    loop {
        let mut message: MSG = unsafe { std::mem::zeroed() };
        while unsafe { PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) } != 0 {
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        let action = ACTION.swap(0, Ordering::Relaxed);
        let result = match action {
            OPEN => open(&url),
            DATA => open(&layout.data),
            SETUP => setup(&layout),
            DISABLE => configure(&layout, true),
            _ => Ok(()),
        };
        if let Err(e) = result {
            error(&format!("{e:#}"));
        }
        if action == EXIT || stop.exists() {
            let _ = fs::remove_file(&stop);
            break;
        }
        if let Some(status) = server.child.try_wait()? {
            bail!("Application server stopped ({status}); see data/logs")
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

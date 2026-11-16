//! Windows 10 PktMon acquisition. Own ETW session; never clears global filters.
use crate::{
    CaptureBackend, Packet, PacketPayload, RealtimeStats, filter::PktMonFilter,
    legacy::driver::Driver,
};
use etherparse::{SlicedPacket, TransportSlice};
use std::{
    ffi::{CString, c_void},
    io,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering::Relaxed},
        mpsc::{self, Receiver, RecvError, RecvTimeoutError, SyncSender, TryRecvError},
    },
    thread::JoinHandle,
    time::Duration,
};
use windows::{
    Win32::{Foundation::INVALID_HANDLE_VALUE, System::Diagnostics::Etw::*},
    core::{GUID, PCWSTR, PSTR},
    w,
};

const PROVIDER: GUID = GUID::from_values(
    0x4d4f80d9,
    0xc8bd,
    0x4d73,
    [0xbb, 0x5b, 0x19, 0xc9, 0x04, 0x02, 0xc5, 0xac],
);
static SESSION_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn check(code: u32) -> io::Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code as i32))
    }
}

fn empty_filters(text: &str) -> bool {
    let text = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    matches!(
        text.trim_end_matches('.'),
        "packet filters: none"
            | "no filters"
            | "no packet filters"
            | "no filters defined"
            | "no packet filters defined"
            | "нет фильтров"
            | "фильтры отсутствуют"
            | "фильтры пакетов: нет"
            | "фильтры пакетов: отсутствуют"
    )
}

fn console_text(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_owned();
    }
    use windows::Win32::Globalization::{
        GetOEMCP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS, MultiByteToWideChar,
    };
    let size =
        unsafe { MultiByteToWideChar(GetOEMCP(), MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, None) };
    if size <= 0 {
        return String::new();
    }
    let mut wide = vec![0; size as usize];
    let count = unsafe {
        MultiByteToWideChar(
            GetOEMCP(),
            MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0),
            bytes,
            Some(&mut wide),
        )
    };
    if count <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&wide[..count as usize])
}

fn require_empty_filters() -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    // No localized table parsing or removal: unknown output conservatively refuses capture.
    let exe = std::path::PathBuf::from(
        std::env::var_os("WINDIR").ok_or_else(|| io::Error::other("WINDIR unavailable"))?,
    )
    .join("System32/pktmon.exe");
    let output = Command::new(exe)
        .args(["filter", "list"])
        .creation_flags(0x08000000)
        .output()?;
    if !output.status.success() || !empty_filters(&console_text(&output.stdout)) {
        return Err(io::Error::other(
            "PktMon/ETW requires empty global filters. Existing filters or unrecognized filter output were left unchanged; finish external diagnostics first.",
        ));
    }
    Ok(())
}

fn pktmon(args: &[std::ffi::OsString]) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    let exe = std::path::PathBuf::from(
        std::env::var_os("WINDIR").ok_or_else(|| io::Error::other("WINDIR unavailable"))?,
    )
    .join("System32/pktmon.exe");
    let output = Command::new(exe)
        .args(args)
        .creation_flags(0x08000000)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "PktMon command failed with exit code {:?}; external sessions were not stopped",
            output.status.code()
        )));
    }
    Ok(())
}

#[repr(C)]
struct Properties {
    value: EVENT_TRACE_PROPERTIES,
    name: [u8; 128],
}
impl Properties {
    fn new(name: &CString) -> Self {
        let mut p = Self {
            value: EVENT_TRACE_PROPERTIES::default(),
            name: [0; 128],
        };
        let bytes = name.as_bytes_with_nul();
        p.name[..bytes.len()].copy_from_slice(bytes);
        p.value.Wnode.BufferSize = std::mem::size_of::<Self>() as u32;
        p.value.Wnode.Flags = WNODE_FLAG_TRACED_GUID;
        p.value.Wnode.ClientContext = 1;
        p.value.LogFileMode = EVENT_TRACE_REAL_TIME_MODE | EVENT_TRACE_INDEPENDENT_SESSION_MODE;
        p.value.BufferSize = 64; // KB per buffer, not MB. Bounded at 16 MiB.
        p.value.MinimumBuffers = 64;
        p.value.MaximumBuffers = 256;
        p.value.FlushTimer = 1;
        p.value.LoggerNameOffset = std::mem::offset_of!(Self, name) as u32;
        p
    }
}

struct Session {
    handle: CONTROLTRACE_HANDLE,
    name: CString,
    owned: bool,
}
impl Session {
    fn new() -> io::Result<Self> {
        let name = CString::new(format!(
            "VisionOverlay-ETW-{}-{}",
            std::process::id(),
            SESSION_ID.fetch_add(1, Relaxed)
        ))
        .unwrap();
        let mut p = Properties::new(&name);
        let mut handle = CONTROLTRACE_HANDLE::default();
        unsafe {
            check(
                StartTraceA(
                    &mut handle,
                    windows::core::PCSTR(name.as_ptr().cast()),
                    &mut p.value,
                )
                .0,
            )?;
        }
        let session = Self {
            handle,
            name,
            owned: true,
        };
        unsafe {
            check(
                EnableTraceEx2(
                    handle,
                    &PROVIDER,
                    EVENT_CONTROL_CODE_ENABLE_PROVIDER.0,
                    TRACE_LEVEL_INFORMATION as u8,
                    0x10,
                    0,
                    0,
                    None,
                )
                .0,
            )?;
        }
        Ok(session)
    }
    fn sample(&self, stats: &RealtimeStats) -> io::Result<()> {
        let mut p = Properties::new(&self.name);
        unsafe {
            check(ControlTraceA(self.handle, None, &mut p.value, EVENT_TRACE_CONTROL_QUERY).0)?;
        }
        stats
            .missed_write
            .fetch_max(u64::from(p.value.EventsLost), Relaxed);
        stats
            .missed_read
            .fetch_max(u64::from(p.value.RealTimeBuffersLost), Relaxed);
        Ok(())
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        if self.owned {
            let mut p = Properties::new(&self.name);
            unsafe {
                let _ = ControlTraceA(self.handle, None, &mut p.value, EVENT_TRACE_CONTROL_STOP);
            }
            self.owned = false;
        }
    }
}

struct Context {
    sender: SyncSender<Packet>,
    stats: Arc<RealtimeStats>,
    ended: Arc<AtomicBool>,
    notify: Arc<tokio::sync::Notify>,
}
fn deliver(ctx: &Context, packet: Packet) {
    match ctx.sender.try_send(packet) {
        Ok(()) => ctx.notify.notify_one(),
        Err(mpsc::TrySendError::Full(_)) => {
            ctx.stats.callback_queue_drops.fetch_add(1, Relaxed);
        }
        Err(mpsc::TrySendError::Disconnected(_)) => {}
    }
}
struct Consumer {
    handle: PROCESSTRACE_HANDLE,
    thread: Option<JoinHandle<()>>,
    context: Box<Context>,
}
unsafe fn property(record: *mut EVENT_RECORD, name: PCWSTR, bytes: &mut [u8]) -> io::Result<()> {
    unsafe {
        check(TdhGetProperty(
            record,
            None,
            &[PROPERTY_DATA_DESCRIPTOR {
                PropertyName: name.0 as u64,
                ArrayIndex: u32::MAX,
                Reserved: 0,
            }],
            bytes,
        ))
    }
}
unsafe fn scalar<const N: usize>(record: *mut EVENT_RECORD, name: PCWSTR) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    unsafe {
        property(record, name, &mut bytes)?;
    }
    Ok(bytes)
}
fn game_packet(payload: &[u8], kind: u32) -> bool {
    let parsed = match kind {
        1 => SlicedPacket::from_ethernet(payload),
        3 => SlicedPacket::from_ip(payload),
        _ => return false,
    };
    parsed.is_ok_and(|p| matches!(p.transport,Some(TransportSlice::Udp(u)) if [u.source_port(),u.destination_port()].into_iter().any(|p|matches!(p,22101|22102))))
}
unsafe fn decode(record: *mut EVENT_RECORD) -> io::Result<Option<Packet>> {
    let kind = u32::from_le_bytes(unsafe { scalar(record, w!("PacketType"))? });
    if !matches!(kind, 1 | 3) {
        return Ok(None);
    }
    let size = u32::from_le_bytes(unsafe { scalar(record, w!("LoggedPayloadSize"))? });
    // A truncated target packet must not silently become an apparently continuous stream.
    let original = u32::from_le_bytes(unsafe { scalar(record, w!("OriginalPayloadSize"))? });
    if size == 0 || size > 131072 || original != size {
        return Err(io::Error::other("Invalid or truncated ETW payload"));
    }
    let mut bytes = vec![0; size as usize];
    unsafe {
        property(record, w!("Payload"), &mut bytes)?;
    }
    if !game_packet(&bytes, kind) {
        return Ok(None);
    }
    let id = u16::from_le_bytes(unsafe { scalar(record, w!("ComponentId"))? });
    Ok(Some(Packet {
        component_id: id,
        payload: if kind == 1 {
            PacketPayload::Ethernet(bytes)
        } else {
            PacketPayload::IP(bytes)
        },
    }))
}
unsafe extern "system" fn event(record: *mut EVENT_RECORD) {
    if record.is_null() {
        return;
    }
    let r = unsafe { &*record };
    if !r.UserContext.is_null() {
        unsafe { &*(r.UserContext as *const Context) }
            .stats
            .etw_events
            .fetch_add(1, Relaxed);
    }
    if r.UserContext.is_null()
        || r.EventHeader.ProviderId != PROVIDER
        || r.EventHeader.EventDescriptor.Id != 160
    {
        return;
    }
    let ctx = unsafe { &*(r.UserContext as *const Context) };
    ctx.stats.etw_payload_events.fetch_add(1, Relaxed);
    match unsafe { decode(record) } {
        Ok(Some(packet)) => deliver(ctx, packet),
        Ok(None) => {}
        Err(_) => {
            ctx.stats.invalid_descriptors.fetch_add(1, Relaxed);
        }
    }
}
impl Consumer {
    fn new(
        session: &Session,
        stats: Arc<RealtimeStats>,
        notify: Arc<tokio::sync::Notify>,
    ) -> io::Result<(Self, Receiver<Packet>)> {
        let (sender, receiver) = mpsc::sync_channel(8192);
        let ended = Arc::new(AtomicBool::new(false));
        let mut context = Box::new(Context {
            sender,
            stats,
            ended: ended.clone(),
            notify,
        });
        let mut file = EVENT_TRACE_LOGFILEA::default();
        file.LoggerName = PSTR(session.name.as_ptr() as *mut u8);
        file.Anonymous1.ProcessTraceMode =
            PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
        file.Anonymous2.EventRecordCallback = Some(event);
        file.Context = (&mut *context as *mut Context).cast::<c_void>();
        let handle = unsafe { OpenTraceA(&mut file) };
        if handle.0 == INVALID_HANDLE_VALUE.0 as u64 {
            return Err(io::Error::last_os_error());
        }
        let raw_handle = handle.0;
        let thread = match std::thread::Builder::new()
            .name("pktmon-etw-consumer".into())
            .spawn(move || {
                unsafe {
                    let _ = ProcessTrace(&[PROCESSTRACE_HANDLE(raw_handle)], None, None);
                }
                ended.store(true, Relaxed);
            }) {
            Ok(t) => t,
            Err(e) => {
                unsafe {
                    let _ = CloseTrace(handle);
                }
                return Err(e);
            }
        };
        Ok((
            Self {
                handle,
                thread: Some(thread),
                context,
            },
            receiver,
        ))
    }
}
impl Drop for Consumer {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseTrace(self.handle);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) struct EtwBackend {
    driver: Driver,
    session: Option<Session>,
    consumer: Option<Consumer>,
    receiver: Option<Receiver<Packet>>,
    stats: Arc<RealtimeStats>,
    notify: Arc<tokio::sync::Notify>,
    component: Option<u32>,
    running: bool,
    scratch: tempfile::TempDir,
}
impl std::fmt::Debug for EtwBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PktMon/ETW NIC backend")
    }
}
impl EtwBackend {
    pub fn new(stats: Arc<RealtimeStats>, component: Option<u32>) -> io::Result<Self> {
        let driver = Driver::open_idle()?;
        require_empty_filters()?;
        Ok(Self {
            driver,
            session: None,
            consumer: None,
            receiver: None,
            stats,
            notify: Arc::new(tokio::sync::Notify::new()),
            component,
            running: false,
            scratch: tempfile::Builder::new()
                .prefix("vision-overlay-etw-")
                .tempdir()?,
        })
    }
}
impl CaptureBackend for EtwBackend {
    fn start(&mut self) -> io::Result<()> {
        if self.running {
            return Ok(());
        }
        require_empty_filters()?;
        let session = Session::new()?;
        let (consumer, receiver) =
            Consumer::new(&session, self.stats.clone(), self.notify.clone())?;
        if self.driver.is_running()? {
            return Err(io::Error::other("PktMon is already in use"));
        }
        // Documented CLI configures complete flow packets; no reliance on the
        // undocumented PktmonStart configuration layout. Memory logging is
        // bounded to 4 MiB; its scratch ETL is discarded after shutdown.
        let mut args: Vec<std::ffi::OsString> = ["start", "--capture", "--comp"]
            .into_iter()
            .map(Into::into)
            .collect();
        args.push(
            self.component
                .map(|c| c.to_string())
                .unwrap_or_else(|| "nics".into())
                .into(),
        );
        args.extend(
            [
                "--type",
                "flow",
                "--pkt-size",
                "0",
                "--flags",
                "0x10",
                "--log-mode",
                "memory",
                "--file-size",
                "4",
                "--file-name",
            ]
            .into_iter()
            .map(Into::into),
        );
        args.push(self.scratch.path().join("capture.etl").into_os_string());
        pktmon(&args)?;
        self.session = Some(session);
        self.consumer = Some(consumer);
        self.receiver = Some(receiver);
        self.running = true;
        self.stats.backend_kind.store(2, Relaxed);
        Ok(())
    }
    fn stop(&mut self) -> io::Result<()> {
        if !self.running {
            return Ok(());
        }
        let result = pktmon(&["stop".into()]);
        if result.is_ok() {
            self.running = false;
        }
        if let Some(s) = &self.session {
            let _ = s.sample(&self.stats);
        }
        self.session.take();
        self.consumer.take();
        self.receiver.take();
        result
    }
    fn poll_health(&mut self) -> io::Result<()> {
        if !self.driver.is_running()? {
            self.stats.invalid_descriptors.fetch_add(1, Relaxed);
            return Err(io::Error::other(
                "Global PktMon capture stopped unexpectedly",
            ));
        }
        if self
            .consumer
            .as_ref()
            .is_some_and(|c| c.context.ended.load(Relaxed))
        {
            self.stats.invalid_descriptors.fetch_add(1, Relaxed);
            return Err(io::Error::other("ETW consumer terminated unexpectedly"));
        }
        if let Some(s) = &self.session {
            if let Err(e) = s.sample(&self.stats) {
                self.stats.invalid_descriptors.fetch_add(1, Relaxed);
                return Err(e);
            }
        }
        Ok(())
    }
    fn unload(&mut self) -> io::Result<()> {
        self.stop()
    }
    fn add_filter(&mut self, filter: PktMonFilter) -> io::Result<()> {
        let mut expected = PktMonFilter {
            name: filter.name.clone(),
            transport_protocol: Some(crate::filter::TransportProtocol::UDP),
            port: filter.port,
            ..Default::default()
        };
        if !matches!(
            expected.port,
            crate::filter::OptionPair::Some(22101 | 22102)
        ) {
            expected.port = crate::filter::OptionPair::None;
        }
        if filter != expected || matches!(expected.port, crate::filter::OptionPair::None) {
            return Err(io::Error::other(
                "ETW backend only supports game UDP filtering",
            ));
        }
        // Software allowlist before the bounded callback queue; global filters unchanged.
        Ok(())
    }
    fn next_packet(&self) -> Result<Packet, RecvError> {
        self.receiver.as_ref().ok_or(RecvError)?.recv()
    }
    fn next_packet_timeout(&self, t: Duration) -> Result<Packet, RecvTimeoutError> {
        self.receiver
            .as_ref()
            .ok_or(RecvTimeoutError::Disconnected)?
            .recv_timeout(t)
    }
    fn try_next_packet(&self) -> Result<Packet, TryRecvError> {
        self.receiver
            .as_ref()
            .ok_or(TryRecvError::Disconnected)?
            .try_recv()
    }
    fn notify(&self) -> Option<Arc<tokio::sync::Notify>> {
        Some(self.notify.clone())
    }
}
impl Drop for EtwBackend {
    fn drop(&mut self) {
        if let Err(e) = self.stop() {
            log::error!("Owned PktMon capture could not be stopped: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_queue_loss_and_receiver_shutdown_are_nonblocking() {
        let stats = Arc::new(RealtimeStats::default());
        let (sender, receiver) = mpsc::sync_channel(1);
        let ctx = Context {
            sender,
            stats: stats.clone(),
            ended: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(tokio::sync::Notify::new()),
        };
        let packet = || Packet {
            component_id: 1,
            payload: PacketPayload::Ethernet(vec![1, 2, 3]),
        };
        deliver(&ctx, packet());
        deliver(&ctx, packet());
        assert_eq!(stats.callback_queue_drops.load(Relaxed), 1);
        assert_eq!(receiver.try_iter().count(), 1);
        drop(receiver);
        deliver(&ctx, packet());
        assert_eq!(stats.callback_queue_drops.load(Relaxed), 1);
    }
    #[test]
    fn filter_output_never_guesses_nonempty_or_unrecognized_state() {
        assert!(empty_filters("Packet Filters:\r\n    None\r\n"));
        for s in [
            "",
            "Packet Filters: Name Port\nfoo 443",
            "Access denied",
            "None extra",
        ] {
            assert!(!empty_filters(s));
        }
    }
    #[test]
    fn only_game_udp_enters_queue_and_raw_recording() {
        for v6 in [false, true] {
            for port in [22101, 22102, 443] {
                let b = if v6 {
                    etherparse::PacketBuilder::ipv6([1; 16], [2; 16], 64)
                } else {
                    etherparse::PacketBuilder::ipv4([1; 4], [2; 4], 64)
                }
                .udp(40000, port);
                let mut bytes = Vec::new();
                b.write(&mut bytes, &[1, 2, 3]).unwrap();
                assert_eq!(game_packet(&bytes, 3), port != 443);
                let mut eth = vec![0; 12];
                eth.extend_from_slice(if v6 { &[0x86, 0xdd] } else { &[0x08, 0x00] });
                eth.extend_from_slice(&bytes);
                assert_eq!(game_packet(&eth, 1), port != 443);
            }
        }
        assert!(!game_packet(&[0; 20], 1));
    }
    #[test]
    #[ignore = "requires a private saved PktMon ETL fixture in VISION_TEST_ETL"]
    fn saved_etl_schema_and_payload_decode() {
        let path =
            CString::new(std::env::var("VISION_TEST_ETL").expect("VISION_TEST_ETL required"))
                .unwrap();
        let stats = Arc::new(RealtimeStats::default());
        let (sender, receiver) = mpsc::sync_channel(8192);
        let mut context = Box::new(Context {
            sender,
            stats: stats.clone(),
            ended: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(tokio::sync::Notify::new()),
        });
        let mut file = EVENT_TRACE_LOGFILEA::default();
        file.LogFileName = PSTR(path.as_ptr() as *mut u8);
        file.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_EVENT_RECORD;
        file.Anonymous2.EventRecordCallback = Some(event);
        file.Context = (&mut *context as *mut Context).cast();
        let drain = std::thread::spawn(move || receiver.iter().count());
        let handle = unsafe { OpenTraceA(&mut file) };
        assert_ne!(handle.0, INVALID_HANDLE_VALUE.0 as u64);
        unsafe {
            check(ProcessTrace(&[handle], None, None).0).unwrap();
            let _ = CloseTrace(handle);
        }
        drop(context);
        let count = drain.join().unwrap();
        assert!(count > 0, "No game UDP packets decoded from ETL");
        assert_eq!(
            stats.invalid_descriptors.load(Relaxed),
            0,
            "ETW schema mismatch or truncated packets"
        );
        assert_eq!(
            stats.callback_queue_drops.load(Relaxed),
            0,
            "Choose a smaller fixture"
        );
        eprintln!("Decoded {count} complete game packets from saved ETL");
    }
}

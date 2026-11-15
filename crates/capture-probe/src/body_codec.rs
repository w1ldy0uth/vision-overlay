//! Version-pinned, isolated adapter to the locally installed client's codec.
//! Never loads a game entrypoint, resolves imports, or touches another process.
use anyhow::{Context, Result, ensure};
use std::{
    io::{Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
};

pub const LIMIT: usize = 16 * 1024 * 1024;
pub struct Worker {
    child: Child,
    input: std::process::ChildStdin,
    responses: std::sync::mpsc::Receiver<std::result::Result<Vec<u8>, String>>,
    reader: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
impl Worker {
    pub fn start(config: &Path) -> Result<Self> {
        use std::os::windows::process::CommandExt;
        let mut child = Command::new(std::env::current_exe()?)
            .arg("--body-codec-worker")
            .arg(config)
            .creation_flags(0x08000000)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child.stdin.take().context("Codec input")?;
        let mut output = child.stdout.take().context("Codec output")?;
        let (send, responses) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut ready = [0];
            if output.read_exact(&mut ready).is_err() || ready != [1] {
                let _ = send.send(Err("Codec initialization rejected".into()));
                return;
            }
            if send.send(Ok(vec![1])).is_err() {
                return;
            }
            loop {
                let message = (|| -> Result<Vec<u8>> {
                    let mut n = [0; 4];
                    output.read_exact(&mut n)?;
                    let n = u32::from_le_bytes(n) as usize;
                    ensure!(n <= LIMIT, "Codec reply limit");
                    let mut data = vec![0; n];
                    output.read_exact(&mut data)?;
                    Ok(data)
                })();
                let failed = message.is_err();
                if send
                    .send(message.map_err(|_| "Codec pipe closed".into()))
                    .is_err()
                    || failed
                {
                    return;
                }
            }
        });
        let mut worker = Self {
            child,
            input,
            responses,
            reader: Some(reader),
        };
        ensure!(
            worker.response(std::time::Duration::from_secs(30))? == [1],
            "Codec initialization rejected"
        );
        Ok(worker)
    }
    fn response(&mut self, timeout: std::time::Duration) -> Result<Vec<u8>> {
        match self.responses.recv_timeout(timeout) {
            Ok(Ok(data)) => Ok(data),
            _ => {
                let _ = self.child.kill();
                anyhow::bail!("Codec worker failed or timed out");
            }
        }
    }
    pub fn decode(&mut self, body: &[u8], expected: usize) -> std::result::Result<Vec<u8>, String> {
        self.request(body, expected)
            .map_err(|_| "Isolated body decoder failed".into())
    }
    fn request(&mut self, body: &[u8], expected: usize) -> Result<Vec<u8>> {
        ensure!(
            !body.is_empty() && body.len() <= LIMIT && expected > 0 && expected <= LIMIT,
            "Codec size limit"
        );
        self.input.write_all(&(body.len() as u32).to_le_bytes())?;
        self.input.write_all(&(expected as u32).to_le_bytes())?;
        self.input.write_all(body)?;
        self.input.flush()?;
        let out = self.response(std::time::Duration::from_secs(10))?;
        ensure!(out.len() == expected, "Codec rejected body");
        Ok(out)
    }
}

fn u16le(b: &[u8], o: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        b.get(o..o + 2).context("PE u16")?.try_into()?,
    ))
}
fn u32le(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(o..o + 4).context("PE u32")?.try_into()?,
    ))
}
fn u64le(b: &[u8], o: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(
        b.get(o..o + 8).context("PE u64")?.try_into()?,
    ))
}
fn verified_file(path: &Path, hash: &str, max: u64) -> Result<Vec<u8>> {
    use sha2::{Digest, Sha256};
    ensure!(
        std::fs::metadata(path)?.len() <= max,
        "Client file size limit"
    );
    let data = std::fs::read(path)?;
    ensure!(
        format!("{:x}", Sha256::digest(&data)) == hash,
        "Unsupported client version (codec disabled)"
    );
    Ok(data)
}
struct Image {
    base: *mut u8,
    sections: Vec<(usize, usize)>,
}
impl Drop for Image {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Memory::VirtualFree(self.base.cast(), 0, 0x8000);
        }
    }
}
impl Image {
    fn load(data: &[u8]) -> Result<Self> {
        use windows_sys::Win32::System::Memory::{VirtualAlloc, VirtualProtect};
        let pe = u32le(data, 0x3c)? as usize;
        ensure!(data.get(pe..pe + 4) == Some(b"PE\0\0"), "Invalid PE");
        ensure!(
            u16le(data, pe + 4)? == 0x8664 && u16le(data, pe + 24)? == 0x20b,
            "PE architecture"
        );
        let preferred = u64le(data, pe + 24 + 24)?;
        let size = u32le(data, pe + 24 + 56)? as usize;
        ensure!(size <= 512 * 1024 * 1024, "PE image limit");
        let base = unsafe { VirtualAlloc(std::ptr::null(), size, 0x2000, 1) }.cast::<u8>();
        ensure!(!base.is_null(), "Reserve codec image");
        let mut image = Self {
            base,
            sections: Vec::new(),
        };
        let table = pe + 24 + u16le(data, pe + 20)? as usize;
        let mut reloc = None;
        for i in 0..u16le(data, pe + 6)? as usize {
            let h = table + i * 40;
            let name = data.get(h..h + 8).context("PE section")?;
            let rva = u32le(data, h + 12)? as usize;
            let raw = u32le(data, h + 20)? as usize;
            let bytes = u32le(data, h + 16)? as usize;
            let span = bytes.max(u32le(data, h + 8)? as usize);
            ensure!(
                rva.checked_add(span).is_some_and(|end| end <= size),
                "PE section bounds"
            );
            if name.starts_with(b".reloc") {
                reloc = Some((raw, bytes));
            }
            if ![
                b".text\0\0\0".as_slice(),
                b".rdata\0\0",
                b".data\0\0\0",
                b"_RDATA\0\0",
            ]
            .contains(&name)
            {
                continue;
            }
            let src = data.get(raw..raw + bytes).context("PE section bytes")?;
            ensure!(
                !unsafe { VirtualAlloc(base.wrapping_add(rva).cast(), span, 0x1000, 4) }.is_null(),
                "Commit codec section"
            );
            unsafe {
                std::ptr::copy_nonoverlapping(src.as_ptr(), base.add(rva), bytes);
            }
            image.sections.push((rva, span));
        }
        let (raw, len) = reloc.context("PE relocations missing")?;
        let rel = data.get(raw..raw + len).context("PE relocation bounds")?;
        let mut at = 0;
        let delta = (base as u64).wrapping_sub(preferred);
        while at + 8 <= rel.len() {
            let page = u32le(rel, at)? as usize;
            let n = u32le(rel, at + 4)? as usize;
            if n == 0 {
                break;
            }
            ensure!(
                n >= 8 && n.is_multiple_of(2) && at + n <= rel.len(),
                "PE relocation block"
            );
            for o in (at + 8..at + n).step_by(2) {
                let entry = u16le(rel, o)?;
                let rva = page + (entry & 4095) as usize;
                if entry >> 12 == 10
                    && image
                        .sections
                        .iter()
                        .any(|(s, n)| rva >= *s && rva + 8 <= s + n)
                {
                    unsafe {
                        let ptr = base.add(rva).cast::<u64>();
                        ptr.write_unaligned(ptr.read_unaligned().wrapping_add(delta));
                    }
                }
            }
            at += n;
        }
        // W^X: executable text is read-only; constants are read-only; data stays RW.
        for (rva, span) in &image.sections {
            let protection = if *rva == 0x1000 {
                0x20
            } else if *rva == 0x5597000 {
                4
            } else {
                2
            };
            let mut old = 0;
            ensure!(
                unsafe {
                    VirtualProtect(base.wrapping_add(*rva).cast(), *span, protection, &mut old)
                } != 0,
                "Protect codec section"
            );
        }
        Ok(image)
    }
}
type Setup = unsafe extern "system" fn(*mut u8, i32, *const u8, i32);
type Decode = unsafe extern "system" fn(*mut u8, *mut u8, *const u8, isize, *mut u8, isize) -> i32;

pub fn worker_main(config: &Path) -> Result<()> {
    ensure!(
        std::fs::metadata(config)?.len() <= 8192,
        "Codec configuration size"
    );
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(config)?)?;
    let client = Path::new(
        config["client_directory"]
            .as_str()
            .context("Client directory")?,
    );
    let exe = verified_file(
        &client.join("GenshinImpact.exe"),
        "b3001fc110b63e0e0ce24a77386e71d0eefaffaae244ea0c72645a659cb5ea6e",
        512 * 1024 * 1024,
    )?;
    let metadata = verified_file(
        &client.join("GenshinImpact_Data/Managed/Metadata/global-metadata.dat"),
        "4ed10538735861ad87b027d30039e2afb92e334f1b4de4f643e17c9c79d19fd7",
        128 * 1024 * 1024,
    )?;
    let mut initial = Vec::new();
    flate2::read::ZlibDecoder::new(
        metadata
            .get(51434996..51434996 + 237)
            .context("Codec state")?,
    )
    .take(84105)
    .read_to_end(&mut initial)?;
    ensure!(initial.len() == 84104, "Codec state size");
    let image = Image::load(&exe)?;
    drop(exe);
    drop(metadata);
    let setup: Setup = unsafe { std::mem::transmute(image.base.add(0x1cb7a20)) };
    let decode: Decode = unsafe { std::mem::transmute(image.base.add(0x1cb7b50)) };
    let window = vec![0u8; 1024];
    let mut shared = vec![0u64; (8 * (1 << 10) + 32) / 8];
    unsafe {
        setup(shared.as_mut_ptr().cast(), 10, window.as_ptr(), 1024);
    }
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    output.write_all(&[1])?;
    output.flush()?;
    loop {
        let mut header = [0; 8];
        match input.read_exact(&mut header) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e.into()),
        }
        let n = u32le(&header, 0)? as usize;
        let expected = u32le(&header, 4)? as usize;
        ensure!(
            n > 0 && n <= LIMIT && expected > 0 && expected <= LIMIT,
            "Codec request limit"
        );
        let mut body = vec![0; n];
        input.read_exact(&mut body)?;
        let mut out = vec![0; expected];
        // The client's managed reset restores the trained state before each body.
        let mut state = initial.clone();
        let ok = unsafe {
            decode(
                state.as_mut_ptr(),
                shared.as_mut_ptr().cast(),
                body.as_ptr(),
                n as isize,
                out.as_mut_ptr(),
                expected as isize,
            )
        };
        output.write_all(&(if ok != 0 { expected as u32 } else { 0 }).to_le_bytes())?;
        if ok != 0 {
            output.write_all(&out)?;
        }
        output.flush()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn truncated_pe_fields_are_errors() {
        assert!(u16le(&[1], 0).is_err());
        assert!(u32le(&[1, 2, 3], 0).is_err());
        assert!(u64le(&[0; 8], 1).is_err());
    }
    #[test]
    fn unknown_client_is_rejected_before_executable_mapping() {
        let path = std::env::temp_dir().join(format!("vision-codec-{}.bin", std::process::id()));
        std::fs::write(&path, b"untrusted").unwrap();
        let result = verified_file(&path, "not-a-supported-client-hash", 64);
        std::fs::remove_file(&path).unwrap();
        assert!(result.is_err());
    }
}

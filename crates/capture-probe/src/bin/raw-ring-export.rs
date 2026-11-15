//! Freeze a complete startup prefix without modifying an active capture.
use anyhow::{Result, bail, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    ring: PathBuf,
    #[arg(long)]
    output: PathBuf,
}
fn main() -> Result<()> {
    let args = Args::parse();
    let manifest: Value = serde_json::from_reader(File::open(args.ring.join("manifest.json"))?)?;
    ensure!(
        manifest["schema_version"] == 1 && manifest["evicted_segments"] == 0,
        "Export requires the complete, non-evicted startup prefix"
    );
    let sidecar = args.output.with_extension("export.json");
    ensure!(
        !args.output.exists() && !sidecar.exists(),
        "Output already exists"
    );
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args.output)?;
    output.write_all(b"VORAW001")?;
    let mut sources = Vec::new();
    let mut packets = 0u64;
    let mut last = 0u64;
    for (index, segment) in manifest["segments"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing segments"))?
        .iter()
        .enumerate()
    {
        let name = format!("segment-{index:08}.voraw");
        ensure!(segment["file"] == name, "Unexpected segment order/name");
        let path = args.ring.join(&name);
        ensure!(
            !std::fs::symlink_metadata(&path)?.file_type().is_symlink(),
            "Symlink segment"
        );
        let file = File::open(path)?;
        let length = file.metadata()?.len();
        let mut input = file.take(length);
        let mut magic = [0; 8];
        input.read_exact(&mut magic)?;
        ensure!(&magic == b"VORAW001", "Invalid recording header");
        let closed = segment["closed"] == true;
        let mut footer = false;
        let mut tail = 0;
        loop {
            let remaining = input.limit();
            let mut header = [0; 12];
            if let Err(error) = input.read_exact(&mut header) {
                if !closed && error.kind() == std::io::ErrorKind::UnexpectedEof {
                    tail = remaining;
                    break;
                }
                return Err(error.into());
            }
            let elapsed = u64::from_le_bytes(header[..8].try_into()?);
            let size = u32::from_le_bytes(header[8..].try_into()?);
            ensure!(elapsed >= last && size <= 131072, "Invalid packet record");
            if size == 0 {
                ensure!(input.limit() == 0, "Data after footer");
                footer = true;
                last = elapsed;
                break;
            }
            let mut bytes = vec![0; size as usize];
            if let Err(error) = input.read_exact(&mut bytes) {
                if !closed && error.kind() == std::io::ErrorKind::UnexpectedEof {
                    tail = remaining;
                    break;
                }
                return Err(error.into());
            }
            output.write_all(&header)?;
            output.write_all(&bytes)?;
            last = elapsed;
            packets += 1;
        }
        if closed && !footer {
            bail!("Closed segment has no footer");
        }
        sources.push(json!({"file":name,"frozen_bytes":length,"incomplete_tail_bytes":tail,"original_footer":footer}));
        if !closed {
            break;
        }
    }
    ensure!(packets > 0, "No packets exported");
    output.write_all(&last.to_le_bytes())?;
    output.write_all(&0u32.to_le_bytes())?;
    output.flush()?;
    let report = json!({"kind":"diagnostic_live_prefix_export","original_capture_closed":manifest["closed"],
        "footer":"synthetic_export_cut_not_capture_shutdown","cut_elapsed_ms":last,"packets":packets,"sources":sources});
    let mut metadata = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(sidecar)?;
    serde_json::to_writer_pretty(&mut metadata, &report)?;
    println!("Exported {packets} packets through {last} ms; original capture unchanged.");
    Ok(())
}

//! Package a synthetic startup script for the installed engine's isolated oracle.
//! No installed game files are read or changed by this utility.
use anyhow::{Context, Result, ensure};
use krkrz_assets::{
    cx::CxEncryption,
    xp3::{MAGIC, adler32},
};

fn chunk(tag: &[u8; 4], bytes: &[u8]) -> Vec<u8> {
    let mut result = tag.to_vec();
    result.extend((bytes.len() as u64).to_le_bytes());
    result.extend(bytes);
    result
}

fn main() -> Result<()> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    ensure!(
        args.len() == 2,
        "usage: reference_archive SCRIPT OUTPUT.xp3"
    );
    let mut script = std::fs::read(&args[0]).context("read synthetic script")?;
    let hash = adler32(&script);
    CxEncryption::otome_domain()?.apply(hash, 0, &mut script);
    let len = (script.len() as u64).to_le_bytes();
    let mut info = 0x80000000u32.to_le_bytes().to_vec();
    info.extend(len);
    info.extend(len);
    info.extend(11u16.to_le_bytes());
    for unit in "startup.tjs".encode_utf16() {
        info.extend(unit.to_le_bytes());
    }
    // startup.tjs contains eleven UTF-16 code units.
    let mut segment = 0u32.to_le_bytes().to_vec();
    segment.extend(19u64.to_le_bytes());
    segment.extend(len);
    segment.extend(len);
    let mut entry = chunk(b"info", &info);
    entry.extend(chunk(b"segm", &segment));
    entry.extend(chunk(b"adlr", &hash.to_le_bytes()));
    let index = chunk(b"File", &entry);
    let mut archive = MAGIC.to_vec();
    archive.extend((19 + script.len() as u64).to_le_bytes());
    archive.extend(script);
    archive.push(0);
    archive.extend((index.len() as u64).to_le_bytes());
    archive.extend(index);
    std::fs::write(&args[1], archive).context("write synthetic archive")
}

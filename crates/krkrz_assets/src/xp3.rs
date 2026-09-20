use crate::{binary::Reader, cx::CxEncryption};
use anyhow::{Context, Result, bail, ensure};
use flate2::read::ZlibDecoder;
use krkrz_core::{Limits, storage_name};
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
};

pub const MAGIC: &[u8; 11] = b"XP3\r\n \n\x1a\x8b\x67\x01";
#[derive(Clone, Debug, Serialize)]
pub struct Segment {
    pub compressed: bool,
    pub offset: u64,
    pub size: u64,
    pub packed_size: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    pub name: String,
    pub encrypted: bool,
    pub size: u64,
    pub packed_size: u64,
    pub hash: u32,
    pub segments: Vec<Segment>,
}
/// Segment cache has a strict byte bound; reads never cache a whole archive.
type CachedSegment = ((u64, u64, u64), Arc<Vec<u8>>);
pub struct Archive {
    pub path: PathBuf,
    pub entries: BTreeMap<String, Entry>,
    file: File,
    length: u64,
    limits: Limits,
    cache: VecDeque<CachedSegment>,
    cache_bytes: usize,
}
fn file_u64(file: &mut File) -> Result<u64> {
    let mut b = [0; 8];
    file.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}
fn range(offset: u64, size: u64, total: u64) -> Result<()> {
    ensure!(
        offset.checked_add(size).is_some_and(|end| end <= total),
        "range {offset:#x}+{size:#x} exceeds {total:#x}"
    );
    Ok(())
}
fn read_at(file: &mut File, offset: u64, size: u64, total: u64) -> Result<Vec<u8>> {
    range(offset, size, total)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0; usize::try_from(size)?];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}
pub(crate) fn inflate(bytes: &[u8], size: u64, limit: u64) -> Result<Vec<u8>> {
    ensure!(
        size <= limit,
        "decompressed data exceeds limit ({size} > {limit})"
    );
    let mut decoder = ZlibDecoder::new(bytes);
    let mut result = Vec::new();
    (&mut decoder).take(size + 1).read_to_end(&mut result)?;
    ensure!(
        result.len() as u64 == size,
        "decompressed size mismatch: expected {size}, got {}",
        result.len()
    );
    ensure!(
        decoder.total_in() == bytes.len() as u64,
        "trailing compressed data"
    );
    Ok(result)
}
impl Archive {
    /// Release decoded segments without closing the mounted archive.
    pub fn clear_cache(&mut self) {
        self.cache.clear();
        self.cache_bytes = 0;
    }

    pub fn open(path: impl AsRef<Path>, limits: Limits) -> Result<Self> {
        let path = path.as_ref();
        let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
        let length = file.metadata()?.len();
        let magic = read_at(&mut file, 0, 11, length)?;
        ensure!(magic == MAGIC, "{}: not an XP3 archive", path.display());
        let mut next = file_u64(&mut file)?;
        let mut entries = BTreeMap::new();
        let mut seen = HashSet::new();
        let mut total_index = 0u64;
        loop {
            ensure!(
                seen.len() < 1024 && seen.insert(next),
                "cyclic or excessive XP3 index chain"
            );
            range(next, 9, length)?;
            file.seek(SeekFrom::Start(next))?;
            let mut flags = [0];
            file.read_exact(&mut flags)?;
            let flags = flags[0];
            ensure!(flags & !0x81 == 0, "unsupported XP3 index flags {flags:#x}");
            let packed = file_u64(&mut file)?;
            let unpacked = if flags & 1 != 0 {
                file_u64(&mut file)?
            } else {
                packed
            };
            total_index = total_index
                .checked_add(unpacked)
                .context("index size overflow")?;
            ensure!(
                total_index <= limits.index_bytes && packed <= limits.index_bytes,
                "XP3 index exceeds limit"
            );
            let start = file.stream_position()?;
            let bytes = read_at(&mut file, start, packed, length)?;
            let index = if flags & 1 != 0 {
                inflate(&bytes, unpacked, limits.index_bytes)?
            } else {
                bytes
            };
            let mut index = Reader::new(&index);
            while !index.done() {
                let (tag, data) = index.chunk()?;
                if &tag != b"File" {
                    continue;
                }
                let entry = parse_entry(data, length, limits)
                    .with_context(|| format!("{} index {next:#x}", path.display()))?;
                ensure!(entries.len() < limits.entries, "too many XP3 entries");
                ensure!(
                    !entries.contains_key(&entry.name),
                    "duplicate XP3 entry {}",
                    entry.name
                );
                entries.insert(entry.name.clone(), entry);
            }
            if flags & 0x80 == 0 {
                break;
            }
            next = file_u64(&mut file)?;
        }
        Ok(Self {
            path: path.to_owned(),
            entries,
            file,
            length,
            limits,
            cache: VecDeque::new(),
            cache_bytes: 0,
        })
    }
    pub fn read(&mut self, name: &str, cipher: Option<&CxEncryption>) -> Result<Vec<u8>> {
        let name = storage_name(name)?;
        let size = self
            .entries
            .get(&name)
            .with_context(|| format!("storage not found: {name}"))?
            .size;
        self.read_range(&name, 0, size, cipher)
    }
    pub fn read_range(
        &mut self,
        name: &str,
        offset: u64,
        size: u64,
        cipher: Option<&CxEncryption>,
    ) -> Result<Vec<u8>> {
        let name = storage_name(name)?;
        let entry = self
            .entries
            .get(&name)
            .with_context(|| format!("{}: missing {name}", self.path.display()))?
            .clone();
        ensure!(
            entry.segments.iter().map(|s| s.size).sum::<u64>() == entry.size
                && entry.segments.iter().map(|s| s.packed_size).sum::<u64>() == entry.packed_size,
            "segment totals disagree with file sizes: {name}"
        );
        range(offset, size, entry.size)?;
        if entry.encrypted && cipher.is_none() {
            bail!(
                "{}: encrypted entry requires an explicit Cx profile",
                entry.name
            );
        }
        let mut result = Vec::with_capacity(usize::try_from(size)?);
        let end = offset + size;
        let mut base = 0;
        for segment in &entry.segments {
            let segment_end = base + segment.size;
            if base < end && segment_end > offset {
                let start = offset.saturating_sub(base);
                let stop = (end - base).min(segment.size);
                if segment.compressed {
                    let bytes = self.segment(segment)?;
                    result.extend_from_slice(&bytes[start as usize..stop as usize]);
                } else {
                    result.extend(read_at(
                        &mut self.file,
                        segment.offset + start,
                        stop - start,
                        self.length,
                    )?);
                }
            }
            base = segment_end;
        }
        ensure!(
            result.len() as u64 == size,
            "incomplete segmented read: {name}"
        );
        if entry.encrypted {
            cipher
                .context("missing Cx profile")?
                .apply(entry.hash, offset, &mut result);
        }
        Ok(result)
    }
    fn segment(&mut self, segment: &Segment) -> Result<Arc<Vec<u8>>> {
        let key = (segment.offset, segment.size, segment.packed_size);
        if let Some(index) = self.cache.iter().position(|(k, _)| *k == key) {
            let item = self
                .cache
                .remove(index)
                .context("cache index disappeared")?;
            let bytes = item.1.clone();
            self.cache.push_back(item);
            return Ok(bytes);
        }
        let packed = read_at(
            &mut self.file,
            segment.offset,
            segment.packed_size,
            self.length,
        )?;
        let bytes = Arc::new(inflate(&packed, segment.size, self.limits.file_bytes)?);
        if bytes.len() <= self.limits.cache_bytes {
            while self.cache_bytes + bytes.len() > self.limits.cache_bytes {
                if let Some((_, old)) = self.cache.pop_front() {
                    self.cache_bytes -= old.len();
                }
            }
            self.cache_bytes += bytes.len();
            self.cache.push_back((key, bytes.clone()));
        }
        Ok(bytes)
    }
    pub fn verify(&mut self, name: &str, cipher: Option<&CxEncryption>) -> Result<()> {
        let bytes = self.read(name, cipher)?;
        let entry = &self.entries[&storage_name(name)?];
        ensure!(
            adler32(&bytes) == entry.hash,
            "{}: Adler-32 mismatch (wrong profile or damaged data)",
            entry.name
        );
        Ok(())
    }
}
fn parse_entry(data: &[u8], length: u64, limits: Limits) -> Result<Entry> {
    let mut r = Reader::new(data);
    let (mut info, mut segments, mut hash) = (None, None, None);
    while !r.done() {
        let (tag, data) = r.chunk()?;
        let mut c = Reader::new(data);
        match &tag {
            b"info" => {
                ensure!(info.is_none(), "duplicate info chunk");
                let flags = c.u32()?;
                ensure!(
                    flags & !0x80000000 == 0,
                    "unsupported XP3 file flags {flags:#x}"
                );
                let size = c.u64()?;
                let packed = c.u64()?;
                ensure!(
                    size <= limits.file_bytes && packed <= limits.file_bytes,
                    "XP3 file exceeds limit"
                );
                let count = c.u16()? as usize;
                let units = c
                    .take(count * 2)?
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect::<Vec<_>>();
                let name = storage_name(&String::from_utf16(&units)?)?;
                info = Some((name, flags != 0, size, packed));
            }
            b"segm" => {
                ensure!(
                    segments.is_none() && data.len().is_multiple_of(28),
                    "invalid segment table"
                );
                let mut list = Vec::new();
                while !c.done() {
                    let flags = c.u32()?;
                    ensure!(flags <= 1, "unsupported segment flags {flags:#x}");
                    let s = Segment {
                        compressed: flags != 0,
                        offset: c.u64()?,
                        size: c.u64()?,
                        packed_size: c.u64()?,
                    };
                    range(s.offset, s.packed_size, length)?;
                    ensure!(
                        s.size <= limits.file_bytes && s.packed_size <= limits.file_bytes,
                        "segment exceeds limit"
                    );
                    ensure!(
                        s.compressed || s.size == s.packed_size,
                        "raw segment size mismatch"
                    );
                    list.push(s);
                }
                segments = Some(list);
            }
            b"adlr" => {
                ensure!(hash.is_none() && data.len() == 4, "invalid checksum chunk");
                hash = Some(c.u32()?);
            }
            _ => (),
        }
    }
    let (name, encrypted, size, packed_size) = info.context("missing info chunk")?;
    let segments: Vec<Segment> = segments.context("missing segment table")?;
    let sum = |packed: bool| -> Result<u64> {
        segments.iter().try_fold(0u64, |n, s| {
            n.checked_add(if packed { s.packed_size } else { s.size })
                .context("segment size overflow")
        })
    };
    // Some protected archives include a deliberately inconsistent notice entry.
    // Catalog it like upstream; reject inconsistent sizes when it is read.
    ensure!(
        sum(false)? <= limits.file_bytes && sum(true)? <= limits.file_bytes,
        "segment totals exceed limit"
    );
    Ok(Entry {
        name,
        encrypted,
        size,
        packed_size,
        hash: hash.context("missing checksum")?,
        segments,
    })
}
pub fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in bytes.chunks(5552) {
        for v in chunk {
            a += u32::from(*v);
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

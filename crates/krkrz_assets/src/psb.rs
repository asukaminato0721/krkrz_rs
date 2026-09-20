//! PSB v2 structured data. Format reference: GARbro ArcPSB.cs; see notices.
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet, VecDeque};
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(untagged)]
pub enum Value {
    Null,
    Bool(bool),
    Integer(i64),
    Real(f64),
    String(String),
    List(Vec<Value>),
    Object(BTreeMap<String, Value>),
    Resource {
        resource_index: u32,
        offset: usize,
        length: usize,
    },
}
#[derive(Debug, Serialize)]
pub struct Document {
    pub version: u16,
    pub root: Value,
}
struct Parser<'a> {
    data: &'a [u8],
    names: BTreeMap<u32, String>,
    strings: Vec<u32>,
    strings_data: usize,
    chunk_offsets: Vec<u32>,
    chunk_lengths: Vec<u32>,
    chunk_data: usize,
    remaining: usize,
    output_bytes: usize,
    active: HashSet<usize>,
}
fn slice(data: &[u8], offset: usize, length: usize) -> Result<&[u8]> {
    data.get(offset..offset.checked_add(length).context("PSB offset overflow")?)
        .context("PSB offset outside data")
}
fn uint(data: &[u8], offset: usize, width: usize) -> Result<u64> {
    ensure!(width <= 8, "invalid PSB integer width");
    let mut bytes = [0; 8];
    bytes[..width].copy_from_slice(slice(data, offset, width)?);
    Ok(u64::from_le_bytes(bytes))
}
fn array(data: &[u8], offset: usize) -> Result<(Vec<u32>, usize)> {
    let width = usize::from(*data.get(offset).context("missing PSB array")?)
        .checked_sub(12)
        .context("invalid PSB array type")?;
    ensure!((1..=4).contains(&width), "invalid PSB array count width");
    let count = usize::try_from(uint(data, offset + 1, width)?)?;
    ensure!(count <= 1_000_000, "PSB array too large");
    let elem = usize::from(
        *data
            .get(offset + 1 + width)
            .context("missing PSB element type")?,
    )
    .checked_sub(12)
    .context("invalid PSB array element type")?;
    ensure!((1..=4).contains(&elem), "invalid PSB array element width");
    let start = offset + 2 + width;
    let bytes = slice(data, start, count * elem)?;
    let mut result = Vec::with_capacity(count);
    for b in bytes.chunks_exact(elem) {
        result.push(uint(b, 0, elem)? as u32);
    }
    Ok((result, start + bytes.len()))
}
impl Document {
    pub fn parse(data: &[u8]) -> Result<Self> {
        ensure!(slice(data, 0, 4)? == b"PSB\0", "not a PSB container");
        let version = uint(data, 4, 2)? as u16;
        ensure!(version == 2, "unsupported PSB version {version}");
        ensure!(
            uint(data, 6, 2)? == 0,
            "encrypted PSB headers are unsupported"
        );
        let offsets = (8..40)
            .step_by(4)
            .map(|i| uint(data, i, 4).map(|n| n as usize))
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            offsets.iter().all(|&o| o >= 40 && o <= data.len()),
            "invalid PSB header offsets"
        );
        let [
            _,
            names,
            strings,
            strings_data,
            chunk_offsets,
            chunk_lengths,
            chunk_data,
            root,
        ]: [usize; 8] = offsets
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid PSB header"))?;
        let metadata = &data[..chunk_data];
        let (bases, next) = array(metadata, names)?;
        let (parents, _) = array(metadata, next)?;
        ensure!(
            bases.len() == parents.len(),
            "PSB name table lengths disagree"
        );
        let mut pending = VecDeque::new();
        if !bases.is_empty() {
            pending.push_back((0usize, Vec::new()));
        }
        let mut seen = HashSet::new();
        let mut map = BTreeMap::new();
        let mut name_bytes = 64usize << 20;
        while let Some((node, prefix)) = pending.pop_front() {
            ensure!(
                prefix.len() <= 4096 && seen.insert(node),
                "cyclic or excessive PSB name trie"
            );
            let base = *bases.get(node).context("PSB name trie node out of range")? as usize;
            for c in 0..256usize {
                let index = base.checked_add(c).context("PSB trie overflow")?;
                if parents.get(index).copied() != Some(node as u32) {
                    continue;
                }
                if c == 0 {
                    let id = *bases.get(index).context("missing PSB name ID")?;
                    ensure!(
                        map.insert(id, String::from_utf8(prefix.clone())?).is_none(),
                        "duplicate PSB name ID"
                    );
                } else {
                    name_bytes = name_bytes
                        .checked_sub(prefix.len() + 1)
                        .context("PSB name trie allocation budget exceeded")?;
                    let mut name = prefix.clone();
                    name.push(c as u8);
                    pending.push_back((index, name));
                }
            }
        }
        let mut parser = Parser {
            data: metadata,
            names: map,
            strings: array(metadata, strings)?.0,
            strings_data,
            chunk_offsets: array(metadata, chunk_offsets)?.0,
            chunk_lengths: array(metadata, chunk_lengths)?.0,
            chunk_data,
            remaining: 1_000_000,
            output_bytes: 256 << 20,
            active: HashSet::new(),
        };
        ensure!(
            parser.chunk_offsets.len() == parser.chunk_lengths.len(),
            "PSB chunk table lengths disagree"
        );
        for (&o, &n) in parser.chunk_offsets.iter().zip(&parser.chunk_lengths) {
            slice(data, chunk_data + o as usize, n as usize)?;
        }
        let root = parser.value(root, 0)?;
        Ok(Self { version, root })
    }
}
impl Parser<'_> {
    fn allocate(&mut self, bytes: usize) -> Result<()> {
        self.output_bytes = self
            .output_bytes
            .checked_sub(bytes)
            .context("PSB output allocation budget exceeded")?;
        Ok(())
    }
    fn value(&mut self, offset: usize, depth: usize) -> Result<Value> {
        ensure!(
            depth < 128 && self.remaining > 0,
            "PSB object budget exceeded"
        );
        self.remaining -= 1;
        ensure!(self.active.insert(offset), "cyclic PSB value offset");
        let tag = *self
            .data
            .get(offset)
            .context("PSB value outside metadata")?;
        let result = match tag {
            1 => Value::Null,
            2 => Value::Bool(true),
            3 => Value::Bool(false),
            4 => Value::Integer(0),
            5..=12 => {
                let width = (tag - 4) as usize;
                let n = uint(self.data, offset + 1, width)?;
                // PSB integer payloads are signed, with the shortest sign-preserving width.
                let shift = (8 - width) * 8;
                Value::Integer(((n << shift) as i64) >> shift)
            }
            0x15..=0x18 => {
                let id = uint(self.data, offset + 1, (tag - 0x14) as usize)? as usize;
                let pos = self.strings_data
                    + *self
                        .strings
                        .get(id)
                        .context("PSB string ID outside table")? as usize;
                let rest = self
                    .data
                    .get(pos..)
                    .context("PSB string outside metadata")?;
                let end = rest
                    .iter()
                    .position(|b| *b == 0)
                    .context("unterminated PSB string")?;
                self.allocate(end)?;
                Value::String(std::str::from_utf8(&rest[..end])?.to_owned())
            }
            0x19..=0x1c => {
                let id = uint(self.data, offset + 1, (tag - 0x18) as usize)? as usize;
                let o = *self
                    .chunk_offsets
                    .get(id)
                    .context("PSB resource ID outside table")? as usize;
                let n = *self
                    .chunk_lengths
                    .get(id)
                    .context("PSB resource length missing")? as usize;
                Value::Resource {
                    resource_index: id as u32,
                    offset: self.chunk_data + o,
                    length: n,
                }
            }
            0x1d => Value::Real(0.0),
            0x1e => Value::Real(f32::from_bits(uint(self.data, offset + 1, 4)? as u32) as f64),
            0x1f => Value::Real(f64::from_bits(uint(self.data, offset + 1, 8)?)),
            0x20 => {
                let (items, base) = array(self.data, offset + 1)?;
                self.allocate(
                    items
                        .len()
                        .checked_mul(std::mem::size_of::<Value>())
                        .context("PSB list allocation overflow")?,
                )?;
                let mut result = Vec::with_capacity(items.len());
                for item in items {
                    result.push(self.value(base + item as usize, depth + 1)?);
                }
                Value::List(result)
            }
            0x21 => {
                let (keys, next) = array(self.data, offset + 1)?;
                let (values, base) = array(self.data, next)?;
                ensure!(
                    keys.len() == values.len(),
                    "PSB object keys and values disagree"
                );
                self.allocate(
                    keys.len()
                        .checked_mul(128)
                        .context("PSB object allocation overflow")?,
                )?;
                let mut result = BTreeMap::new();
                for (key, value) in keys.into_iter().zip(values) {
                    let name = self
                        .names
                        .get(&key)
                        .context("PSB object key missing from name table")?
                        .clone();
                    self.allocate(name.len())?;
                    let value = self.value(base + value as usize, depth + 1)?;
                    ensure!(
                        result.insert(name, value).is_none(),
                        "duplicate PSB object key"
                    );
                }
                Value::Object(result)
            }
            _ => bail!("unsupported PSB type {tag:#x} at {offset:#x}"),
        };
        self.active.remove(&offset);
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn minimal(value: &[u8]) -> Vec<u8> {
        let mut d = b"PSB\0\x02\0\0\0".to_vec();
        // Empty trie, string and resource tables, followed by one root value.
        for n in [40u32, 40, 46, 49, 49, 52, (55 + value.len()) as u32, 55] {
            d.extend(n.to_le_bytes());
        }
        for _ in 0..5 {
            d.extend([13, 0, 13]);
        }
        d.extend(value);
        d
    }
    #[test]
    fn scalars_and_lists() {
        assert_eq!(
            Document::parse(&minimal(&[5, 255])).unwrap().root,
            Value::Integer(-1)
        );
        let list = [32, 13, 2, 13, 0, 1, 2, 3];
        assert_eq!(
            Document::parse(&minimal(&list)).unwrap().root,
            Value::List(vec![Value::Bool(true), Value::Bool(false)])
        );
    }
    #[test]
    fn truncated_inputs_never_panic() {
        let d = minimal(&[4]);
        for n in 0..d.len() {
            assert!(Document::parse(&d[..n]).is_err());
        }
    }
}

//! Cx algorithm adapted from GARbro KiriKiriCx.cs (see THIRD_PARTY_NOTICES.md).
//! The generated program is interpreted Rust data, never executable machine code.
use anyhow::{Result, ensure};
use serde::Deserialize;

/// Named data profiles are independent of executable names and script behavior.
pub const BUILTIN_PROFILES: &[(&str, &str)] =
    &[("otome-domain", include_str!("../data/otome_domain_cx.json"))];

#[derive(Deserialize)]
struct Profile {
    m_mask: u32,
    m_offset: u32,
    #[serde(rename = "PrologOrder")]
    prolog: [u8; 3],
    #[serde(rename = "OddBranchOrder")]
    odd: [u8; 6],
    #[serde(rename = "EvenBranchOrder")]
    even: [u8; 8],
    #[serde(rename = "ControlBlock")]
    control: Vec<u32>,
}
#[derive(Clone, Debug)]
enum Op {
    Arg,
    Imm(u32),
    Lookup,
    Push,
    Pop,
    Save,
    Odd(u8),
    Even(u8),
    Xor(u32),
    Add(u32),
    Sub(u32),
}
struct Generator<'a> {
    seed: u32,
    length: usize,
    code: Vec<Op>,
    profile: &'a Profile,
}
impl Generator<'_> {
    fn random(&mut self) -> u32 {
        let old = self.seed;
        self.seed = old.wrapping_mul(1103515245).wrapping_add(12345);
        self.seed ^ (old << 16) ^ (old >> 16)
    }
    fn space(&mut self, bytes: usize) -> Option<()> {
        if self.length + bytes > 128 {
            return None;
        }
        self.length += bytes;
        Some(())
    }
    fn emit(&mut self, op: Op, bytes: usize) -> Option<()> {
        self.space(bytes)?;
        self.code.push(op);
        Some(())
    }
    fn prolog(&mut self) -> Option<()> {
        match self.profile.prolog[(self.random() % 3) as usize] {
            0 => {
                self.space(1)?;
                let n = self.random();
                self.emit(Op::Imm(n), 4)
            }
            1 => self.emit(Op::Arg, 2),
            2 => {
                self.space(7)?;
                let n = self.random() & 0x3ff;
                self.emit(Op::Imm(n), 4)?;
                self.emit(Op::Lookup, 0)
            }
            _ => None,
        }
    }
    fn body(&mut self, stage: u8, odd: bool) -> Option<()> {
        if stage == 1 {
            return self.prolog();
        }
        if odd {
            self.emit(Op::Push, 1)?;
        }
        let branch = self.random() & 1 != 0;
        self.body(stage - 1, branch)?;
        if odd {
            self.emit(Op::Save, 2)?;
            let branch = self.random() & 1 != 0;
            self.body(stage - 1, branch)?;
            let op = self.profile.odd[(self.random() % 6) as usize];
            self.emit(Op::Odd(op), [9, 9, 2, 4, 3, 2][op as usize])?;
            self.emit(Op::Pop, 1)
        } else {
            let op = self.profile.even[(self.random() & 7) as usize];
            match op {
                0..=5 => {
                    // Match incremental emission: a failed stage preserves PRNG state.
                    for n in match op {
                        0 | 2 => &[2][..],
                        1 | 3 => &[1][..],
                        4 => &[5, 1, 4, 3][..],
                        5 => &[1, 2, 2, 4, 1, 4, 2, 2, 2, 1][..],
                        _ => unreachable!(),
                    } {
                        self.space(*n)?;
                    }
                    self.code.push(Op::Even(op));
                    Some(())
                }
                6 => {
                    self.space(1)?;
                    let n = self.random();
                    self.emit(Op::Xor(n), 4)
                }
                7 => {
                    let add = self.random() & 1 != 0;
                    self.space(1)?;
                    let n = self.random();
                    self.emit(if add { Op::Add(n) } else { Op::Sub(n) }, 4)
                }
                _ => None,
            }
        }
    }
}
pub struct CxEncryption {
    profile: Profile,
    programs: Vec<Vec<Op>>,
}
impl CxEncryption {
    pub fn otome_domain() -> Result<Self> {
        Self::from_json(include_str!("../data/otome_domain_cx.json"))
    }
    pub fn from_json(json: &str) -> Result<Self> {
        let profile: Profile = serde_json::from_str(json)?;
        ensure!(
            profile.control.len() == 1024,
            "Cx requires 1024 control words"
        );
        for (order, size) in [
            (&profile.prolog[..], 3),
            (&profile.odd[..], 6),
            (&profile.even[..], 8),
        ] {
            let mut sorted = order.to_vec();
            sorted.sort();
            ensure!(
                sorted == (0..size).collect::<Vec<u8>>(),
                "invalid Cx branch permutation"
            );
        }
        let mut programs = Vec::with_capacity(128);
        for seed in 0..128 {
            let mut g = Generator {
                seed,
                length: 0,
                code: vec![],
                profile: &profile,
            };
            let mut ok = false;
            for stage in (1..=5).rev() {
                g.length = 0;
                g.code.clear();
                if g.space(9)
                    .and_then(|_| g.body(stage, true))
                    .and_then(|_| g.space(6))
                    .is_some()
                {
                    ok = true;
                    break;
                }
            }
            ensure!(ok, "Cx program too large");
            programs.push(g.code);
        }
        Ok(Self { profile, programs })
    }
    fn execute(&self, seed: usize, arg: u32) -> u32 {
        let (mut a, mut b) = (0u32, 0u32);
        let mut stack = Vec::new();
        for op in &self.programs[seed] {
            match *op {
                Op::Arg => a = arg,
                Op::Imm(n) => a = n,
                Op::Lookup => a = !self.profile.control[a as usize],
                Op::Push => stack.push(b),
                Op::Pop => b = stack.pop().expect("generated balanced Cx stack"),
                Op::Save => b = a,
                Op::Odd(n) => {
                    a = match n {
                        0 => a >> (b & 15),
                        1 => a << (b & 15),
                        2 => a.wrapping_add(b),
                        3 => b.wrapping_sub(a),
                        4 => a.wrapping_mul(b),
                        5 => a.wrapping_sub(b),
                        _ => unreachable!(),
                    }
                }
                Op::Even(n) => {
                    a = match n {
                        0 => !a,
                        1 => a.wrapping_sub(1),
                        2 => a.wrapping_neg(),
                        3 => a.wrapping_add(1),
                        4 => !self.profile.control[(a & 1023) as usize],
                        5 => ((a & 0xaaaaaaaa) >> 1) | ((a & 0x55555555) << 1),
                        _ => unreachable!(),
                    }
                }
                Op::Xor(n) => a ^= n,
                Op::Add(n) => a = a.wrapping_add(n),
                Op::Sub(n) => a = a.wrapping_sub(n),
            }
        }
        a
    }
    pub fn keys(&self, hash: u32) -> (u32, u32) {
        let seed = (hash & 127) as usize;
        let hash = hash >> 7;
        (self.execute(seed, hash), self.execute(seed, !hash))
    }
    pub fn apply(&self, hash: u32, offset: u64, bytes: &mut [u8]) {
        let boundary = u64::from((hash & self.profile.m_mask).wrapping_add(self.profile.m_offset));
        let split = boundary.saturating_sub(offset).min(bytes.len() as u64) as usize;
        self.decode(hash, offset, &mut bytes[..split]);
        self.decode(
            (hash >> 16) ^ hash,
            offset + split as u64,
            &mut bytes[split..],
        );
    }
    fn decode(&self, hash: u32, offset: u64, bytes: &mut [u8]) {
        if bytes.is_empty() {
            return;
        }
        let (a, b) = self.keys(hash);
        let k1 = b >> 16;
        let mut k2 = b & 65535;
        if k1 == k2 {
            k2 += 1;
        }
        let k3 = if a as u8 == 0 { 1 } else { a as u8 };
        for (key, mask) in [(k1, (a >> 8) as u8), (k2, (a >> 16) as u8)] {
            if let Some(index) = u64::from(key)
                .checked_sub(offset)
                .filter(|n| *n < bytes.len() as u64)
            {
                bytes[index as usize] ^= mask;
            }
        }
        for b in bytes {
            *b ^= k3;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chunk_boundaries_and_involution() {
        let cx = CxEncryption::otome_domain().unwrap();
        for hash in [0, 1, 127, 128, 0x12345678, u32::MAX] {
            let mut whole = (0..70000).map(|n| n as u8).collect::<Vec<_>>();
            let plain = whole.clone();
            cx.apply(hash, 0, &mut whole);
            let mut chunks = plain.clone();
            for (i, c) in chunks.chunks_mut(137).enumerate() {
                cx.apply(hash, (i * 137) as u64, c);
            }
            assert_eq!(whole, chunks);
            cx.apply(hash, 0, &mut whole);
            assert_eq!(whole, plain);
        }
    }
}

use anyhow::{Result, ensure};

pub struct Reader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}
impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| anyhow::anyhow!("offset overflow"))?;
        ensure!(
            end <= self.data.len(),
            "truncated input at {:#x}: need {n} bytes",
            self.pos
        );
        let out = &self.data[self.pos..end];
        self.pos = end;
        Ok(out)
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into()?))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into()?))
    }
    pub fn done(&self) -> bool {
        self.pos == self.data.len()
    }
    pub fn chunk(&mut self) -> Result<([u8; 4], &'a [u8])> {
        let tag = self.take(4)?.try_into()?;
        let len = usize::try_from(self.u64()?)?;
        Ok((tag, self.take(len)?))
    }
}

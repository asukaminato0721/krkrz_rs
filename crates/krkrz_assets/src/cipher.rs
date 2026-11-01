//! XP3 ciphers apply to entry bytes after segment decompression.
use crate::cx::CxEncryption;

pub enum Cipher {
    Cx(CxEncryption),
    Applique,
    PoringSoft,
}

impl From<CxEncryption> for Cipher {
    fn from(cipher: CxEncryption) -> Self {
        Self::Cx(cipher)
    }
}

impl Cipher {
    pub fn apply(&self, hash: u32, offset: u64, bytes: &mut [u8]) {
        match self {
            Self::Cx(cipher) => cipher.apply(hash, offset, bytes),
            // GARbro CryptAlgorithms.cs: AppliqueCrypt preserves the first
            // five entry bytes and XORs the rest with bits 12..19 of the hash.
            Self::Applique => {
                let skip = 5u64.saturating_sub(offset).min(bytes.len() as u64) as usize;
                let key = (hash >> 12) as u8;
                for byte in &mut bytes[skip..] {
                    *byte ^= key;
                }
            }
            // GARbro CryptAlgorithms.cs: PoringSoftCrypt XORs every byte
            // with the low byte of the complemented, incremented entry hash.
            Self::PoringSoft => {
                let key = !hash.wrapping_add(1) as u8;
                for byte in bytes {
                    *byte ^= key;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Cipher;

    #[test]
    fn poring_soft_uses_the_entry_hash_at_every_offset() {
        let plain = [0, 1, 2, 3, 4, 5, 6, 7];
        // Adler-32(plain) = 0x005c001d, so the XOR byte is 0xe1.
        let encoded = [0xe1, 0xe0, 0xe3, 0xe2, 0xe5, 0xe4, 0xe7, 0xe6];
        for (start, end) in [(0, 8), (0, 3), (3, 7), (5, 8), (8, 8)] {
            let mut bytes = plain[start..end].to_vec();
            Cipher::PoringSoft.apply(0x005c001d, start as u64, &mut bytes);
            assert_eq!(bytes, encoded[start..end]);
            Cipher::PoringSoft.apply(0x005c001d, start as u64, &mut bytes);
            assert_eq!(bytes, plain[start..end]);
        }
        let mut bytes = [0, 0xff];
        Cipher::PoringSoft.apply(u32::MAX, u64::MAX, &mut bytes);
        assert_eq!(bytes, [0xff, 0]);
    }

    #[test]
    fn applique_preserves_header_and_uses_absolute_entry_offsets() {
        let plain = [0, 1, 2, 3, 4, 5, 6, 7];
        // Adler-32(plain) = 0x005c001d, so the XOR byte is 0xc0.
        let encoded = [0, 1, 2, 3, 4, 0xc5, 0xc6, 0xc7];
        for (start, end) in [(0, 8), (0, 3), (3, 7), (5, 8), (6, 8), (8, 8)] {
            let mut bytes = plain[start..end].to_vec();
            Cipher::Applique.apply(0x005c001d, start as u64, &mut bytes);
            assert_eq!(bytes, encoded[start..end]);
            Cipher::Applique.apply(0x005c001d, start as u64, &mut bytes);
            assert_eq!(bytes, plain[start..end]);
        }
        let mut bytes = plain;
        Cipher::Applique.apply(0, u64::MAX, &mut bytes);
        assert_eq!(bytes, plain);
    }
}

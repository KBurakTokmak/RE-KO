//! JvCryption stream cipher used by Knight Online v1453.

/// Hardcoded client private key recovered from KnightOnline.exe at 0x004624E0.
pub const KO_PRIVATE_KEY: u64 = 0x1257091582190465;

/// Server public key exchanged during opcode 0xF2.
pub const KO_PUBLIC_KEY: u64 = 0x1234567890123456;

/// JvCryption symmetric stream cipher.
#[derive(Debug, Clone)]
pub struct JvCrypt {
    tkey: [u8; 8],
    pub enabled: bool,
}

impl Default for JvCrypt {
    fn default() -> Self {
        Self::new(KO_PUBLIC_KEY)
    }
}

impl JvCrypt {
    pub fn new(public_key: u64) -> Self {
        let tkey = (public_key ^ KO_PRIVATE_KEY).to_le_bytes();
        Self {
            tkey,
            enabled: true,
        }
    }

    /// Transforms data in place (symmetric XOR stream cipher).
    pub fn transform(&self, data: &mut [u8]) {
        let n = data.len();
        let lkey = ((n * 157) & 0xFF) as u8;
        let mut rkey: u32 = 2157;
        for (i, byte) in data.iter_mut().enumerate() {
            let rsk = ((rkey >> 8) & 0xFF) as u8;
            *byte = ((*byte ^ rsk) ^ self.tkey[i % 8]) ^ lkey;
            rkey = rkey.wrapping_mul(2171);
        }
    }

    /// Decrypts a client packet: transforms in place, skips 4-byte header, strips 4-byte CRC.
    pub fn decrypt_client_packet<'a>(&self, data: &'a mut [u8]) -> Option<&'a [u8]> {
        if data.len() < 8 {
            return None;
        }
        self.transform(data);
        Some(&data[4..data.len() - 4])
    }

    /// Encrypts a server response: prepends 5-byte header [0xFC, 0x1E, 0, 0, 0], then transforms in place.
    pub fn encrypt_server_packet(&self, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(payload.len() + 5);
        out.extend_from_slice(&[0xFC, 0x1E, 0x00, 0x00, 0x00]);
        out.extend_from_slice(payload);
        self.transform(&mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jvcrypt_roundtrip() {
        let crypt = JvCrypt::default();
        let mut data = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let orig = data;
        crypt.transform(&mut data);
        assert_ne!(data, orig);
        crypt.transform(&mut data);
        assert_eq!(data, orig);
    }
}

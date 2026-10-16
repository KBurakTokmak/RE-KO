//! Incremental KO framing for TCP streams.

#[derive(Default)]
pub(crate) struct Decoder {
    bytes: Vec<u8>,
}

impl Decoder {
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    pub(crate) fn pending(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn next(&mut self) -> Result<Option<Vec<u8>>, &'static str> {
        if self.bytes.len() < 2 {
            return Ok(None);
        }
        if self.bytes[..2] != [0xAA, 0x55] {
            return Err("invalid KO frame header");
        }
        if self.bytes.len() < 4 {
            return Ok(None);
        }
        let len = u16::from_le_bytes([self.bytes[2], self.bytes[3]]) as usize;
        if len == 0 {
            return Err("empty KO frame payload");
        }
        let end = len + 6;
        if self.bytes.len() < end {
            return Ok(None);
        }
        if self.bytes[end - 2..end] != [0x55, 0xAA] {
            return Err("invalid KO frame footer");
        }
        let payload = self.bytes[4..end - 2].to_vec();
        self.bytes.drain(..end);
        Ok(Some(payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_packet;

    #[test]
    fn every_split_and_coalesced_frames() {
        let first = frame_packet(&[0x2B, 0xAD, 0x05]);
        let second = frame_packet(&[0x0C]);
        for split in 1..first.len() {
            let mut decoder = Decoder::default();
            decoder.push(&first[..split]);
            assert_eq!(decoder.next().unwrap(), None);
            decoder.push(&first[split..]);
            decoder.push(&second);
            assert_eq!(decoder.next().unwrap(), Some(vec![0x2B, 0xAD, 0x05]));
            assert_eq!(decoder.next().unwrap(), Some(vec![0x0C]));
            assert_eq!(decoder.next().unwrap(), None);
            assert_eq!(decoder.pending(), 0);
        }
    }

    #[test]
    fn malformed_frames_are_reported() {
        for bytes in [
            vec![0, 0],
            vec![0xAA, 0x55, 0, 0],
            vec![0xAA, 0x55, 1, 0, 2, 0, 0],
        ] {
            let mut decoder = Decoder::default();
            decoder.push(&bytes);
            assert!(decoder.next().is_err());
        }
    }

    #[test]
    fn maximum_payload_can_arrive_in_small_reads() {
        let payload = vec![0x02; u16::MAX as usize];
        let frame = frame_packet(&payload);
        let mut decoder = Decoder::default();
        for chunk in frame.chunks(4096) {
            decoder.push(chunk);
        }
        assert_eq!(decoder.next().unwrap(), Some(payload));
        assert_eq!(decoder.pending(), 0);
    }
}

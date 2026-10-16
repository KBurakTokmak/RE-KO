//! KO `.tbl` cipher and table parser.

use thiserror::Error;

pub const DEFAULT_VOLATILE_KEY: u16 = 0x0816;
pub const CIPHER_KEY_1: u16 = 0x6081;
pub const CIPHER_KEY_2: u16 = 0x1608;

#[derive(Debug, Error)]
pub enum TblError {
    #[error("buffer too short: need {needed} bytes, have {available}")]
    UnexpectedEof { needed: usize, available: usize },

    #[error("unknown column type identifier: {0}")]
    UnknownColumnType(u32),

    #[error("invalid string length {len} at offset {offset}")]
    InvalidStringLength { len: usize, offset: usize },
}

// Decrypts ciphertext in place using the KO stream cipher.
pub fn decrypt(data: &mut [u8], volatile_key: u16) {
    let mut key = volatile_key;
    for byte in data.iter_mut() {
        let c = *byte;
        *byte = ((key >> 8) as u8) ^ c;
        key = ((c as u16).wrapping_add(key))
            .wrapping_mul(CIPHER_KEY_1)
            .wrapping_add(CIPHER_KEY_2);
    }
}

// Encrypts plaintext in place using the KO stream cipher.
pub fn encrypt(data: &mut [u8], volatile_key: u16) {
    let mut key = volatile_key;
    for byte in data.iter_mut() {
        let p = *byte;
        let c = ((key >> 8) as u8) ^ p;
        *byte = c;
        key = ((c as u16).wrapping_add(key))
            .wrapping_mul(CIPHER_KEY_1)
            .wrapping_add(CIPHER_KEY_2);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    String,
    F32,
    F64,
    I64,
    U64,
}

impl ColumnType {
    pub fn from_u32(val: u32) -> Result<Self, TblError> {
        match val {
            1 => Ok(Self::I8),
            2 => Ok(Self::U8),
            3 => Ok(Self::I16),
            4 => Ok(Self::U16),
            5 => Ok(Self::I32),
            6 => Ok(Self::U32),
            7 => Ok(Self::String),
            8 => Ok(Self::F32),
            9 => Ok(Self::F64),
            10 => Ok(Self::I64),
            11 => Ok(Self::U64),
            _ => Err(TblError::UnknownColumnType(val)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    I8(i8),
    U8(u8),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    String(String),
    F32(f32),
    F64(f64),
    I64(i64),
    U64(u64),
}

#[derive(Debug, Clone)]
pub struct Table {
    pub columns: Vec<ColumnType>,
    pub rows: Vec<Vec<CellValue>>,
}

impl Table {
    // Parses a decrypted table byte buffer.
    pub fn parse(bytes: &[u8]) -> Result<Self, TblError> {
        let mut cur = Cursor::new(bytes);

        let num_cols = cur.read_u32()? as usize;
        let mut columns = Vec::with_capacity(num_cols);
        for _ in 0..num_cols {
            columns.push(ColumnType::from_u32(cur.read_u32()?)?);
        }

        let num_rows = cur.read_u32()? as usize;
        let mut rows = Vec::with_capacity(num_rows);

        for _ in 0..num_rows {
            let mut row = Vec::with_capacity(num_cols);
            for &col in &columns {
                let cell = match col {
                    ColumnType::I8 => CellValue::I8(cur.read_i8()?),
                    ColumnType::U8 => CellValue::U8(cur.read_u8()?),
                    ColumnType::I16 => CellValue::I16(cur.read_i16()?),
                    ColumnType::U16 => CellValue::U16(cur.read_u16()?),
                    ColumnType::I32 => CellValue::I32(cur.read_i32()?),
                    ColumnType::U32 => CellValue::U32(cur.read_u32()?),
                    ColumnType::String => {
                        let len = cur.read_u32()? as usize;
                        let slice = cur.read_bytes(len)?;
                        CellValue::String(String::from_utf8_lossy(slice).into_owned())
                    }
                    ColumnType::F32 => CellValue::F32(cur.read_f32()?),
                    ColumnType::F64 => CellValue::F64(cur.read_f64()?),
                    ColumnType::I64 => CellValue::I64(cur.read_i64()?),
                    ColumnType::U64 => CellValue::U64(cur.read_u64()?),
                };
                row.push(cell);
            }
            rows.push(row);
        }

        Ok(Self { columns, rows })
    }

    // Decrypts and parses in one call.
    pub fn from_encrypted_bytes(raw: &[u8]) -> Result<Self, TblError> {
        let mut buf = raw.to_vec();
        decrypt(&mut buf, DEFAULT_VOLATILE_KEY);
        Self::parse(&buf)
    }
}

struct Cursor<'a> {
    buf: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, offset: 0 }
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], TblError> {
        if self.offset + len > self.buf.len() {
            return Err(TblError::UnexpectedEof {
                needed: len,
                available: self.buf.len().saturating_sub(self.offset),
            });
        }
        let slice = &self.buf[self.offset..self.offset + len];
        self.offset += len;
        Ok(slice)
    }

    fn read_u8(&mut self) -> Result<u8, TblError> {
        Ok(self.read_bytes(1)?[0])
    }

    fn read_i8(&mut self) -> Result<i8, TblError> {
        Ok(self.read_u8()? as i8)
    }

    fn read_u16(&mut self) -> Result<u16, TblError> {
        let b = self.read_bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn read_i16(&mut self) -> Result<i16, TblError> {
        Ok(self.read_u16()? as i16)
    }

    fn read_u32(&mut self) -> Result<u32, TblError> {
        let b = self.read_bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_i32(&mut self) -> Result<i32, TblError> {
        Ok(self.read_u32()? as i32)
    }

    fn read_f32(&mut self) -> Result<f32, TblError> {
        let b = self.read_bytes(4)?;
        Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_f64(&mut self) -> Result<f64, TblError> {
        let b = self.read_bytes(8)?;
        Ok(f64::from_le_bytes(b.try_into().unwrap()))
    }

    fn read_i64(&mut self) -> Result<i64, TblError> {
        let b = self.read_bytes(8)?;
        Ok(i64::from_le_bytes(b.try_into().unwrap()))
    }

    fn read_u64(&mut self) -> Result<u64, TblError> {
        let b = self.read_bytes(8)?;
        Ok(u64::from_le_bytes(b.try_into().unwrap()))
    }
}

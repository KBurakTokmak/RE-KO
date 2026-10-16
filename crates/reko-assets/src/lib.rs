//! N3 engine asset readers. Pure: bytes in, structs out. No GPU code.

pub mod tbl;

pub use tbl::{CellValue, ColumnType, Table, TblError};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cipher_roundtrip() {
        let original = b"Knight Online 1453 table test payload 1234567890";
        let mut buf = original.to_vec();
        tbl::encrypt(&mut buf, tbl::DEFAULT_VOLATILE_KEY);
        assert_ne!(&buf[..], &original[..]);
        tbl::decrypt(&mut buf, tbl::DEFAULT_VOLATILE_KEY);
        assert_eq!(&buf[..], &original[..]);
    }

    #[test]
    fn test_load_real_skill_table_if_present() {
        let game_dir = reko_common::game_dir();
        let path = game_dir.join("Data").join("Skill_Magic_Main_us.tbl");
        if !path.exists() {
            return;
        }
        let raw = std::fs::read(&path).expect("read tbl file");
        let table = Table::from_encrypted_bytes(&raw).expect("parse tbl file");
        assert_eq!(table.columns.len(), 31);
        assert_eq!(table.columns[0], ColumnType::U32);
        assert_eq!(table.columns[1], ColumnType::String);
        assert!(!table.rows.is_empty());

        let sprint_row = &table.rows[0];
        assert_eq!(sprint_row[0], CellValue::U32(101001));
        assert_eq!(sprint_row[1], CellValue::String("Sprint".into()));
    }
}

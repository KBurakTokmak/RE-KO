# KO .tbl File Format & Cipher

Binary table storage for game definitions (items, skills, NPCs, quests).

## Cipher

Stream cipher over raw file bytes. Operates on 16-bit state (`key`).

- **VolatileKey**: `0x0816`
- **CipherKey1**: `0x6081`
- **CipherKey2**: `0x1608`

### Decryption

```
key = 0x0816
for i in 0..len:
    c = ciphertext[i]
    plaintext[i] = ((key & 0xFF00) >> 8) ^ c
    key = ((c + key) * 0x6081 + 0x1608) & 0xFFFF
```

Note: `key` advances using the **ciphertext** byte `c`, not the decrypted byte.

### Encryption

```
key = 0x0816
for i in 0..len:
    p = plaintext[i]
    c = ((key & 0xFF00) >> 8) ^ p
    ciphertext[i] = c
    key = ((c + key) * 0x6081 + 0x1608) & 0xFFFF
```

## Binary Layout (Decrypted)

Little-endian throughout.

```
[4 bytes] u32: column_count
[4 * column_count bytes] u32[column_count]: column_data_types
[4 bytes] u32: row_count
[rows...]
```

### Column Types

| ID | Type | Size | Notes |
|---|---|---|---|
| 1 | i8 | 1 byte | Signed byte |
| 2 | u8 | 1 byte | Unsigned byte |
| 3 | i16 | 2 bytes | Signed short |
| 4 | u16 | 2 bytes | Unsigned short |
| 5 | i32 | 4 bytes | Signed int |
| 6 | u32 | 4 bytes | Unsigned int |
| 7 | String | 4 + len | `u32` byte-length prefix followed by ASCII/Latin1 string |
| 8 | f32 | 4 bytes | 32-bit float |
| 9 | f64 | 8 bytes | 64-bit float |
| 10 | i64 | 8 bytes | Signed 64-bit int |
| 11 | u64 | 8 bytes | Unsigned 64-bit int |

Each row contains all columns sequentially according to `column_data_types`.
Rows have variable lengths if any column is of type 7 (String).

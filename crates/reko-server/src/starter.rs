//! User-specified starter kit, resolved against original v1453 item tables.

use reko_assets::{CellValue, Table};
use rusqlite::{Connection, OptionalExtension, params};
use std::{collections::HashMap, path::Path};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Item {
    pub id: u32,
    pub durability: u16,
    pub count: u16,
}

pub(crate) struct Catalog {
    items: HashMap<u32, Item>,
}

impl Catalog {
    pub fn load(game_dir: &Path) -> anyhow::Result<Self> {
        let table =
            Table::from_encrypted_bytes(&std::fs::read(game_dir.join("Data/item_org_us.tbl"))?)?;
        let expected = [
            (110110000, "Dagger"),
            (120150000, "Short Sword"),
            (180110000, "Wood Staff"),
            (190250000, "Priest Morning Star"),
            (170150000, "Wooden Shield"),
        ];
        let mut items = HashMap::new();
        for (id, name) in expected {
            let row = table
                .rows
                .iter()
                .find(|row| row.first() == Some(&CellValue::U32(id)))
                .ok_or_else(|| anyhow::anyhow!("missing starter item {id}"))?;
            anyhow::ensure!(
                matches!(row.get(2), Some(CellValue::String(label)) if label.trim() == name),
                "starter item name mismatch for {id}"
            );
            let group = match row.get(1) {
                Some(CellValue::U8(value)) => *value as u32,
                Some(CellValue::U32(value)) => *value,
                Some(CellValue::I32(value)) => (*value).try_into()?,
                _ => anyhow::bail!("invalid item extension group for {id}"),
            };
            let extension = Table::from_encrypted_bytes(&std::fs::read(
                game_dir.join(format!("Data/Item_Ext_{group}_us.tbl")),
            )?)?;
            anyhow::ensure!(
                extension
                    .rows
                    .iter()
                    .any(|row| row.first() == Some(&CellValue::U32(1))),
                "starter +1 extension missing for {id}"
            );
            let durability = match row.get(19) {
                Some(CellValue::U16(value)) => *value,
                Some(CellValue::I16(value)) => (*value).try_into()?,
                Some(CellValue::U32(value)) => (*value).try_into()?,
                Some(CellValue::I32(value)) => (*value).try_into()?,
                _ => anyhow::bail!("invalid durability for {id}"),
            };
            items.insert(
                id,
                Item {
                    id: id + 1,
                    durability,
                    count: 1,
                },
            );
        }
        Ok(Self { items })
    }

    fn kit(&self, class: u16) -> anyhow::Result<Vec<(u8, Item)>> {
        let weapon = match class % 100 {
            1 | 5 | 6 => 120150000,
            2 | 7 | 8 => 110110000,
            3 | 9 | 10 => 180110000,
            4 | 11 | 12 => 190250000,
            _ => anyhow::bail!("unsupported starter class {class}"),
        };
        let mut kit = vec![(
            6,
            *self
                .items
                .get(&weapon)
                .ok_or_else(|| anyhow::anyhow!("starter catalog incomplete"))?,
        )];
        if matches!(class % 100, 4 | 11 | 12) {
            kit.push((8, self.items[&170150000]));
        }
        Ok(kit)
    }

    #[cfg(test)]
    fn fixture() -> Self {
        Self {
            items: [
                (110110000, 4000),
                (120150000, 5000),
                (180110000, 5000),
                (190250000, 10000),
                (170150000, 5000),
            ]
            .into_iter()
            .map(|(id, durability)| {
                (
                    id,
                    Item {
                        id: id + 1,
                        durability,
                        count: 1,
                    },
                )
            })
            .collect(),
        }
    }
}

pub(crate) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS character_equipment (
        account TEXT NOT NULL COLLATE NOCASE, slot INTEGER NOT NULL,
        equipment_slot INTEGER NOT NULL CHECK(equipment_slot BETWEEN 0 AND 13),
        item_id INTEGER NOT NULL, durability INTEGER NOT NULL,
        PRIMARY KEY(account,slot,equipment_slot),
        FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE
    );
    CREATE TABLE IF NOT EXISTS character_inventory (
        account TEXT NOT NULL COLLATE NOCASE, slot INTEGER NOT NULL,
        inventory_slot INTEGER NOT NULL CHECK(inventory_slot BETWEEN 0 AND 27),
        item_id INTEGER NOT NULL, durability INTEGER NOT NULL,
        PRIMARY KEY(account,slot,inventory_slot),
        FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE
    );
    CREATE TABLE IF NOT EXISTS character_wallets (
        account TEXT NOT NULL COLLATE NOCASE, slot INTEGER NOT NULL, coins INTEGER NOT NULL,
        PRIMARY KEY(account,slot), FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE
    );
    CREATE TABLE IF NOT EXISTS starter_grants (
        account TEXT NOT NULL COLLATE NOCASE, slot INTEGER NOT NULL, version INTEGER NOT NULL,
        PRIMARY KEY(account,slot), FOREIGN KEY(account,slot) REFERENCES lobby_characters(account,slot) ON DELETE CASCADE
    );
    UPDATE character_equipment AS e SET equipment_slot=8
    WHERE equipment_slot=7 AND item_id IN (110110001,170150001)
    AND NOT EXISTS (SELECT 1 FROM character_equipment AS other WHERE other.account=e.account AND other.slot=e.slot AND other.equipment_slot=8);")?;
    let mut columns = db.prepare("PRAGMA table_info(character_inventory)")?;
    let has_count = columns
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|name| name == "count");
    if !has_count {
        db.execute_batch("ALTER TABLE character_inventory ADD COLUMN count INTEGER NOT NULL DEFAULT 1 CHECK(count BETWEEN 1 AND 9999)")?;
    }
    Ok(())
}

pub(crate) fn inventory(db: &Connection, account: &str, slot: u8) -> rusqlite::Result<[Item; 28]> {
    let mut items = [Item::default(); 28];
    let mut statement=db.prepare("SELECT inventory_slot,item_id,durability,count FROM character_inventory WHERE account=?1 AND slot=?2")?;
    for row in statement.query_map(params![account, slot], |row| {
        Ok((
            row.get::<_, usize>(0)?,
            Item {
                id: row.get(1)?,
                durability: row.get(2)?,
                count: row.get(3)?,
            },
        ))
    })? {
        let (index, item) = row?;
        *items.get_mut(index).ok_or(rusqlite::Error::InvalidQuery)? = item;
    }
    Ok(items)
}

pub(crate) fn grant_missing(db: &mut Connection, catalog: &Catalog) -> anyhow::Result<()> {
    let characters: Vec<(String,u8,u16)> = db.prepare("SELECT c.account,c.slot,c.class FROM lobby_characters c LEFT JOIN starter_grants g ON c.account=g.account AND c.slot=g.slot WHERE g.version IS NULL")?.query_map([], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
    for (account, slot, class) in characters {
        let kit = catalog.kit(class)?;
        let tx = db.transaction()?;
        for (equipment_slot, item) in kit {
            tx.execute("INSERT OR IGNORE INTO character_equipment(account,slot,equipment_slot,item_id,durability) VALUES (?1,?2,?3,?4,?5)", params![account,slot,equipment_slot,item.id,item.durability])?;
        }
        tx.execute(
            "INSERT OR IGNORE INTO character_wallets(account,slot,coins) VALUES (?1,?2,20000)",
            params![account, slot],
        )?;
        tx.execute(
            "INSERT INTO starter_grants(account,slot,version) VALUES (?1,?2,1)",
            params![account, slot],
        )?;
        tx.commit()?;
    }
    Ok(())
}

pub(crate) fn equipment(db: &Connection, account: &str, slot: u8) -> rusqlite::Result<[Item; 14]> {
    let mut items = [Item::default(); 14];
    let mut statement = db.prepare("SELECT equipment_slot,item_id,durability FROM character_equipment WHERE account=?1 AND slot=?2")?;
    for row in statement.query_map(params![account, slot], |row| {
        Ok((
            row.get::<_, usize>(0)?,
            Item {
                id: row.get(1)?,
                durability: row.get(2)?,
                count: 1,
            },
        ))
    })? {
        let (index, item) = row?;
        let entry = items.get_mut(index).ok_or(rusqlite::Error::InvalidQuery)?;
        *entry = item;
    }
    Ok(items)
}

pub(crate) fn coins(db: &Connection, account: &str, slot: u8) -> rusqlite::Result<u32> {
    Ok(db
        .query_row(
            "SELECT coins FROM character_wallets WHERE account=?1 AND slot=?2",
            params![account, slot],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kit_is_equipped_and_money_is_not_reset_on_relogin() {
        let mut db = Connection::open_in_memory().unwrap();
        crate::characters::initialize(&db).unwrap();
        initialize(&db).unwrap();
        crate::characters::ensure_account(&db, "admin").unwrap();
        crate::characters::set_nation(&db, "admin", 1).unwrap();
        for (slot, class, race) in [(0, 102, 2), (1, 104, 2), (2, 103, 3)] {
            let character = crate::characters::Character {
                slot,
                name: format!("test{slot}"),
                race,
                class,
                face: 0,
                hair: 0,
                stats: [60; 5],
            };
            assert_eq!(
                crate::characters::create(&mut db, "admin", &character).unwrap(),
                0
            );
        }
        let catalog = Catalog::fixture();
        grant_missing(&mut db, &catalog).unwrap();
        assert_eq!(equipment(&db, "admin", 0).unwrap()[6].id, 110110001);
        assert_eq!(equipment(&db, "admin", 1).unwrap()[8].id, 170150001);
        assert_eq!(coins(&db, "admin", 0).unwrap(), 20000);
        db.execute(
            "UPDATE character_wallets SET coins=123 WHERE account='admin' AND slot=0",
            [],
        )
        .unwrap();
        grant_missing(&mut db, &catalog).unwrap();
        assert_eq!(coins(&db, "admin", 0).unwrap(), 123);
        assert_eq!(catalog.kit(201).unwrap()[0].1.id, 120150001);
    }
    #[test]
    fn original_item_ids_and_extensions_resolve_when_available() {
        let dir = reko_common::game_dir();
        if !dir.join("Data/item_org_us.tbl").exists() {
            return;
        }
        let catalog = Catalog::load(&dir).unwrap();
        assert_eq!(catalog.kit(102).unwrap()[0].1.id, 110110001);
        assert_eq!(catalog.kit(104).unwrap()[0].1.id, 190250001);
    }
}

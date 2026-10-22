//! Frame-zero joint bounds used by the original N3 character collision radius.
use anyhow::{Context, ensure};
use reko_assets::{CellValue, Table};
use std::{
    collections::HashMap,
    io::{Cursor, Read},
    path::Path,
};

type Matrix = [[f32; 4]; 4];
fn identity() -> Matrix {
    std::array::from_fn(|i| std::array::from_fn(|j| if i == j { 1.0 } else { 0.0 }))
}
fn multiply(a: Matrix, b: Matrix) -> Matrix {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..4).map(|k| a[i][k] * b[k][j]).sum()))
}
fn rotation([x, y, z, w]: [f32; 4]) -> Matrix {
    [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y + z * w),
            2.0 * (x * z - y * w),
            0.0,
        ],
        [
            2.0 * (x * y - z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z + x * w),
            0.0,
        ],
        [
            2.0 * (x * z + y * w),
            2.0 * (y * z - x * w),
            1.0 - 2.0 * (x * x + y * y),
            0.0,
        ],
        [0.0, 0.0, 0.0, 1.0],
    ]
}
struct Reader<'a> {
    data: Cursor<&'a [u8]>,
    points: Vec<[f32; 3]>,
}
impl Reader<'_> {
    fn string(&mut self) -> anyhow::Result<String> {
        let length = self.word()? as usize;
        ensure!(length <= 512, "excessive asset filename");
        let start = self.data.position() as usize;
        self.skip(length)?;
        Ok(std::str::from_utf8(&self.data.get_ref()[start..start + length])?.to_string())
    }
    fn word(&mut self) -> anyhow::Result<u32> {
        let mut bytes = [0; 4];
        self.data.read_exact(&mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }
    fn floats<const N: usize>(&mut self) -> anyhow::Result<[f32; N]> {
        let mut out = [0.0; N];
        for v in &mut out {
            *v = f32::from_bits(self.word()?);
            ensure!(v.is_finite(), "nonfinite joint transform");
        }
        Ok(out)
    }
    fn skip(&mut self, length: usize) -> anyhow::Result<()> {
        let end = (self.data.position() as usize)
            .checked_add(length)
            .context("joint length overflow")?;
        ensure!(end <= self.data.get_ref().len(), "truncated joint data");
        self.data.set_position(end as u64);
        Ok(())
    }
    fn key<const N: usize>(&mut self, fallback: [f32; N], kind: u32) -> anyhow::Result<[f32; N]> {
        let count = self.word()? as usize;
        ensure!(count <= 1_000_000, "excessive joint keys");
        if count == 0 {
            return Ok(fallback);
        }
        ensure!(self.word()? == kind, "unexpected joint key type");
        let rate = self.floats::<1>()?[0];
        ensure!(rate > 0.0, "invalid key rate");
        let first = self.floats()?;
        self.skip((count - 1) * N * 4)?;
        Ok(first)
    }
    fn joint(&mut self, parent: Matrix, depth: usize) -> anyhow::Result<()> {
        ensure!(
            depth < 128 && self.points.len() < 4096,
            "excessive joint hierarchy"
        );
        let length = self.word()? as usize;
        ensure!(length <= 256, "invalid joint name");
        self.skip(length)?;
        let pos = self.floats::<3>()?;
        let rot = self.floats::<4>()?;
        let scale = self.floats::<3>()?;
        let pos = self.key(pos, 0)?;
        let rot = self.key(rot, 1)?;
        let scale = self.key(scale, 0)?;
        let orient = self.key([0.0, 0.0, 0.0, 1.0], 1)?;
        let mut local = multiply(rotation(rot), rotation(orient));
        for row in local.iter_mut().take(3) {
            for (value, factor) in row.iter_mut().zip(scale) {
                *value *= factor;
            }
        }
        local[3][..3].copy_from_slice(&pos);
        let world = multiply(local, parent);
        self.points.push([world[3][0], world[3][1], world[3][2]]);
        let children = self.word()?;
        ensure!(children <= 4096, "excessive child joints");
        for _ in 0..children {
            self.joint(world, depth + 1)?;
        }
        Ok(())
    }
}
fn asset_path(name: &str) -> anyhow::Result<String> {
    let relative = name.replace('\\', "/").to_ascii_lowercase();
    ensure!(
        !relative.is_empty()
            && Path::new(&relative)
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
        "invalid character asset path"
    );
    Ok(relative)
}
fn character_joint(bytes: &[u8]) -> anyhow::Result<String> {
    let mut reader = Reader {
        data: Cursor::new(bytes),
        points: Vec::new(),
    };
    reader.string()?;
    reader.floats::<10>()?;
    reader.key([0.0; 3], 0)?;
    reader.key([0.0, 0.0, 0.0, 1.0], 1)?;
    reader.key([1.0; 3], 0)?;
    reader.string()?; // Collision and climb meshes precede the root joint.
    reader.string()?;
    let joint = asset_path(&reader.string()?)?;
    ensure!(joint.ends_with(".n3joint"), "invalid character root joint");
    Ok(joint)
}
fn joint_asset(game: &Path, row: &[CellValue]) -> anyhow::Result<String> {
    // Native InitChr loads the complete character before consulting table joint fields.
    if let Some(CellValue::String(character)) = row.get(15)
        && !character.is_empty()
    {
        let relative = asset_path(character)?;
        return character_joint(&std::fs::read(game.join(relative))?);
    }
    let Some(CellValue::String(joint)) = row.get(2) else {
        anyhow::bail!("missing table root joint");
    };
    asset_path(joint)
}
fn skeleton_radius(bytes: &[u8]) -> anyhow::Result<f32> {
    let mut reader = Reader {
        data: Cursor::new(bytes),
        points: Vec::new(),
    };
    reader.joint(identity(), 0)?;
    ensure!(
        reader.data.position() as usize == bytes.len(),
        "unexpected trailing joint data"
    );
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    for point in reader.points {
        for i in 0..3 {
            minimum[i] = minimum[i].min(point[i]);
            maximum[i] = maximum[i].max(point[i]);
        }
    }
    let diameter_squared: f32 = (0..3)
        .map(|i| {
            let span = maximum[i] - minimum[i];
            let span = if span < 0.5 { span + 0.5 } else { span };
            span * span
        })
        .sum();
    let radius = diameter_squared.sqrt() * 0.5;
    ensure!(radius.is_finite() && radius > 0.0, "invalid joint radius");
    Ok(radius)
}

#[derive(Default)]
pub(super) struct Radii {
    pub monsters: HashMap<u16, f32>,
    pub players: HashMap<u8, f32>,
    forms: HashMap<u32, (u16, u16)>,
}
impl Radii {
    pub fn load(game: &Path, looks: &Table) -> anyhow::Result<Self> {
        let players =
            Table::from_encrypted_bytes(&std::fs::read(game.join("Data/UPC_DefaultLooks.tbl"))?)?;
        let mut result = Self::default();
        let forms =
            Table::from_encrypted_bytes(&std::fs::read(game.join("Data/Skill_Magic_6.tbl"))?)?;
        for row in forms.rows {
            let id = crate::combat::integer(&row, 0) as u32;
            if id / 1000 == 472 {
                result.forms.insert(
                    id,
                    (
                        crate::combat::integer(&row, 4).try_into()?,
                        crate::combat::integer(&row, 3).try_into()?,
                    ),
                );
            }
        }
        let mut cache = HashMap::new();
        for (rows, player) in [(&looks.rows, false), (&players.rows, true)] {
            for row in rows {
                let relative = match joint_asset(game, row) {
                    Ok(relative) => relative,
                    Err(error) => {
                        tracing::warn!(appearance=crate::combat::integer(row, 0),%error,"native character root unavailable");
                        continue;
                    }
                };
                let radius = if let Some(radius) = cache.get(&relative) {
                    *radius
                } else {
                    match std::fs::read(game.join(&relative))
                        .with_context(|| relative.clone())
                        .and_then(|b| skeleton_radius(&b))
                    {
                        Ok(radius) => {
                            cache.insert(relative, radius);
                            radius
                        }
                        Err(error) => {
                            tracing::warn!(joint=%relative,%error,"native model radius unavailable");
                            continue;
                        }
                    }
                };
                let id = crate::combat::integer(row, 0);
                if player {
                    result.players.insert(u8::try_from(id)?, radius);
                } else {
                    result.monsters.insert(u16::try_from(id)?, radius);
                }
            }
        }
        tracing::info!(
            monsters = result.monsters.len(),
            players = result.players.len(),
            "original model radii loaded"
        );
        Ok(result)
    }
    pub fn player(&self, race: u8, form: Option<u32>) -> Option<f32> {
        form.and_then(|id| self.forms.get(&id))
            .and_then(|(model, size)| {
                self.monsters
                    .get(model)
                    .map(|r| r * f32::from(*size) / 100.0)
            })
            .or_else(|| self.players.get(&race).copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hierarchy_rotation_scale_and_native_minimum_bounds() {
        fn node(out: &mut Vec<u8>, pos: [f32; 3], rot: [f32; 4], scale: [f32; 3], children: u32) {
            out.extend(0u32.to_le_bytes());
            for value in pos.into_iter().chain(rot).chain(scale) {
                out.extend(value.to_le_bytes());
            }
            out.extend([0; 16]); // Four absent animation-key blocks.
            out.extend(children.to_le_bytes());
        }
        let mut data = Vec::new();
        let half = std::f32::consts::FRAC_1_SQRT_2;
        node(
            &mut data,
            [10.0, 20.0, 30.0],
            [0.0, 0.0, half, half],
            [2.0; 3],
            1,
        );
        node(
            &mut data,
            [2.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
            [1.0; 3],
            0,
        );
        assert!((skeleton_radius(&data).unwrap() - (16.5f32).sqrt() / 2.0).abs() < 0.0001);
        for end in 0..data.len() {
            assert!(skeleton_radius(&data[..end]).is_err());
        }
        let mut key = Vec::new();
        for bits in [1u32, 0, 30f32.to_bits(), 4f32.to_bits(), 0, 0] {
            key.extend(bits.to_le_bytes());
        }
        data.splice(108..112, key);
        assert!((skeleton_radius(&data).unwrap() - (64.5f32).sqrt() / 2.0).abs() < 0.0001);
    }
    #[test]
    fn original_boss_radii_and_races_load() {
        let game = reko_common::game_dir();
        if !game.join("Data/NPC_Looks.tbl").exists() {
            return;
        }
        let looks =
            Table::from_encrypted_bytes(&std::fs::read(game.join("Data/NPC_Looks.tbl")).unwrap())
                .unwrap();
        let radii = Radii::load(&game, &looks).unwrap();
        assert_eq!(radii.players.len(), 8);
        let troll = radii.monsters[&1701];
        let player = radii.players[&2];
        assert!(troll > player);
        assert!(
            radii.monsters[&6200] > player,
            "Felankor uses its N3Chr root joint"
        );
        let character = std::fs::read(game.join("Chr/mon_dragon.n3chr")).unwrap();
        assert_eq!(
            character_joint(&character).unwrap(),
            "chr/mon_dragon.n3joint"
        );
        for end in 0..100.min(character.len()) {
            assert!(character_joint(&character[..end]).is_err());
        }
    }
}

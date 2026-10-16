use crate::{
    characters, combat,
    lobby::Lobby,
    starter,
    world_entry::{Bootstrap, Session},
};
use rusqlite::Connection;
pub(crate) fn setup() -> Option<(Lobby, Session)> {
    let game = reko_common::game_dir();
    let rules = std::path::Path::new("../../logs/reference-coefficients.csv");
    if !game.join("Data/item_org_us.tbl").exists() || !rules.exists() {
        return None;
    }
    let mut lobby = Lobby::new(
        Connection::open_in_memory().unwrap(),
        characters::CreationRules::fixture(),
        Bootstrap { next_level_exp: 50 },
    )
    .unwrap();
    lobby.combat = Some(combat::Catalog::load(&game, rules).unwrap());
    let c = characters::Character {
        slot: 0,
        name: "ServiceTest".into(),
        race: 2,
        class: 108,
        face: 0,
        hair: 0,
        stats: [60, 150, 255, 50, 50],
    };
    {
        let mut db = lobby.db.lock().unwrap();
        characters::ensure_account(&db, "test").unwrap();
        characters::set_nation(&db, "test", 1).unwrap();
        characters::create(&mut db, "test", &c).unwrap();
        db.execute(
            "INSERT INTO character_wallets VALUES ('test',0,1000000)",
            [],
        )
        .unwrap();
    }
    let mut s = Session::new(c).unwrap();
    s.entered = true;
    s.progress.level = 80;
    s.combat = lobby
        .combat
        .as_ref()
        .unwrap()
        .calculate(&s.character, &s.progress, &s.equipment)
        .unwrap();
    s.base_combat = s.combat;
    s.skills.set_limits(s.combat.hp, s.combat.mp);
    s.coins = 1000000;
    s.inventory = [starter::Item::default(); 28];
    Some((lobby, s))
}

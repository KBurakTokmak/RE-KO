use super::*;
use crate::{social, starter};

#[tokio::test]
async fn native_swift_targets_a_nonparty_player_and_received_foreign_class_buffs_can_be_cancelled()
{
    use tokio::{io::AsyncReadExt, net::TcpListener};
    let Some((lobby, mut s)) = setup() else {
        return;
    };
    let mut target = ally(&lobby, &s, "SocketTarget");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (mut sock, peer) = listener.accept().await.unwrap();
    async fn receive(c: &mut TcpStream, prefix: &[u8]) -> Vec<u8> {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let mut h = [0; 4];
                c.read_exact(&mut h).await.unwrap();
                let n = u16::from_le_bytes([h[2], h[3]]) as usize;
                let mut p = vec![0; n + 2];
                c.read_exact(&mut p).await.unwrap();
                p.truncate(n);
                if p.starts_with(prefix) {
                    return p;
                }
            }
        })
        .await
        .unwrap()
    }
    let mut request = skills::effect(108010, s.player_id);
    request[8..10].copy_from_slice(&target.player_id.to_le_bytes());
    request[1] = 1;
    let before = s.skills.mana;
    crate::magic::handle(
        &mut sock,
        peer,
        &request,
        &lobby,
        Some("test"),
        Some(&mut s),
    )
    .await;
    assert_eq!(
        &receive(&mut client, &[0x31, 1]).await[8..10],
        &target.player_id.to_le_bytes()
    );
    request[1] = 3;
    crate::magic::handle(
        &mut sock,
        peer,
        &request,
        &lobby,
        Some("test"),
        Some(&mut s),
    )
    .await;
    assert_eq!(
        &receive(&mut client, &[0x31, 3]).await[8..10],
        &target.player_id.to_le_bytes()
    );
    assert!(!s.skills.buffs.contains_key(&6));
    assert_eq!(
        s.skills.mana,
        before - lobby.skills.as_ref().unwrap().spells[&108010].mana
    );
    social::synchronize(&lobby, "SocketTarget", &mut target);
    assert_eq!(
        target.skills.speed(lobby.skills.as_ref(), skills::now_ms()),
        68
    );
    let armor = spell(&lobby, 112621);
    supply(&lobby, &armor);
    apply(&lobby, "test", &s, &armor, target.player_id).unwrap();
    social::synchronize(&lobby, "SocketTarget", &mut target);
    assert!(target.skills.buffs.contains_key(&2));
    let before = target.skills.mana;
    let mut cancel = skills::effect(112621, target.player_id);
    cancel[1] = 6;
    crate::magic::handle(
        &mut sock,
        peer,
        &cancel,
        &lobby,
        Some("SocketTarget"),
        Some(&mut target),
    )
    .await;
    assert_eq!(receive(&mut client, &[0x31, 5]).await, [0x31, 5, 2]);
    assert!(!state(&lobby, "SocketTarget").buffs.contains_key(&2));
    assert_eq!(target.skills.mana, before);
}

fn setup() -> Option<(Lobby, Session)> {
    let (mut lobby, mut s) = crate::world_service_tests::setup()?;
    lobby.skills = Some(skills::Catalog::load(&reko_common::game_dir()).unwrap());
    s.progress.points = [0, 0, 0, 0, 0, 80, 80, 80, 20];
    s.base_combat.hp = 5000;
    s.base_combat.mp = 10000;
    s.combat = s.base_combat;
    s.skills.set_limits(5000, 10000);
    skills::save(&lobby.db.lock().unwrap(), "test", 0, &s.skills).unwrap();
    social::synchronize(&lobby, "test", &mut s);
    Some((lobby, s))
}
fn ally(lobby: &Lobby, s: &Session, name: &str) -> Session {
    let mut p = social::tests::player(lobby, name, 1, 80);
    p.position = s.position;
    p.base_combat = s.base_combat;
    p.combat = p.base_combat;
    p.skills.set_limits(5000, 10000);
    skills::save(&lobby.db.lock().unwrap(), name, 0, &p.skills).unwrap();
    social::synchronize(lobby, name, &mut p);
    p
}
fn state(lobby: &Lobby, name: &str) -> skills::State {
    skills::load(&lobby.db.lock().unwrap(), name, 0).unwrap()
}
fn spell(lobby: &Lobby, id: u32) -> skills::Spell {
    let mut s = lobby.skills.as_ref().unwrap().spells[&id].clone();
    s.cooldown_ms = 0;
    s
}
fn supply(lobby: &Lobby, spell: &skills::Spell) {
    if spell.use_item != 0 {
        lobby
            .db
            .lock()
            .unwrap()
            .execute(
                "INSERT OR REPLACE INTO character_inventory VALUES('test',0,0,?1,1,5)",
                [spell.consumed_item()],
            )
            .unwrap();
    }
}

#[test]
fn wolf_reaches_only_living_loaded_party_members_in_original_radius_and_pays_once() {
    let Some((lobby, s)) = setup() else {
        return;
    };
    let near = ally(&lobby, &s, "Near");
    let mut far = ally(&lobby, &s, "Far");
    let mut dead = ally(&lobby, &s, "Dead");
    let mut loading = ally(&lobby, &s, "Loading");
    let mut elsewhere = ally(&lobby, &s, "Elsewhere");
    let outsider = ally(&lobby, &s, "Outsider");
    far.position.x += 360;
    dead.skills.hp = 0;
    loading.zone_loading = true;
    elsewhere.position.zone = 1;
    for (name, player) in [
        ("Far", &mut far),
        ("Dead", &mut dead),
        ("Loading", &mut loading),
        ("Elsewhere", &mut elsewhere),
    ] {
        skills::save(&lobby.db.lock().unwrap(), name, 0, &player.skills).unwrap();
        social::synchronize(&lobby, name, player);
    }
    lobby.social.lock().unwrap().parties.insert(
        s.player_id,
        vec![
            s.player_id,
            near.player_id,
            far.player_id,
            dead.player_id,
            loading.player_id,
            elsewhere.player_id,
        ],
    );
    let wolf = spell(&lobby, 108030);
    assert_eq!(wolf.moral, 6);
    supply(&lobby, &wolf);
    let before = state(&lobby, "test").mana;
    apply(&lobby, "test", &s, &wolf, u16::MAX).unwrap();
    for name in ["test", "Near"] {
        assert_eq!(state(&lobby, name).buffs[&4].skill, wolf.id);
    }
    for name in ["Far", "Dead", "Loading", "Elsewhere", "Outsider"] {
        assert!(!state(&lobby, name).buffs.contains_key(&4));
    }
    assert_eq!(state(&lobby, "test").mana, before - wolf.mana);
    assert_eq!(state(&lobby, "Near").mana, 10000);
    if wolf.use_item != 0 {
        assert_eq!(
            starter::inventory(&lobby.db.lock().unwrap(), "test", 0).unwrap()[0].count,
            4
        );
    }
    let paid = state(&lobby, "test").mana;
    assert!(apply(&lobby, "test", &s, &wolf, u16::MAX).is_err());
    assert_eq!(state(&lobby, "test").mana, paid);
    let late = ally(&lobby, &s, "Late");
    lobby
        .social
        .lock()
        .unwrap()
        .parties
        .get_mut(&s.player_id)
        .unwrap()
        .push(late.player_id);
    apply(&lobby, "test", &s, &wolf, u16::MAX).unwrap();
    assert!(state(&lobby, "Late").buffs.contains_key(&4));
    assert_eq!(state(&lobby, "test").mana, paid - wolf.mana);
    assert!(apply(&lobby, "test", &s, &wolf, outsider.player_id).is_err());
}

#[test]
fn group_heal_restore_and_buff_save_all_recipients_atomically_with_rollback() {
    let Some((lobby, mut s)) = setup() else {
        return;
    };
    let mut target = ally(&lobby, &s, "HealTarget");
    s.skills.hp = 100;
    target.skills.hp = 100;
    skills::save(&lobby.db.lock().unwrap(), "test", 0, &s.skills).unwrap();
    skills::save(&lobby.db.lock().unwrap(), "HealTarget", 0, &target.skills).unwrap();
    social::synchronize(&lobby, "test", &mut s);
    social::synchronize(&lobby, "HealTarget", &mut target);
    lobby
        .social
        .lock()
        .unwrap()
        .parties
        .insert(s.player_id, vec![s.player_id, target.player_id]);
    let heal = spell(&lobby, 112557);
    supply(&lobby, &heal);
    lobby.db.lock().unwrap().execute_batch("CREATE TRIGGER reject_party_skill BEFORE UPDATE ON character_skill_state WHEN NEW.account='HealTarget' BEGIN SELECT RAISE(ABORT,'fixture');END;").unwrap();
    assert!(apply(&lobby, "test", &s, &heal, u16::MAX).is_err());
    assert_eq!(
        (state(&lobby, "test").hp, state(&lobby, "test").mana),
        (100, 10000)
    );
    assert_eq!(state(&lobby, "HealTarget").hp, 100);
    lobby
        .db
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_party_skill;")
        .unwrap();
    apply(&lobby, "test", &s, &heal, u16::MAX).unwrap();
    assert_eq!(
        (state(&lobby, "test").hp, state(&lobby, "HealTarget").hp),
        (1060, 1060)
    );
    let restore = spell(&lobby, 112575);
    supply(&lobby, &restore);
    apply(&lobby, "test", &s, &restore, u16::MAX).unwrap();
    for name in ["test", "HealTarget"] {
        let mut st = state(&lobby, name);
        let effect = st.periodic[&restore.id];
        let hp = st.hp;
        st.tick_periodic(effect.next);
        assert!(st.hp > hp);
    }
    let buff = spell(&lobby, 112656);
    supply(&lobby, &buff);
    apply(&lobby, "test", &s, &buff, u16::MAX).unwrap();
    assert!(state(&lobby, "HealTarget").max_hp > 5000);
    let counter = spell(&lobby, 112676);
    supply(&lobby, &counter);
    apply(&lobby, "test", &s, &counter, u16::MAX).unwrap();
    assert!(state(&lobby, "HealTarget").buffs.contains_key(&29));
}

#[test]
fn swift_armor_resistance_heal_and_cure_accept_friendly_nonparty_targets_but_enforce_moral_and_range()
 {
    let Some((lobby, s)) = setup() else {
        return;
    };
    let mut target = ally(&lobby, &s, "FriendlyTarget");
    for id in [108010, 112621, 110506] {
        let skill = spell(&lobby, id);
        assert_eq!(skill.moral, 2);
        supply(&lobby, &skill);
        apply(&lobby, "test", &s, &skill, target.player_id).unwrap();
    }
    let st = state(&lobby, "FriendlyTarget");
    assert_eq!(st.speed(lobby.skills.as_ref(), skills::now_ms()), 68);
    assert!(st.buffs.contains_key(&2) && st.buffs.contains_key(&8));
    let party_only = spell(&lobby, 112606);
    supply(&lobby, &party_only);
    assert!(apply(&lobby, "test", &s, &party_only, target.player_id).is_err());
    target.skills = st;
    target.skills.hp = 100;
    target.skills.periodic.insert(
        999,
        skills::Periodic {
            amount: -5,
            next: skills::now_ms() + 2000,
            until: skills::now_ms() + 6000,
        },
    );
    skills::save(
        &lobby.db.lock().unwrap(),
        "FriendlyTarget",
        0,
        &target.skills,
    )
    .unwrap();
    social::synchronize(&lobby, "FriendlyTarget", &mut target);
    let heal = spell(&lobby, 112509);
    supply(&lobby, &heal);
    apply(&lobby, "test", &s, &heal, target.player_id).unwrap();
    assert_eq!(state(&lobby, "FriendlyTarget").hp, 340);
    let cure = spell(&lobby, 112535);
    supply(&lobby, &cure);
    apply(&lobby, "test", &s, &cure, target.player_id).unwrap();
    assert!(state(&lobby, "FriendlyTarget").periodic.is_empty());
    target.position.x += 260;
    social::synchronize(&lobby, "FriendlyTarget", &mut target);
    let mana = state(&lobby, "test").mana;
    assert!(apply(&lobby, "test", &s, &heal, target.player_id).is_err());
    assert_eq!(state(&lobby, "test").mana, mana);
}

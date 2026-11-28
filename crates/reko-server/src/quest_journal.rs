//! Private chat access to the character's persisted concurrent quest journal.
use crate::{combat, lobby::Lobby, world_entry::Session};
use reko_assets::{CellValue, Table};
use rusqlite::params;
use std::net::SocketAddr;
use tokio::net::TcpStream;

fn table(name: &str) -> Vec<Vec<CellValue>> {
    std::fs::read(reko_common::game_dir().join("Data").join(name))
        .ok()
        .and_then(|b| Table::from_encrypted_bytes(&b).ok())
        .map_or_else(Vec::new, |t| t.rows)
}

/// Quest_guide_us.tbl column 0 is a guide ID, not a quest flag. The client
/// Quest_Helper_us.tbl links them: column 9 is the flag, 16 the guide ID and
/// 4/5 the class/nation, so pick the guide shown to this character's variant.
fn guide_row<'a>(
    guide: &'a [Vec<CellValue>],
    helpers: &[Vec<CellValue>],
    s: &Session,
    id: u16,
) -> Option<&'a [CellValue]> {
    let job = match s.character.class % 100 {
        1 | 5 | 6 => 1,
        2 | 7 | 8 => 2,
        3 | 9 | 10 => 3,
        4 | 11 | 12 => 4,
        _ => 0,
    };
    let nation = i32::from(s.character.nation());
    let mut ids: Vec<(bool, i32)> = helpers
        .iter()
        .filter(|r| combat::integer(r, 9) == i32::from(id) && combat::integer(r, 16) > 0)
        .map(|r| {
            let class = combat::integer(r, 4);
            let side = combat::integer(r, 5);
            let own = (class == 5 || class == job) && (side == 3 || side == nation);
            (!own, combat::integer(r, 16))
        })
        .collect();
    ids.sort_unstable();
    ids.iter().find_map(|&(_, guide_id)| {
        guide
            .iter()
            .find(|r| combat::integer(r, 0) == guide_id)
            .map(Vec::as_slice)
    })
}
fn text(row: &[CellValue], column: usize) -> &str {
    match row.get(column) {
        Some(CellValue::String(s)) => s,
        _ => "",
    }
}
fn lines(
    lobby: &Lobby,
    account: &str,
    s: &Session,
    command: &str,
    argument: Option<&str>,
) -> anyhow::Result<Vec<String>> {
    let db = lobby.db.lock().unwrap();
    let mut q=db.prepare("SELECT quest,state FROM character_quests WHERE account=?1 AND slot=?2 AND quest<32000 AND state IN (1,3) ORDER BY quest")?;
    let active = q
        .query_map(params![account, s.character.slot], |r| {
            Ok((r.get::<_, u16>(0)?, r.get::<_, u8>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let guide = table("Quest_guide_us.tbl");
    let helpers = table("Quest_Helper_us.tbl");
    let title = |id: u16| {
        guide_row(&guide, &helpers, s, id)
            .map(|r| text(r, 3))
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Quest {id}"))
    };
    if command == "quests" {
        let page = argument
            .unwrap_or("1")
            .parse::<usize>()
            .ok()
            .filter(|p| *p > 0 && *p <= crate::quests::ACTIVE_LIMIT)
            .ok_or_else(|| anyhow::anyhow!("Usage: quests [page]"))?;
        let pages = active.len().div_ceil(8).max(1);
        anyhow::ensure!(page <= pages, "The journal has {pages} page(s).");
        let mut out = vec![format!(
            "Quest journal: {} active, page {page}/{pages}.",
            active.len()
        )];
        for &(id, state) in active.iter().skip((page - 1) * 8).take(8) {
            out.push(format!(
                "{id}: {} [{}]",
                title(id),
                if state == 3 { "Ready" } else { "Accepted" }
            ));
        }
        out.push("Use quest <ID> for details; questcancel <ID> to abandon a quest.".into());
        return Ok(out);
    }
    let id = argument
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| anyhow::anyhow!("Usage: quest <ID>"))?;
    let &(_, state) = active
        .iter()
        .find(|(q, _)| *q == id)
        .ok_or_else(|| anyhow::anyhow!("This quest is not active on your character."))?;
    let mut out = vec![format!(
        "{id}: {} [{}]",
        title(id),
        if state == 3 { "Ready" } else { "Accepted" }
    )];
    if let Some(row) = guide_row(&guide, &helpers, s, id) {
        let description = text(row, 5).replace(['\r', '\n'], " ");
        // Keep each native chat line bounded while preserving the original description.
        let words = description.split_whitespace().collect::<Vec<_>>();
        let mut line = String::new();
        for word in words {
            if line.len() + word.len() > 180 && !line.is_empty() {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        if !line.is_empty() {
            out.push(line);
        }
    }
    let counts = crate::quests::counts(&db, account, s.character.slot, id)?;
    if let Ok(data) = db.query_row(
        "SELECT data FROM world_reference_rules WHERE table_name='QUEST_MONSTER' AND rule_id=?1",
        [id],
        |r| r.get::<_, String>(0),
    ) {
        let data: serde_json::Value = serde_json::from_str(&data)?;
        for (i, count) in counts.iter().enumerate() {
            let value = &data[format!("sCount{}", i + 1)];
            let required = value
                .as_str()
                .and_then(|s| s.parse::<u32>().ok())
                .or_else(|| value.as_u64().map(|n| n as u32))
                .unwrap_or(0);
            if required > 0 {
                out.push(format!("Monster objective {}: {count}/{required}", i + 1));
            }
        }
    }
    out.push(format!("To abandon this quest: questcancel {id}"));
    Ok(out)
}
pub(crate) async fn handle(
    sock: &mut TcpStream,
    peer: SocketAddr,
    lobby: &Lobby,
    account: &str,
    s: &mut Session,
    raw: &[u8],
) -> bool {
    let Ok(raw) = std::str::from_utf8(raw) else {
        return false;
    };
    let mut parts = raw.trim().trim_start_matches('/').split_whitespace();
    let command = parts.next().unwrap_or("").to_ascii_lowercase();
    if !matches!(command.as_str(), "quests" | "quest" | "questcancel") {
        return false;
    }
    let argument = parts.next();
    let result = if parts.next().is_some() {
        Err(anyhow::anyhow!(
            "Usage: quests [page], quest <ID>, questcancel <ID>"
        ))
    } else if command == "questcancel" {
        let request = argument
            .and_then(|s| s.parse::<u16>().ok())
            .ok_or_else(|| anyhow::anyhow!("Usage: questcancel <ID>"))
            .and_then(|id| {
                crate::npc_scripts::cancellation_request(&lobby.db.lock().unwrap(), account, s, id)
            });
        match request {
            Ok(request) => {
                crate::npc_scripts::handle(sock, peer, lobby, account, s, &request).await;
                let cancelled=lobby.db.lock().unwrap().query_row("SELECT state=4 FROM character_quests WHERE account=?1 AND slot=?2 AND quest=?3",params![account,s.character.slot,argument.unwrap().parse::<u16>().unwrap()],|r|r.get::<_,bool>(0)).unwrap_or(false);
                if cancelled {
                    Ok(vec![format!(
                        "Quest {} abandoned. Other quests are unchanged.",
                        argument.unwrap()
                    )])
                } else {
                    Err(anyhow::anyhow!("This quest could not be abandoned."))
                }
            }
            Err(e) => Err(e),
        }
    } else {
        lines(lobby, account, s, &command, argument)
    };
    let lines = result.unwrap_or_else(|e| vec![e.to_string()]);
    for text in lines {
        let p = crate::chat::packet(7, s.character.nation(), u16::MAX, "", text.as_bytes());
        crate::send_response(sock, peer, &None, &p, "QUEST_JOURNAL").await;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn journal_titles_follow_the_client_helper_guide_column() {
        if !reko_common::game_dir()
            .join("Data/Quest_Helper_us.tbl")
            .exists()
        {
            return;
        }
        let lobby = crate::social::tests::lobby();
        let s = crate::social::tests::player(&lobby, "Guide", 1, 80);
        let guide = table("Quest_guide_us.tbl");
        let helpers = table("Quest_Helper_us.tbl");
        let title = |id| guide_row(&guide, &helpers, &s, id).map(|r| text(r, 3).to_owned());
        // Guide 415 belongs to quest 9; quest flag 415 is the clan promotion.
        assert_eq!(title(9).as_deref(), Some("Guardian of 7 Keys"));
        assert_ne!(title(415).as_deref(), Some("Guardian of 7 Keys"));
        assert_eq!(title(101).as_deref(), Some("Worm hunt"));
    }
    #[tokio::test]
    async fn private_journal_cancels_only_the_selected_owned_active_quest() {
        let lobby = crate::social::tests::lobby();
        let mut s = crate::social::tests::player(&lobby, "Journal", 1, 80);
        let _other = crate::social::tests::player(&lobby, "Other", 1, 80);
        {
            let db = lobby.db.lock().unwrap();
            for id in [5, 6] {
                let helper = serde_json::json!({"sNpcId":"100","sEventDataIndex":id.to_string(),"bQuestType":"1","bNation":"1","bClass":"5"});
                db.execute(
                    "INSERT INTO world_reference_rules VALUES ('QUEST_HELPER',?1,?2)",
                    params![id, helper.to_string()],
                )
                .unwrap();
                db.execute("INSERT INTO world_reference_rules VALUES ('QUEST_MONSTER',?1,'{\"sCount1\":\"10\"}')",[id]).unwrap();
                for account in ["Journal", "Other"] {
                    db.execute(
                        "INSERT INTO character_quests VALUES (?1,0,?2,1)",
                        params![account, id],
                    )
                    .unwrap();
                    db.execute(
                        "INSERT INTO character_quest_kills VALUES (?1,0,?2,1,3)",
                        params![account, id],
                    )
                    .unwrap();
                }
            }
            db.execute_batch(
                "INSERT INTO character_quests VALUES ('Journal',0,7,2),('Journal',0,32000,1);",
            )
            .unwrap();
        }
        let list = lines(&lobby, "Journal", &s, "quests", None).unwrap();
        assert!(list[0].contains("2 active"));
        assert!(
            !list
                .iter()
                .any(|s| s.starts_with("7:") || s.starts_with("32000:"))
        );
        assert!(
            lines(
                &lobby,
                "Journal",
                &s,
                "quests",
                Some("999999999999999999999999999999999")
            )
            .is_err()
        );
        assert!(
            lines(&lobby, "Journal", &s, "quest", Some("5"))
                .unwrap()
                .iter()
                .any(|s| s.contains("3/10"))
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, peer) = listener.accept().await.unwrap();
        assert!(
            handle(
                &mut socket,
                peer,
                &lobby,
                "Journal",
                &mut s,
                b"questcancel 5"
            )
            .await
        );
        assert!(
            handle(
                &mut socket,
                peer,
                &lobby,
                "Journal",
                &mut s,
                b"questcancel 7"
            )
            .await
        );
        let db = lobby.db.lock().unwrap();
        assert_eq!(crate::quests::counts(&db, "Journal", 0, 5).unwrap(), [0; 4]);
        assert_eq!(
            crate::quests::counts(&db, "Journal", 0, 6).unwrap(),
            [3, 0, 0, 0]
        );
        assert_eq!(
            crate::quests::counts(&db, "Other", 0, 5).unwrap(),
            [3, 0, 0, 0]
        );
        assert!(crate::quests::completed(&db, "Journal", 0, 7).unwrap());
        assert!(crate::npc_scripts::cancellation_request(&db, "Journal", &s, 32000).is_err());
        assert!(crate::npc_scripts::cancellation_request(&db, "Journal", &s, 5).is_err());
    }
}

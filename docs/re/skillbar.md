# Server-persisted skillbar shortcuts

Backlog CHAR-03: character-owned shortcut save/load using native opcode79.
This stores UI layout, not learned skill points, equipment or permission to cast.
Records are packed u32 values; their order/encoding is preserved. Native records
encode `(zero_based_bar * 10 + zero_based_slot) * 1,000,000 + skill_id`.
All eight bars, eight slots each, fit the64-record limit. No skill ID filter
removes high-level or quest-unlocked skills. The server stores records unchanged.

Supplied1453 `GameServer/User.cpp::SkillDataProcess/SkillDataSave/SkillDataLoad`
and `DatabaseThread.cpp` define save1/load2. DBAgent::LoadSkillShortcut emits
count:u16 followed by that many u32 entries. Open-KO UIExitMenu contains disabled
save hooks; it does not establish this version's working protocol by itself.

Native1453 evidence:

- Dispatcher0069D281 accepts subcode2, then0069D2BC calls0050F8F0.
- Receiver0050F919..0050F988 reads count:u16 and repeated u32 records.
- Sender0050FB40 sets7901, counts an8x8 slot grid, writes count at0050FB7E,
  packs nonempty shortcut records and sends at0050FBD5.
- The native sender can emit zero entries for an empty bar. This implementation
  permits an empty save to clear the saved layout, despite the private source's
  zero-count rejection. Original native behavior takes precedence.

Wire payloads:

```
79 01 count:u16 entries[count]:u32   save (0..64)
79 02                             load request
79 02 count:u16 entries[count]:u32   load reply
```

`character_hotkeys` stores the blob by account/slot, with character foreign-key
cleanup and a256-byte size limit. Requests require an authenticated entered
character. Saves validate exact lengths/counts and commit atomically; malformed
saves leave the previous layout intact. Save has no speculative acknowledgement,
matching the source. If no server layout has ever been saved, leave the client's
existing local layout in place by returning no load reply. This preserves
pre-feature shortcut arrangements until the first ordinary save. An explicitly
saved empty bar does return a zero-count load reply.

Reconnect and daemon restart preserve the layout. The client must send a save:
exit/log out through its normal menu rather than killing the process. Native
in-game icon restoration still requires verification; logs label replies SKILLBAR_LOAD and
accepted saves/loads record their character slot and entry count. A native exit
capture confirmed Bloody Beast108670 on both bars1 and8 was persisted intact;
the next login received the same15 records byte-for-byte in its load response.
Regression covers all64 slots with native packed records including Bloody Beast,
empty clearing, malformed/trailing requests,
account/slot isolation, initialization again and database reopen.

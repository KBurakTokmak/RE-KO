# Native1453 protocol findings

Widths/layouts below are observed in the unpacked1453 client or regression
captures. Semantic labels for some ranking/tail fields remain inferred. Do not
substitute the1298 or2086 wire format without native verification.

## Framing, login and creation

Packets are `AA55 length:u16 payload 55AA`, little-endian; length excludes the
six framing bytes. The stream decoder handles split/coalesced packets.
Login usesTCP15100, game usesTCP15001. Login F2 exchanges an eight-byte public
key for JvCryption. Private protocol key1257091582190465 is recovered at004624E0;
the shared key is its XOR with the public key. See `crypto.rs` for the cipher
and directional envelope. The current game2B reply uses an all-zero key to
select the client's supported plaintext game-session branch (0075EE10).

Creation02 request: slot:u8, name length:u16/name, race:u8, class:u16, face:u8,
hair:u8, five stat bytes. Native006E6887 serializes it;006E54B3 receives the
single status byte, where **zero means success**. Failure must reply to release
the client's waiting UI. NewChrValue.tbl supplies legal combinations and the
point budget; there is no universal hardcoded300-point total.

Character list0C has success:u8 and three complete records:
name length:u16/name, race:u8, class:u16, level/face/hair/zone:u8, followed by
eight(item:u32,durability:u16). Native006E4FF0 reads fixed fields even for empty
names; each empty slot is57bytes. Selection04 success uses zone:u8,
x/z:u16,y:i16 scaled by0.1 and an environmental byte (0075F270), not weather
floats. Selection/start names and character ownership are validated.

Password registration/failure and nation-choice semantics are documented in
[accounts](accounts.md). Credential packets must not enter hex captures.

## World entry

Native0069BC50 dispatches game opcodes. On matching0D/1 start, send0E full player
data before the one-byte0D acknowledgement. Native0069C368 sets the game-start
flag and sends7902; omitting acknowledgement stalls loading at99percent.
Repeated stage1 requests resend only the acknowledgement. Matching0D/2 confirms
load completion after the snapshot; refresh native0x15 player regions for that
viewer and nearby observers, without another acknowledgement or bootstrap.
The0x15 list includes self plus nearby loaded IDs; original0069C18C/006A5D30
requests missing records through0x16. Individual0x16 records have the same fields
as0x07/1 after its opcode/subcode, with no extra leading byte. Optional6B empty
list is `6B 0000`. The full player parser is0069FA60:

| Field order after0E | Width |
| --- | --- |
| ID, name | i16, u8 length/bytes |
| x,z,y | u16,u16,i16 in tenths |
| nation,race,class,face,hair | u8,u8,u16,u8,u8 |
| rank,title,level,free stats | u8,u8,u8,i16 |
| required/current XP, two loyalty values | fouru32 |
| city,clan,fame,alliance,clan type | u8,i16,u8,i16,u8 |
| clan name,grade,ranking,mark,cape | u8 length/bytes,u8,u8,i16,u16 |
| maximum/current HP, maximum/current MP, maximum/current weight | sixi16 |
| five stats with bonus bytes | tenu8 |
| attack,defense,resistances,coins | i16,i16,sixu8,u32 |
| authority and two ranking bytes | threeu8 |
| learned skills | nineu8 |
| equipment, auxiliary slots, inventory | fourteen19-byte, two15-byte, twenty-eight19-byte entries |
| tail | u8,u8,i16,u8,u32 |

Ordinary item entry: item:u32,durability:i16,count:i16,flag:u8,rental:u16,
nested count:u32, optional nested metadata, remaining time:u32. Auxiliary slots
omit the remaining-time field. Current code emits zero nested metadata.
Anchors006A0CE5/006A0E49/006A1230 cover skills/equipment/inventory.
The auxiliary records are costume helmet then armour (native slots1/4), with
zero flags/rental/nested metadata for permanent equipment. Ordinary equipment
records remain fourteen entries. Other-player07/1 appends costume armour then
helmet as two(item:u32,durability:u16,flag:u8) records after the eight visible
ordinary pieces, then zone:u8 (native006A443D..006A44B4).

## Inventory, resources and objects

Shop21 replies acknowledge the shop's own icon transfers. Normal moves use1F;
do not also send3D deltas for those same moves. Parallel updates can orphan
icons until another NPC rebuilds the inventory. Open-KO UITransactionDlg and
native006B12B0/004E97A0 corroborate this ownership distinction.
Costume moves use the same8-byte1F request: direction7 bag-to-costume or8
costume-to-bag, item:u32, source:u8,destination:u8. Costume positions are1 for
helmet and4 for armour, not compact0/1 storage indices. Success owns the native
icon recovery through the ordinary1F attribute acknowledgement; subsequent54/17/18
updates refresh weight/vitals without duplicate3D inventory deltas. Native004C3C60 has no valid costume3F encoder (only ordinary equipment/bag
districts); unequip into inventory before deleting. Malformed direct deletion
requests fail safely through the existing acknowledgement.
Warehouse45 transfer acknowledgements and anvil5B results likewise own their
window's inventory icons. Do not follow them with3D for the same slots: the
warehouse temporarily owns those icons and the upgrade result already consumes
each material, including Trina. Duplicate updates caused invisible/ghost items
after a Trina upgrade followed by a bank deposit. Socket/SQLite regression now
covers that sequence, complete nonstackable transfers and partial stacks.

Native3D/006AA7A0: entry count:u16, then21-byte entries containing district:u8,
slot:u8,item:u32,quantity:u32,new:u8,durability:u16,two trailingu32 fields.
Keep the trailers even though older source omits them. Weight54 carriesu16.
For kind255 charged consumables, quantity remains the actual stack count while
durability carries remaining uses. A30-use pack updates count1/durability29 on
its first use; count0 is sent only on exhaustion. Do not serialize remaining uses
as stack quantity or remove the whole pack after one skill.
For quest exchanges reusing consumed-item slots, send a zero-count entry for
the old ID before the new item entry. Native004C2816's different-ID branch
replaces the item record without constructing its icon; the next merchant
inventory transfer dereferences that null icon at004E7681. The empty-slot path
constructs the complete icon. Open-KO UIInventory::ItemCountChange corroborates
the distinction. Quest additions/count increases use new=100 for the native
received-item message; removals/decreases use0. Shop/move acknowledgements keep
their separate ownership rules above. This fix is packet/state regression tested;
native-client visual verification remains necessary.
Durability38 carries slot:u8,durability:i16 (006B7950). Item deletion uses the
native reply required to release its inventory recovery/wait state.

HP17: max:u16,current:u16,**attacker ID:i16**, not damage delta (006AAE80).
Send NPC HP loss before its attack-result animation so lethal damage logs.
MP18: max/current:i16 (006AB300). XP1A: subcode0,current:u32 (006AB660).
NP2A: subcode1,loyalty:u32,monthly:u32 (006AC500).
Wallet4A: gain1/loss2:u8,delta:u32,balance:u32 (006BA790);14 is weather.

Region1C lists NPC IDs;1D returns requested visible living details (006A79F0).
Special map objects need their source status as well as the special flag:
anvil event8/status1 reveals its shape; status0 hides it (006A73C6).

## Magic and quests

Magic31 is22bytes: subcode:u8,skill:u32,caster/target:i16,sixi16 data fields.
Stages1/2/3/4 represent cast/projectile/completion/failure. Successful type4
effect needs data[0]=0,data[1]=1;0063F323 checks data[1]. Buff removal is
`31 05 category:u8`; reconnect duration uses data[3] (0063F866).

Quest64 state2: quest:u16,state:u8. Monster snapshot9/1: quest:u16 and fouru8
counts; increment9/2: group:u8,count:u8. Native006C3CB0 has one counter context;
select the quest with a snapshot before an ID-less increment for another quest.
Readiness request64/4 and item-regression64/12 carry helper:u32; native005F4A10
and005F50E0 generate these away from NPCs. See [quest behavior](quests-progression.md).

SelectMsg55 serializes **12** button IDs before its filename, even when only
two are visible. Native00606C10 reads two then skips ten. The bridge supports
ten selectable events and pads two slots. A short menu misaligns the filename
and the client silently suppresses the confirmation request. Keep event choices
scoped to the server-issued dialog.

Warehouse45/1 reply includes bank balance and19217-byte records. Normal upgrade
5B/2 reply has mode/result and ten(item:u32,position:i8) pairs; map effect33/8
includes result and NPC:u16. Native006B92D0/006B94A0/004B01F0 corroborate layouts.
Chaotic exchange5B/5 requests are9bytes: NPC:u16, fragment:u32, source slot:u8
after opcode/subcode (004B4106..004B4130). The14-byte reply contains result:u8,
reward:u32,destination:u8,fragment:u32,source:u8,effect:u8 after opcode/subcode.
Validate/pay the selected source, not a first matching item elsewhere in the bag.
Loot23 binds corpse:u16/bundle:u32;24 returns six(item:u32,count:u16) slots;
26 solo success includes bag slot, item, total count and current coins.

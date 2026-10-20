# Current normal upgrade chances

These are the imported supplied1453 rules, not verified retail rates.
The normal-upgrade handler preserves the source roll algorithm.

The common generic weapon recipe using scroll379021000 has these chances:

| Upgrade | Database rate | Approximate effective chance | With supported +1000 booster |
| --- | ---: | ---: | ---: |
| +1 → +2 | 100% | 100% | 100% |
| +2 → +3 | 100% | 100% | 100% |
| +3 → +4 | 80% | 84.29% | 94.83% |
| +4 → +5 | 70% | 73.76% | 84.29% |
| +5 → +6 | 50% | 52.69% | 63.22% |
| +6 → +7 | 25% | 26.35% | 36.88% |
| +7 → +8 | 5% | 5.28% | 15.81% |
| +8 → +9 | 1% | 1.06% | 11.60% |
| +9 → +10 | 1% | 1.06% | 11.60% |

Other scrolls and item classes have different recipes. Generate a local report
with `python scripts/Export-UpgradeRates.py`. Its output
`docs/re/upgrade-rates.csv` is ignored by Git. A recipe appearing in the report
does not mean accessory or every special upgrade mode is implemented.

## Reverse conversion

The anvil accepts original Reverse Scroll379256000 with eligible normal items.
The actual1453 client sends this conversion as5B/7, not5B/2. Source shared/packets.h
defines ITEM_UPGRADE_REBIRTH=7 and UpgradeHandler::ItemUpgradeRebirth delegates
to ItemUpgrade with that subtype. The server accepts both bounded upgrade paths
and echoes7 for native reverse replies, including failures. Native dispatcher
0069CE3D/table0069D61C maps result subtypes2 and7 to the same004B01F0 parser.
The observed55-byte request contained Shard111210047 and379256000; the former
normal-only dispatch refused it before checking the correct recipe. Regression
now uses that native7 subtype for source poison and plain/Exceptional conversion.
Primary ITEM_UPGRADE conversion recipes retain their costs, result deltas and
10000 rates. Source UpgradeHandler permits ItemClass<=4 for bRateType4; requiring
exactly class4 incorrectly rejected normal high-class weapons/armor before they
could become reverse items. Required materials remain an exact multiset and
ownership, duplicate-slot, range, original-client result and persistence checks
still apply. Conversion does not accept a Trina/booster. Preview consumes nothing.

Source poison weapon recipes map +7/+8/+9/+10 to reverse +1/+5/+11/+21. The owner
also requests plain unenchanted +7 conversion. For original client weapons with
a compatible light reverse variant, the equivalent light recipe is derived from
the source poison recipe (origin suffix minus40, result delta plus10), retaining
the source cost/rate. This extension is an explicit local rule, not a recovered
retail recipe. Both source and derived results must exist as reverse class4
items in the imported metadata and original client; low-class weapons without a
reverse model are refused without consuming anything. No database rows are added.
Accessory upgrades and full reverse-upgrade recipes outside existing supported
item kinds remain separate from this conversion fix.

Evidence: primary `GameServer/UpgradeHandler.cpp::ItemUpgrade` uses
`myrand(0,myrand(9000,10000))`, succeeds when the roll is at most sGenRate,
and adds1000 for Trina, capped10000. The Rust normal-upgrade path matches this.
Thus sGenRate/100 is a table percentage, not precisely the effective probability.
The report averages min(rate+1,upper+1)/(upper+1) for all upper9000..10000;
it assumes uniform draws and does not model finite-PRNG correlations/modulo bias.
Preview always reports success and consumes no resources; it is not a rate test.

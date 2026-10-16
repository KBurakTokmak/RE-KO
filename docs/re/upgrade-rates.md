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
does not mean accessory/rebirth/special upgrade modes are implemented.

Evidence: primary `GameServer/UpgradeHandler.cpp::ItemUpgrade` uses
`myrand(0,myrand(9000,10000))`, succeeds when the roll is at most sGenRate,
and adds1000 for Trina, capped10000. The Rust normal-upgrade path matches this.
Thus sGenRate/100 is a table percentage, not precisely the effective probability.
The report averages min(rate+1,upper+1)/(upper+1) for all upper9000..10000;
it assumes uniform draws and does not model finite-PRNG correlations/modulo bias.
Preview always reports success and consumes no resources; it is not a rate test.

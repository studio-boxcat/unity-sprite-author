# Orgel fixture (tpsheet path)

Canonical fixture for the **per-tpsheet** sprite emit path — the bulk of
sprites produced by `pipeline::generate`. Sourced from
`meow-tower/Assets/21_Collections/OrgelContents/1204/Orgel/` (one atlas
of 62 sprites under the 1204 orgel-event hierarchy).

| File | Source |
| --- | --- |
| `Orgel.{tpsheet, tps, png.meta}` | Shared atlas + sidecar metadata. The `.tpsheet` is committed even though `pipeline::generate` deletes it on success — the test stages the fixture into a temp dir before running. |
| `sprites/*.asset(.meta)` | The 62 committed Unity-emitted goldens, byte-exact targets for the rust emit. |

## Status

- `tests/golden_parity.rs` walks every sprite where the committed
  `.asset`'s `m_PixelsToUnits == 80` (matching the test's `ATLAS_PPU`)
  and asserts byte-exact equality. Eight sprites pass; the other 54
  carry a non-1 `spriteScale` in the current `Orgel.tps` and are skipped
  — see `docs/unity-probes.md#d-non-10-spritescale-fixture-refresh` for
  the refresh procedure.
- `src/render_data.rs` and `src/emit.rs` have inline `cake_decoleft_*`
  tests that pin the typelessdata / index-buffer / full-asset byte
  patterns against this fixture's `Cake__DecoLeft.asset`.
- `benches/pipeline.rs` uses this fixture for every benchmark target.

## Cross-references

- The matching meow-tower-side files live under
  `meow-tower/Assets/21_Collections/OrgelContents/1204/Orgel/`, but the
  `.tpsheet` there is ephemeral (`pipeline::generate` deletes it on
  success). The rust-side copy is what stays stable.
- `Cake__DecoLeft.asset` is the single sprite cited as the canonical
  example throughout `CLAUDE.md`'s "Reference fixture" note.

# unity-sprite-author

Author Unity Sprite `.asset` files **byte-exactly**, outside of Unity, from TexturePacker output.

## Purpose

Unity's TexturePackerImporter is a black box that re-runs on every reimport, slow, non-deterministic across machines/versions, and forces a Unity round-trip into asset-build pipelines. The bytes Unity emits for a `Sprite` `.asset` are fully recoverable from the source artifacts (atlas PNG, tpsheet, importer config) — meaning we can author the `.asset` directly and skip Unity.

The bar is **byte-exactness**: output must equal Unity's output byte-for-byte, so existing `.asset` files in a repo can be replaced without diff churn and downstream consumers (AssetBundle hashes, addressables, `.meta` references) stay stable.

## Goal

**Sprite `.asset` from TexturePacker**:
- Input: `*.tpsheet` (TexturePacker SmartUpdate sheet), atlas `*.png`, importer config (PPU, etc.), source asset `.meta` (for GUIDs)
- Output: one `Sprite` `.asset` per tpsheet entry, byte-identical to what Unity's TexturePackerImporter emits

## Non-Goals

- Other Unity asset types (`.controller`, `.spriteatlasv2`, `Texture2D` reimport, etc.). Sprite-only by design — if scope grows, fork or split.
- Reimplementing Unity's tight-mesh tracer / alpha outline algorithms. We only handle the tpsheet path where geometry is **already in the source artifact** (tpsheet carries verts + tris).
- Editor-time UX. This is a build-pipeline tool — CLI in, files out.
- Cross-version compat across arbitrary Unity versions. Target one version at a time; bump explicitly.

## Pipeline

```
Orgel.tpsheet ─┐
Orgel.png    ──┼─► unity-sprite-author ──► Cake__DecoLeft.asset (byte-exact)
Orgel.png.meta─┤                           Cake__DecoRight.asset
config.toml ───┘                           ...
```

## Reference: tpsheet → Sprite `.asset` field map

tpsheet line format (semicolon-separated, observed):
```
<name>;<x>;<y>;<w>;<h>; <pivotX>;<pivotY>; <bL>;<bR>;<bT>;<bB>;
  <vCount>;<v0x>;<v0y>;...;
  <triCount>;<t0a>;<t0b>;<t0c>;...
```

| Sprite `.asset` field         | Source                                                              |
| ----------------------------- | ------------------------------------------------------------------- |
| `m_Rect`, `textureRect`       | tpsheet rect                                                        |
| `m_Pivot`                     | tpsheet pivot                                                       |
| `m_Border`                    | tpsheet borders (LRTB)                                              |
| `m_PixelsToUnits`             | importer config                                                     |
| `_typelessdata` pos (stream 0)| `(px − w·pivotX)/ppu`, `(py − h·pivotY)/ppu`, `0`  — vec3 f32 LE     |
| `_typelessdata` uv (stream 1) | `(rect.x + px)/atlasW`, `(rect.y + py)/atlasH`     — vec2 f32 LE     |
| `_typelessdata` layout        | stream 0 packed, **padded up to 16-byte boundary**, then stream 1   |
| `m_DataSize`                  | `align16(vCount·12) + vCount·8`                                     |
| `m_IndexBuffer`               | tpsheet triangles, u16 LE                                           |
| `uvTransform`                 | `(ppu, rect.x + w·pivotX, ppu, rect.y + h·pivotY)`                  |
| `settingsRaw`                 | constant from importer (192 = default observed)                     |
| `texture` GUID                | from atlas `.png.meta`                                              |
| `m_RenderDataKey` GUID        | TBD — verify whether it's tpsheet GUID, asset GUID, or derived hash |

Reference asset for parity testing: `meow-tower/Assets/21_Collections/OrgelContents/1204/Orgel/Cake__DecoLeft.asset` (paired with `Orgel.tpsheet`, `Orgel.png`).

## Reference Implementations

A working TypeScript implementation of this exact pipeline already exists in **`prefab-saloon`** (`~/Develop/prefab-saloon`). Port-and-verify, don't redesign.

| File                                      | What's there                                                                                                    |
| ----------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `src/lib/sprite/tpsheet-parser.ts`        | Full `.tpsheet` format docs (header `:format=40300`, sprite line schema) + parser. Mirror the data model.        |
| `src/lib/sprite/generator.ts`             | **Reference impl.** Position/UV formulas, 16-byte stream alignment, vertex channels (14 entries, ch0=pos, ch4=uv), index buffer encoding, `uvTransform`, full Sprite YAML template. |
| `src/lib/sprite/generator.test.ts`        | Test cases — useful for porting test fixtures.                                                                  |
| `src/lib/prefab/parser.ts`                | General Unity YAML parser (handles document streams, `!u!<classId> &<fileId>` headers, flow-style maps).         |
| `src/lib/prefab/serializer.ts`            | General Unity YAML emitter — flow-style for vectors, no-quote conventions. Reference for Phase 2.                |
| `src/lib/prefab/templates.ts`             | Templates for common Unity component types — handy for future asset types.                                       |

Critical details surfaced from the TS impl that I'd missed:
- **Stream alignment**: between stream 0 (positions) and stream 1 (UVs), pad to 16-byte boundary. For 7 verts: posBytes=84 → padded to 96, +uvBytes=56 → `m_DataSize=152` (matches `Cake__DecoLeft.asset`).
- **Vertex channels**: 14-entry array; only ch0 (pos, dim=3, stream=0) and ch4 (uv, dim=2, stream=1) populated; rest are zeroed dim=0 placeholders.
- **`m_RenderDataKey` rendering**: TS impl writes `texture.guid` directly as the key (`<guid>: <fileId>`). Verify this is correct for atlas-packed sprites; the GUID in our `Cake__DecoLeft.asset` (`d4c782eb…`) does not match the texture GUID (`65583bd2…`), so this needs checking.

## Status

The TS reference impl in `prefab-saloon` is **not** byte-exact at the full `.asset` level. Verified status:
- ✓ `m_IndexBuffer` — exact hex match against Unity-emitted reference (4 fixtures).
- ~ `_typelessdata` — float-tolerant match only; bytes likely match for simple int math but not enforced.
- ~ `uvTransform` — float-tolerant match only.
- ✗ Full-file byte equality — no test exists.
- ✗ `m_RenderDataKey` — TS writes texture GUID; observed asset uses a different GUID. Wrong for atlas-packed sprites.
- ✗ `settingsRaw` — hardcoded `192`; never validated against varied importer configs.
- ✗ YAML formatting (indentation, trailing newline, flow-style spacing, key ordering) — Unity has quirks; nothing pins them down.

**Strategy**: lift the mesh-encoding code as-is (proven), then add a real golden test (`assert_eq!(generated_bytes, fs::read("Cake__DecoLeft.asset"))`). Use the diff as the work queue; fix one wrapper field at a time until empty.

## Open Questions

1. `m_RenderDataKey` derivation — confirm against multiple sample assets.
2. `settingsRaw` bit layout — what each bit maps to (filterMode, wrap, mip, colorSpace…).
3. `m_AtlasRD` vs `m_RD` — appear identical in samples; verify under SpriteAtlas usage.
4. YAML emit determinism — Unity uses a specific YAML flavor (flow-style for vectors, no quoting on hex strings). Need a custom emitter; serde_yaml will not match.

## Tech

- Rust (stable). Single binary CLI, library crate underneath for reuse/testing.
- No serde_yaml for output — custom emitter to hit byte-exactness.
- Parity test: golden-file diff against committed Unity-emitted `.asset` samples.

## Layout (planned)

```
unity-sprite-author/
├── src/
│   ├── main.rs          # CLI
│   ├── lib.rs
│   ├── tpsheet.rs       # parser
│   ├── sprite.rs        # Sprite asset model + emitter
│   ├── meta.rs          # .meta GUID lookup
│   └── yaml.rs          # Unity-flavor YAML emitter
├── tests/
│   └── golden/          # paired .tpsheet + .png.meta + expected .asset
└── CLAUDE.md
```

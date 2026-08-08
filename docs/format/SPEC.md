# The .cull Package Format — Specification v1

**Canonical document.** This repository owns the format; the reference implementation is
`crates/glyphcull-format`. The JS and Rust runtimes implement *independent* readers from this
specification, which is how the contract is validated.

- Status: **v1 — locked** (hardening pass, 2026-08-07): the v1 byte layout, the reader
  rules (§1.6), and the compatibility policy (§4) are the contract; changes within v1
  and changes requiring v2 are defined in §4.8 and recorded in §7.
- Byte order: **little-endian** throughout. All integers are unsigned unless stated.
- All text is UTF-8. All text content is NFC-normalized.
- Version: 1 (single u16; any incompatible change requires a new version number).

## 1. Container overview

```
┌─────────────────────────────┐
│ Header (16 bytes)           │
├─────────────────────────────┤
│ Section table (N × 32)      │
├─────────────────────────────┤
│ Section 0 bytes             │
│ Section 1 bytes             │
│ ...                         │
└─────────────────────────────┘
```

A package is a sequence of sections. Each section is self-describing via the table:
its kind, compression, byte offset, stored length, decoded length, and CRC-32 of the
**decoded** payload. Sections appear in file order; the table order equals file order.

### 1.1 Header — 16 bytes

| Offset | Size | Field | Notes |
|---|---|---|---|
| 0 | 4 | `magic` | ASCII `CULL` (`43 55 4C 4C`) |
| 4 | 2 | `version` | Format version; currently `1` |
| 6 | 2 | `flags` | Reserved; must be `0` in v1. Readers ignore unknown bits. |
| 8 | 4 | `section_count` | Number of section entries; `1..=64` |
| 12 | 4 | `header_crc32` | CRC-32 (IEEE, zlib polynomial) over bytes `0..12` |

### 1.2 Section table entry — 32 bytes

| Offset | Size | Field | Notes |
|---|---|---|---|
| 0 | 4 | `kind` | Section kind (see §2) |
| 4 | 1 | `compression` | `0` = none, `1` = zlib (deflate, level 9) |
| 5 | 1 | `flags` | Bit 0 = `critical` (meaningful only for unknown kinds, §1.2.1); bits 1–7 reserved, must be `0` |
| 6 | 2 | `reserved` | Must be `0` |
| 8 | 8 | `offset` | Absolute byte offset of the stored payload |
| 16 | 8 | `stored_len` | Byte length of the stored payload as written |
| 24 | 4 | `decoded_len` | Byte length after decompression (`stored_len` when uncompressed) |
| 28 | 4 | `crc32` | CRC-32 over the **decoded** payload bytes |

### 1.2.1 The critical bit and unknown sections

Bit 0 of a section entry's `flags` byte is the **`critical`** bit. It is meaningful only on
sections whose kind is not defined in the current version:

- **Unknown kind, critical bit clear** — a forward-compatible, ignorable extension.
  Readers MUST skip it (never interpret it) and MUST NOT reject the package for its presence.
- **Unknown kind, critical bit set** — an extension this reader cannot honor. Readers MUST
  reject the package with an `unknown-critical-section` error.
- **Known kind, any flags set** — reserved in v1; readers MUST reject.
- Reserved bits 1–7 — always reserved; readers MUST reject.

Writers in v1 emit `flags = 0` for every section. The bit exists so that a future version can
ship a mandatory extension without breaking old readers silently: the old reader either skips
it (noncritical) or rejects the package loudly (critical).

### 1.3 Limits (v1)

| Limit | Value |
|---|---|
| `section_count` | ≤ 64 |
| single section `decoded_len` | ≤ 2 GiB |
| total decoded size | ≤ 4 GiB |
| file size | ≤ 4 GiB |
| chunk count | ≤ 2^28 (268,435,456) |
| style count | ≤ 2^24 |
| content payload count | ≤ 2^24 |
| atlas page dimension | ≤ 8192 texels |
| glyph count per atlas | ≤ 2^16 (65,536) |
| kerning pairs per atlas | ≤ 2^24 |
| image count | ≤ 2^20 |
| image dimension | ≤ 16384 px |

### 1.4 Section kinds

| kind | Name | Purpose | Compressed by writer |
|---|---|---|---|
| 1 | `INFO` | Metadata (deterministic) | zlib |
| 2 | `CHNK` | Chunk graph | zlib |
| 3 | `STYL` | Resolved style table | zlib |
| 4 | `CONT` | Content payloads (text, image refs) | zlib |
| 5 | `GLYF` | MSDF glyph atlases | none |
| 6 | `IMGS` | Raster images (decoded pixels) | none |
| 7 | `SEAL` | Content hash tree (integrity) | none |

Unknown kinds: readers MUST skip them (they are addressable via the table) and MUST NOT
interpret them, unless the entry's `critical` bit is set — a **critical unknown section is
rejected** (§1.2.1). At most one section per kind (duplicates are an error). Writers MUST emit
sections in canonical order: `INFO, CHNK, STYL, CONT, GLYF, IMGS, SEAL`; readers MUST reject
known sections in any other relative order (§1.6).

### 1.5 Compression

- zlib wrapper (RFC 1950), deflate, fixed level 9, fixed strategy; byte-deterministic.
- `decoded_len` is authoritative: a decoder MUST reject streams that decode to more or fewer
  bytes than `decoded_len`.
- Readers MUST verify the two-byte zlib header (`CMF & 0x0F == 8` and
  `(CMF << 8 | FLG) % 31 == 0`) and the trailing Adler-32 checksum against the decoded
  output (RFC 1950 §2.2/§2.3). Note: the reference implementation verifies the Adler-32
  explicitly because the underlying deflate library does not; truncating the stored stream
  must never decode silently, even when the decoded prefix is identical (the container
  CRC-32 covers decoded content; the Adler-32 check additionally protects the stored form).

### 1.6 Reader rules (normative)

A conforming reader MUST:

1. Validate magic and version; reject others with a typed error.
2. Validate the header CRC-32.
3. Validate `section_count` ≤ 64 and that the table fits within the file.
4. For every entry: `offset + stored_len ≤ file_size` with overflow-checked arithmetic;
   `compression ∈ {0,1}`; `decoded_len ≤ 2 GiB`; flags per §1.2.1 (reserved bits zero;
   known kinds carry no flags; critical unknown kinds rejected at §1.6.5).
5. Decode each payload (decompressing if flagged) and verify its CRC-32.
6. Verify that a decoded stream's length equals `decoded_len`.
7. Reject duplicate section kinds.
8. **Reject known sections in non-canonical relative order** — the known kinds must appear
   in strictly increasing kind order (`INFO, CHNK, STYL, CONT, GLYF, IMGS, SEAL`); unknown
   kinds may appear anywhere. Rationale: canonical serialization is part of the v1 contract
   (writers always emit it, §3), so a conforming reader enforces it; a package that violates
   it was not produced by a conforming writer.
9. **Reject a package without an INFO section** (`missing-required-section`): INFO is the
   one container-required section. (The other sections are required per the INFO counts —
   see §2.1 — enforced by the semantic layer.)
10. Never panic. Every failure is a typed error with a precise variant.

The SEAL section additionally cross-checks every section's content hash (see §2.7).

## 2. Section payloads

### 2.1 INFO — metadata (kind 1)

Deterministic JSON, single object, keys sorted lexicographically, no whitespace, minimal
escaping. Text values are UTF-8. All values are derived from content or configuration —
**no timestamps, no wall-clock data**.

| Key | Type | Notes |
|---|---|---|
| `format_version` | number | Must equal header version (`1`) |
| `generator` | string | Compiler name, e.g. `"glyphcull-compiler"` |
| `generator_version` | string | Semantic compiler version |
| `source_digest` | string | Hex SHA-256 of the normalized source input(s) |
| `document_id` | string | Hex: first 16 bytes of SHA-256 over the decoded content sections `CHNK, STYL, CONT, GLYF, IMGS` in canonical order (INFO and SEAL excluded — the id is computed before INFO is finalized, so including them would be circular). Content-addressed; deterministic. |
| `title` | string (optional) | Document title |
| `lang` | string (optional) | BCP 47 language tag |
| `chunk_count` | number | CHNK record count |
| `style_count` | number | STYL record count |
| `content_count` | number | CONT payload count |
| `atlas_count` | number | GLYF atlas count |
| `image_count` | number | IMGS image count |

JSON number encoding: integers only, no exponents. String escaping: `"` → `\"`, `\` → `\\`,
control chars → `\u00XX`.

### 2.2 CHNK — chunk graph (kind 2)

```
u32 chunk_count                       — records follow
ChunkRecord[chunk_count]              — fixed 44-byte records
u32 extra_count                       — extras follow
ChunkExtra[extra_count]               — variable
```

**ChunkRecord — 44 bytes:**

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | `id` (u32; unique; **1-based** dense in document order; 0 = "none" sentinel) |
| 4 | 1 | `kind` (see below) |
| 5 | 1 | `flags` (see below) |
| 6 | 2 | `reserved` = 0 |
| 8 | 4 | `style_id` (0 = document default style) |
| 12 | 4 | `parent_id` (0 = none) |
| 16 | 4 | `prev_id` (0 = none) |
| 20 | 4 | `next_id` (0 = none) |
| 24 | 4 | `first_child_id` (0 = none) |
| 28 | 4 | `last_child_id` (0 = none) |
| 32 | 4 | `content_index` (1-based index into CONT; `0` = none) |
| 36 | 4 | `ordinal` (u32; dense, 0-based, document order) |
| 40 | 4 | `depth` (root = 0) |

**Chunk kinds (`kind`):**

| value | kind | renderable? |
|---|---|---|
| 1 | `document` | structural |
| 2 | `heading1` | renderable |
| 3 | `heading2` | renderable |
| 4 | `heading3` | renderable |
| 5 | `heading4` | renderable |
| 6 | `heading5` | renderable |
| 7 | `heading6` | renderable |
| 8 | `paragraph` | renderable |
| 9 | `quote` | renderable |
| 10 | `list` | structural |
| 11 | `list_item` | renderable (marker) |
| 12 | `code_block` | renderable |
| 13 | `table` | structural |
| 14 | `table_row` | structural |
| 15 | `table_cell` | renderable |
| 16 | `image` | renderable |
| 17 | `caption` | renderable |
| 18 | `run` | renderable (inline) |
| 19 | `link` | renderable (inline) |
| 20 | `br` | renderable (inline) |
| 21 | `hr` | renderable |

**Chunk flags (bitmask, byte 5):**

| bit | name | meaning |
|---|---|---|
| 0 | `hidden` | excluded by semantic culling |
| 1 | `keep_with_next` | layout hint: avoid break between this chunk and next |
| 2 | `break_before` | layout hint: force break before this chunk |
| 3 | `no_wrap` | suppress line wrapping (code) |
| 4 | `structural` | no direct geometry (document/list/table/row) |

**ChunkExtra:**

```
u32 chunk_id
u8  kind       — 1 = link_target, 2 = cell_span, 3 = list_item_value, 4 = image_alt
u8  flags      — reserved = 0
u16 length
bytes data     — kind-specific (below)
```

- `link_target`: `u16 url_len`, UTF-8 URL bytes.
- `cell_span`: `u16 colspan` (≥1), `u16 rowspan` (≥1).
- `list_item_value`: `u32` explicit ordinal value for ordered lists (0 = auto).
- `image_alt`: UTF-8 alt text (used by selection/copy and accessibility).

**Chunk graph invariants (normative for writers; validated by `cull validate`):**

- Exactly one root (`document`), `depth == 0`, parent 0.
- Tree links form a forest with a single root; no cycles; every node reachable.
- `first_child`/`last_child` and `prev`/`next` are mutually consistent (full sibling ring
  consistency: walking next from first reaches last in `child_count` steps).
- `depth == parent.depth + 1` for every non-root chunk.
- `ordinal` values are exactly `0..chunk_count` in document order; `id` values likewise.
- `style_id` resolves in STYL; `content_index` resolves in CONT when non-zero
  (`content_index` is 1-based: payload id = `content_index − 1`).
- `image` chunks must have a CONT payload of kind `image_ref`.
- `table` children are `table_row` chunks and **at most one `caption` chunk** (the
  HTML table caption), which must precede the rows. Both runtimes lay the caption
  out as a text block above the rows and advance the table origin by its height.
- Renderable chunk kinds never have renderable ancestors of kind `run`/`link`/`br`
  (inline kinds nest only under renderable block kinds or other inline kinds).

### 2.3 STYL — resolved style table (kind 3)

```
u32 style_count
StyleRecord[style_count]              — each: fixed header + property blob
```

**StyleRecord:**

```
u32 id
u16 property_count
u16 blob_len
bytes blob                            — property_count × Property, tightly packed
```

**Property:**

```
u16 tag
── value (fixed size per tag) ──
```

| tag | name | value size | value type | default |
|---|---|---|---|---|
| 1 | `font_id` | 4 | u32 (index into GLYF atlases) | 0 |
| 2 | `font_size_px` | 4 | f32 | 16.0 |
| 3 | `line_height` | 4 | f32 (multiplier of font size) | 1.5 |
| 4 | `font_weight` | 2 | u16 | 400 |
| 5 | `italic` | 1 | u8 0/1 | 0 |
| 6 | `color` | 4 | u32 RGBA | 0x000000FF |
| 7 | `background_color` | 4 | u32 RGBA | 0x00000000 |
| 8 | `margin_top` | 4 | f32 (px) | 0.0 |
| 9 | `margin_bottom` | 4 | f32 (px) | 0.0 |
| 10 | `text_align` | 1 | u8: 0 start, 1 center, 2 end, 3 justify | 0 |
| 11 | `text_indent` | 4 | f32 (px) | 0.0 |
| 12 | `list_style` | 1 | u8: 0 none, 1 disc, 2 circle, 3 square, 4 decimal, 5 lower_alpha, 6 upper_alpha, 7 lower_roman, 8 upper_roman | 0 |
| 13 | `code` | 1 | u8 0/1 | 0 |
| 14 | `underline` | 1 | u8 0/1 | 0 |
| 15 | `letter_spacing` | 4 | f32 (px) | 0.0 |
| 16 | `white_space` | 1 | u8: 0 normal, 1 pre, 2 nowrap | 0 |

A StyleRecord need not contain all properties: absent properties take the documented
defaults above. Style `0` is the implicit document default (all defaults). Properties are
emitted in tag order (determinism). Unknown tags are an error.

### 2.4 CONT — content payloads (kind 4)

```
u32 payload_count
Payload[payload_count]                — fixed 12-byte header + data
```

**Payload:**

```
u32 id
u8  kind      — 0 = text_utf8, 1 = image_ref
u8  flags     — reserved = 0
u16 reserved  — 0
u32 byte_len
bytes data    — text: UTF-8 (NFC); image_ref: u32 image id (into IMGS)
```

Text payloads are the raw text of a chunk (paragraph, run, heading, code block, caption,
alt text). Whitespace policy is the writer's; readers preserve bytes. `image_ref` data is a
single little-endian u32. Payload ids are dense `0..payload_count` in emission order.

### 2.5 GLYF — MSDF glyph atlases (kind 5)

```
u32 atlas_count
Atlas[atlas_count]
```

**Atlas:**

```
u32 font_id
u32 glyph_count
u16 page_count
u8  format          — 0 = MSDF_RGBA8
u8  flags           — reserved = 0
u16 padding         — texels of padding around each glyph box
u32 texels_per_em   — fixed-point ×1024 (e.g., 32768 ⇒ 32 texels/em)
f32 ascent          — em units (typographic ascent)
f32 descent         — em units (positive number; baseline = 0, descent below baseline)
f32 line_gap        — em units
f32 cap_height      — em units
f32 x_height        — em units
f32 units_per_em    — font units per em
u16 family_len
bytes family        — UTF-8 family name
u16 weight          — 100..=900
u8  italic          — 0/1
u8  reserved        — 0
u32 page_width      — texels
u32 page_height     — texels
GlyphRecord[glyph_count]              — 32 bytes each
u32 kerning_count
KerningPair[kerning_count]            — 12 bytes each
Page[page_count]                      — page_width × page_height × 4 bytes each
```

**GlyphRecord — 32 bytes:**

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | `codepoint` (u32; unique within atlas) |
| 4 | 4 | `advance` (f32, em) |
| 8 | 4 | `bearing_x` (f32, em) |
| 12 | 4 | `bearing_y` (f32, em) |
| 16 | 2 | `box_x` (u16, texels, page-space) |
| 18 | 2 | `box_y` (u16, texels, page-space) |
| 20 | 2 | `box_w` (u16, texels; ≥1) |
| 22 | 2 | `box_h` (u16, texels; ≥1) |
| 24 | 2 | `page_index` (u16) |
| 26 | 1 | `glyph_flags`: bit0 `no_outline` (space/combining), bit1 `combining` (advance 0) |
| 27 | 1 | `reserved` = 0 |
| 28 | 4 | `reserved` = 0 |

**KerningPair — 12 bytes:** `u32 left_codepoint`, `u32 right_codepoint`, `f32 adjust` (em;
added to the sum of advances). Pairs are sorted by (left, right) — determinism.

**Page:** raw RGBA8 texels, row-major, top-to-bottom. MSDF semantics: each texel's R, G, B
channels are three signed-distance fields with different edge tie-breaking; A is 255.
Distance values are stored as texels: the glyph edge maps to channel value 0.5; distance in
texels = `channel − 0.5`. Reconstruction (normative): coverage at a sample = median of the
three channels mapped through a smoothstep with screen-space width (see GLOSSARY:
"MSDF reconstruction"). The runtime derives the mapping from `texels_per_em`, the target
font size, and the device pixel ratio.

**Sign convention (normative, canonical — every atlas generator, shader, CPU reference
compositor, and test fixture MUST agree):**

```text
MSDF channel value < 0.5  = outside glyph
MSDF channel value == 0.5 = glyph edge
MSDF channel value > 0.5  = inside glyph
```

The generator stores positive distance toward the glyph interior (inside → `0.5 + dist`,
outside → `0.5 − dist`). Every decoder maps `median − 0.5` to signed screen-space distance
with positive = inside; no subsystem may invert this at render time to compensate for a
non-conforming atlas (a conforming atlas never needs it).

Glyph boxes include `padding` texels of SDF margin on every side; boxes never overlap and
never cross page bounds. `box_w/box_h` count the full box (glyph + padding).

### 2.6 IMGS — raster images (kind 6)

```
u32 image_count
Image[image_count]
```

**Image:**

```
u32 id
u16 width
u16 height
u8  format          — 0 = RGBA8, 1 = RGB8
u8  flags           — reserved = 0
u32 byte_len        — must equal width × height × bpp
bytes data          — raw pixels, row-major, top-to-bottom, no padding
```

The compiler decodes source PNG/JPEG at compile time; runtimes upload raw pixels and never
decode image formats.

### 2.7 SEAL — integrity hash tree (kind 7)

```
u8  mode            — 1 = hash tree (v1)
u8  algo            — 0 = SHA-256
u8  flags           — reserved = 0
u8  reserved        — 0
u32 section_count   — number of covered sections
SectionHash[section_count]            — 36 bytes each
bytes overall_hash  — 32 bytes
```

**SectionHash — 36 bytes:** `u32 kind`, then 32 bytes SHA-256 of that section's **decoded**
payload. `overall_hash` = SHA-256 over: header bytes `0..12`, then for every covered
section in canonical order: `u32 kind` (LE), `u32 decoded_len` (LE), and the decoded
payload bytes.

This definition is deliberately **non-circular**: the table entries contain offsets and
lengths that depend on the SEAL section's own size, so the raw table bytes cannot be
covered without circularity. Table integrity is instead guaranteed structurally (every
entry carries its own CRC-32 over its decoded payload, and bounds are validated), and the
overall hash binds header identity, section kinds, sizes, and content. The SEAL section
itself is not covered by its own hash.

Verification: recompute per-section hashes and the overall hash; mismatch ⇒ integrity
failure. Signature support (authenticity) is a reserved future extension of SEAL.

## 3. Determinism (normative for writers)

- No timestamps, no randomness, no environment-dependent values.
- All emission in canonical order (sections, records by id, properties by tag, kerning by
  key, JSON keys sorted).
- Fixed compression level and strategy.
- Text normalized (NFC) before emission.
- Consequence: identical input + identical compiler version ⇒ identical bytes.

## 4. Compatibility policy (normative)

This section is the format's compatibility contract: what may change within v1, what
requires v2, and what every reader/writer MUST do. It consolidates the normative rules of
§1–§3 into one policy; where a rule is stated here and in §1–§3, both statements bind.

### 4.1 Format versioning

- The header `version` field is the package's format version; it is currently `1`
  (§1.1). The version number changes whenever the byte layout changes incompatibly —
  the version is the compatibility boundary, not a build counter.
- A package's `format_version` inside INFO (§2.1) must equal the header version; a
  mismatch is a malformed package (rejected).
- Readers MUST reject any `version != 1` in a v1 reader with `unsupported-version`
  (§1.6 rule 1). A v2 reader that can read v1 packages is required to do so (§4.7);
  this spec documents v1 only.

### 4.2 Reader behavior (normative)

A conforming v1 reader MUST implement §1.6 verbatim: validate magic, version, header
CRC, table bounds, per-entry bounds and flags, decode + CRC each payload, enforce
canonical order and the required INFO section, verify SEAL when present, and never
panic — every failure is a typed error with a precise variant. Readers enforce the
limits of §1.3 and the rejection rules of §4.10 before interpreting any payload.

### 4.3 Writer behavior (normative)

A conforming v1 writer MUST:

- emit exactly the canonical section order `INFO, CHNK, STYL, CONT, GLYF, IMGS, SEAL`
  (§1.4), each section kind at most once;
- emit `flags = 0` for every section in v1 (no reserved bits, no critical bit on known
  kinds — §1.2.1);
- emit `compression` per §1.5 (zlib for INFO/CHNK/STYL/CONT; none for GLYF/IMGS/SEAL)
  with the fixed level/strategy and a correct `decoded_len`;
- write INFO (required), CHNK/STYL/CONT/GLYF per the INFO counts, IMGS when the
  document has images, and SEAL when the writer ships integrity (writers SHOULD always
  ship SEAL — see §2.7 and §5);
- produce deterministic bytes per §3, and validate its own output against the
  rejection rules of §4.10 before emitting (a writer never emits a package its own
  reader rejects).

### 4.4 Required and optional sections

- **Required at the container**: `INFO` (kind 1) — a package without it is rejected
  (`missing-required-section`, §1.6 rule 9).
- **Required per the INFO counts**: `CHNK` (`chunk_count` > 0), `STYL`
  (`style_count` > 0), `CONT` (`content_count` > 0), `GLYF` (`atlas_count` > 0) —
  the semantic layer enforces consistency between the counts and the sections
  present (§2.1).
- **Optional**: `IMGS` (absent for image-free documents; `image_count` must be 0 then)
  and `SEAL` (verification is mandatory when present; absence means no integrity
  check beyond the per-section CRC-32s).
- Unknown kinds are never "required" by a v1 reader (they are skipped or rejected per
  §1.2.1); a future writer that needs a mandatory extension must use the critical bit
  or bump the version (§4.8).

### 4.5 Unknown section handling; critical vs noncritical extensions

Bit 0 of the section entry `flags` byte is the `critical` bit (§1.2.1), meaningful only
on unknown kinds:

| Unknown kind, flags bit 0 | Reader action | Example |
|---|---|---|
| clear (noncritical) | MUST skip (never interpret), MUST NOT reject for its presence | `future-minor` conformance fixture |
| set (critical) | MUST reject with `unknown-critical-section` | `unknown-critical-section` corpus entry |

Known kinds with any flags set, and reserved flag bits 1–7, are malformed (rejected).
Noncritical sections are addressable through the table, so a skipped section never
invalidates offsets or lengths; the SEAL `overall_hash` covers sections by kind
+ decoded payload, so an added noncritical section simply adds a covered entry
(§2.7).

### 4.6 Forward compatibility

"Forward" = a newer package read by an older reader.

- An older v1 reader MUST load, unchanged, any v1 package whose only additions are
  **noncritical unknown sections** — this is the "future minor-compatible package"
  (the `future-minor` fixture proves it: its canonical model equals the minimal
  fixture's).
- An older v1 reader MUST reject, loudly and with a typed error, any package carrying
  a **critical unknown section**: silently dropping a mandatory extension is worse
  than failing (§4.8).
- An older reader MUST reject `version != 1` (§4.1) rather than guess at the layout.
- Unknown property tags inside a known StyleRecord are an error in v1 (strict — §4.8).

### 4.7 Backward compatibility

"Backward" = an older package read by a newer reader.

- A v1 package is the floor: a v1 reader accepts all v1 packages; a v2 reader MUST
  accept every v1 package exactly as a v1 reader does (v1 is never orphaned).
- New versions MUST NOT reinterpret bytes that a v1 reader accepted (no silent
  semantic change to known sections).
- Within v1, a reader that understands a previously-unknown noncritical section MAY
  interpret it; a reader that does not MUST skip it (§4.5). No reader may fail a v1
  package for carrying sections it merely does not know.

### 4.8 What may change within v1; what requires v2

| Change | Within v1 | Requires v2 |
|---|---|---|
| New **noncritical** section kind (unknown to old readers, skipped) | Allowed — the forward-compatibility mechanism | No |
| New **critical** section kind | No | Yes — old readers reject it rather than drop semantics |
| New property tag inside a known StyleRecord | No (v1 is strict: unknown tags are an error) | Yes |
| New StyleRecord/CHNK/CONT/GLYF/IMGS field, changed width, or reordered records | No | Yes |
| New compression method code | No | Yes (§1.5; readers reject unknown codes) |
| Relaxed/raised §1.3 limits | No — limits are part of the v1 contract | Yes |
| Changed canonical order / required-section set | No | Yes (§1.6 rules 8–9) |
| SEAL mode/algo change | No (mode 1, algo 0 only) | Yes (§2.7) |
| Reserved flag bit repurposed | No | Yes (bits 1–7 are rejected today, §1.2.1) |
| Bug fixes that change emitted bytes but not the layout | Allowed — recorded in §7; packages stay valid, readers stay able to load both | No |

Every within-v1 change MUST be recorded in §7 (History) with the date and the
conformance fixture that proves it — the extension register (§6) tracks reserved and
candidate kinds. Within-v1 changes MUST NOT break determinism (§3) or the canonical
model.

### 4.9 Canonical serialization and determinism

- Canonical serialization is part of the v1 contract: canonical section order (§1.4),
  records by id, properties by tag, kerning by key, sorted JSON keys (§3). Writers MUST
  emit it; readers MUST reject known sections in any other relative order (§1.6 rule
  8). Rationale: determinism (§3) is only enforceable if non-canonical order is
  malformed, not merely discouraged.
- Determinism is normative for writers (§3): identical input + identical compiler
  version ⇒ identical bytes. No timestamps, randomness, or environment-dependent
  values — INFO's metadata is derived from content or configuration only (§2.1). The
  conformance suite regenerates every artifact and asserts byte-stability.

### 4.10 Rejection requirements

A conforming reader MUST reject, with a typed error and a precise cause, and never
crash:

- bad magic, unsupported version, header-CRC mismatch (§1.6 rules 1–2);
- table/entry bounds violations, `offset + stored_len > file_size` (overflow-checked),
  unknown compression codes, `decoded_len` over the §1.3 cap, reserved/flag
  violations, critical unknown sections (§1.6 rules 3–4, §1.2.1);
- payload decode failures: zlib header/Adler-32/CRC-32 mismatch, decoded length ≠
  `decoded_len` (§1.5, §1.6 rule 5–6);
- duplicate section kinds, non-canonical known-section order, missing INFO (§1.6
  rules 7–9);
- SEAL mismatch when SEAL is present (§2.7).

The rejection class per case is fixed (the conformance suite commits the reference
class per hostile entry in `expected/*.reject.json` and asserts all three readers
agree).

### 4.11 Security and resource limits

- §1.3 limits are enforced by every reader **before** interpretation (bounds, CRC,
  length caps); a hostile package cannot force unbounded allocation or oversized
  decode (§5).
- The package format carries no executable content, no paths, no source comments, no
  timestamps (§3, §5); SEAL verification is mandatory when present; signature support
  is a reserved SEAL extension (§2.7).

### 4.12 Compression, checksum, and seal rules

- Compression: zlib wrapper (RFC 1950), deflate, fixed level 9 / fixed strategy;
  `decoded_len` authoritative; two-byte zlib header and trailing Adler-32 verified
  (§1.5). Unknown compression codes rejected (§4.10).
- Checksums: header CRC-32 over bytes `0..12` (§1.1); per-section CRC-32 over the
  **decoded** payload (§1.2); Adler-32 within each zlib stream (§1.5).
- SEAL: content hash tree (mode 1, SHA-256) with per-section hashes and the
  non-circular overall hash (§2.7); verification mandatory when present; mismatch ⇒
  `seal-mismatch` rejection.

### 4.13 Experimental sections

- Experimental kinds are authored as **noncritical** sections (bit 0 clear) so no v1
  reader is harmed by their presence, and are registered in §6 with a status;
  a noncritical section that becomes mandatory semantics MUST be re-filed as
  critical — which requires v2 (§4.8).
- Reserved ranges (section kinds 8..=31, property tags 17..=255) are not
  experimental playgrounds: writers MUST NOT emit them without a §7 record and a
  conformance fixture.

## 5. Security notes

Summarized from SECURITY.md: readers validate structure before interpretation (bounds,
CRC, length caps), never panic, enforce the limits of §1.3, and treat SEAL verification as
mandatory when present (the full rejection set is §4.10 and the resource limits are §4.11).
Compilers never embed source paths, comments, or timestamps.

## 6. Extension register

| Kind / tag | Status |
|---|---|
| Section kinds 8..=31 | reserved |
| Property tags 17..=255 | reserved |
| `IDXM` (search index) | future candidate |
| SEAL signature payload | future candidate |

## 7. History

- v1 draft: Phase 0, canonical definition established.
- v1, Phase 1 (implementation-driven corrections, all reflected above):
  - Chunk ids are **1-based dense** (`id = record index + 1`); `0` is the "none"
    sentinel in every chunk reference field (`parent_id`, `prev_id`, `next_id`,
    `first_child_id`, `last_child_id`, extra `chunk_id`). The draft used 0-based ids
    with `0` as sentinel, which made id 0 (the document root) ambiguous.
  - `content_index` is a **1-based index** into CONT payloads (`0` = none); payload id
    = `content_index − 1`. The draft's 0-based form made payload 0 unreachable.
  - `SEAL.overall_hash` uses the non-circular definition above (header identity +
    kind + decoded_len + payload per covered section) instead of covering raw table
    bytes, which would be circular.
  - `font_weight` valid range clarified to `100..=900` (inclusive).
  - Reader rule: zlib header and trailing Adler-32 verification are mandatory (§1.5).
  - Golden reference implementation, golden byte vectors, and the strict reader corpus
    established; the JS and Rust runtimes will implement independent readers against
    this finalized spec.
- v1, Phase 2 (compiler pipeline, implementation-driven clarifications):
  - `document_id` defined non-circularly: SHA-256 over the decoded content sections
    `CHNK, STYL, CONT, GLYF, IMGS` (INFO/SEAL excluded — see §2.1).
- v1, hardening pass (2026-08-07, compatibility rules locked):
  - Section entry `flags` byte: bit 0 defined as `critical` (unknown-kind semantics,
    §1.2.1); bits 1–7 stay reserved.
  - Canonical known-section order is now a reader requirement (§1.6 rule 8) — a reversal
    of the Phase-1 note that readers "must not depend on order"; recorded here per §4.
  - INFO defined as the container-required section (rule 9).
  - §4 expanded into the full compatibility policy: versioning (§4.1), reader/writer
    behavior (§4.2/§4.3), required/optional sections (§4.4), unknown-section handling
    (§4.5), forward/backward compatibility (§4.6/§4.7), the explicit within-v1 vs v2
    change table (§4.8), canonical serialization + determinism (§4.9), rejection
    requirements (§4.10), security/resource limits (§4.11), compression/checksum/seal
    rules (§4.12), and experimental-section rules (§4.13).
  - Compatibility conformance fixtures committed (H2/H6): `future-minor` (unknown
    noncritical section loads unchanged), `unknown-critical-section`, `bad-version`,
    `bad-compression`, `oversized-section`, `bad-crc`, `bad-seal` — the nine-case
    v1-compatibility matrix proven across all three readers (see
    `glyphcull-demo/conformance/`).
- v1, 2026-08-08 (table captions — implementation-driven clarification):
  - The chunk-graph invariants (§2.2) now state that a `table` chunk holds
    `table_row` children and **at most one `caption` child**, which must precede
    the rows; both runtimes lay it out above the rows and advance the table
    origin by its height. The writer (HTML `<caption>`) and the validator
    (`cull validate`) agree; previously the parser mapped `figcaption` but not
    `<caption>`, which compiled invalid packages.

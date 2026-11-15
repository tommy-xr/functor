# Overlay font

The headset overlay's text (`mod.rs`): a multi-channel signed distance field
atlas rendered from **Inter 4.1** (Regular and SemiBold) by The Inter Project
Authors, under the SIL Open Font License 1.1 (`OFL-Inter.txt`). No font file
ships; the atlas and metrics are derived from it.

| File | What |
| --- | --- |
| `atlas.png` | RGB MSDF, 32 px per em, 4 px range |
| `metrics.rs` | per-glyph advances and boxes, pair kerning (from GPOS) |
| `OFL-Inter.txt` | the license, copied from the release |

Glyphs: U+0020–007E, U+00A0–00FF, dashes, typographic quotes, bullet,
ellipsis, guillemets, euro, arrows, minus, check mark, tabular digits, and
the `.notdef` box drawn for anything else. Lowercase `l` is Inter's tailed
form (`cv05`), to tell it from `I` and `1`.

Regenerate (fetches the pinned release, checks its SHA-256):

```sh
tools/overlay-font/generate.sh
```

Source: <https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip>
(`9883fdd4a49d4fb66bd8177ba6625ef9a64aa45899767dde3d36aa425756b11e`).

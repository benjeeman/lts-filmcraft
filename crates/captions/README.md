# filmcraft-captions

Caption files and caption burn-in for FilmCraft (layer L2: depends on `filmcraft-time`,
`filmcraft-color`, `filmcraft-project`, `filmcraft-text` and `roxmltree` for TTML).

The caption *model* (caption tracks in a sequence, captions, track style) lives in
`filmcraft-project` (`caption.rs`) because it is part of the `.fcproj` document. This crate reads
and writes caption files, converts them to and from caption tracks, and draws captions.

## Formats

| Format | Read | Write | Notes |
|---|---|---|---|
| SubRip `.srt` | yes | yes | BOM, UTF-16, Windows-1252 fallback, CRLF/CR, missing or wrong indexes, `,` or `.` milliseconds, 1–9 fraction digits, missing hours, SSA coordinates, cues not separated by blank lines. Inline tags are kept as text. |
| WebVTT `.vtt` | yes | yes | Cue identifiers, cue settings (verbatim), a leading `<v Speaker>` voice span (as the caption's speaker) and STYLE / REGION / NOTE blocks are kept. A missing `WEBVTT` signature is tolerated. |
| Scenarist SCC `.scc` | yes | yes | CEA-608 at 29.97 fps, drop-frame or not. Reads pop-on, roll-up and paint-on (channel 1); writes pop-on. |
| MacCaption MCC `.mcc` | yes | yes | One SMPTE ST 334-2 caption distribution packet (CDP) per frame in MacCaption's hex text with letter abbreviations. Writes 29.97 fps (`30DF` / `30`) with CEA-608 field 1 (the SCC schedule) **and** CEA-708 service 1 (two-window pop-on emulation). Reads CEA-608 field 1 when present, else CEA-708 service 1; time code rates 30DF, 30, 60DF, 60, 25, 24, 50. |
| EBU STL `.stl` | yes | yes | EBU Tech 3264-E: GSI block + 128-byte TTI blocks, `STL25.01` (25 fps) or `STL30.01` (read/written as 29.97 fps, non-drop labels), ISO/IEC 6937 Latin text (accents as diacritic + letter), italics / underline codes as `<i>` / `<u>`, extension blocks for long subtitles, start-of-programme (TCP) subtracted. |
| TTML `.ttml` (IMSC1) | yes | yes | Writes the IMSC1 Text profile (TTML2 namespaces): `<p>` per cue, `<br/>`, styled `<span>`s for `<i>` / `<b>` / `<u>`, `ttm:agent` speakers, exact frame times (`123f`) at the sequence rate. Reads any TTML (TTML1, TTML2, the 2006 `ttaf1` drafts): clock / frame / offset / tick times, `dur`, nested time containers, timed spans, referenced styles. |
| DFXP `.dfxp` | yes | yes | TTML1 with the DFXP presentation profile; clock times in milliseconds (frame-exact after `snap_to_frames`). |
| Cavena 890 `.890` | yes | no | Binary broadcast subtitles: a 388-byte header and 128-byte blocks (the last one may be 4 bytes short); the title block is skipped. Hebrew rows (code 0x8F) with their reversed digit runs turned back into reading order, Russian rows (0x56, Latin letters standing for Cyrillic) and Latin rows (0x09); a block's two rows join with a line break. Times are frame counts at 25 fps (or a given rate), kept as the absolute timecode they are (`Document::absolute_timecode`, a note with the first and last times). The language reaches the caption track; left / right alignment becomes an `align:` setting. Bytes with no established meaning read as U+FFFD with a typed warning that keeps the row. `write` gives no bytes; `try_write` refuses with `Error::WriteUnsupported`. |

All cue times are exact `Tick`s. Milliseconds convert exactly (1 ms = 254 016 000 ticks); SCC
frames map through `FrameRate::FPS_29_97`. `Document::snap_to_frames` snaps cues to a sequence's
frame grid (the engine does this on import) and removes overlaps.

### Cavena 890

There is no public specification. `cavena890.rs` follows public field-level descriptions of the
format and our own byte analysis of reference files from a broadcast delivery (Hebrew and
Russian), analysed locally and kept out of this repository (the tests build synthetic files byte
by byte).
What the reader relies on, and how sure it is:

| Where | What | Confidence |
|---|---|---|
| file size | 388 + 128·n, or 4 bytes less (the last block stops after its bottom row) | high: every reference file is 4 short; a whole last block is also read |
| header 179 | block count, u16 little-endian, title block included (copies at 208 and 210) | high |
| header 2 / 40 / 68 / 105 / 148 / 218 | tape number (20 bytes), title (28), translator (28), episode (11), comment (24), original title (28); ASCII, NUL padded | high (the public description and the files agree) |
| header 146 / 147 | character-set codes: 147 is the rows' set (0x8F Hebrew, 0x56 Russian), 146 the second set (0x09, Latin, in every file) | high for 0x8F / 0x56; 0x09 = Latin / English comes from the public description |
| header 181 / 246 | font names of the two sets (`HEBNOA.V` / `KYRIL2.V` in 181–194, starting at 187; `VFONTL.V` at 246) | high |
| header 214 | the first subtitle's in frame (u24 big-endian) | high (every file); not used |
| header 256 | start of message `HH:MM:SS:FF` | the text is there, but it is unreliable (`00:00:00:00` or `01:00:00:00` in files whose cues all start after 01:00:00:00): kept in `Header`, never used |
| block 0–1 | number, u16 big-endian (title 1, then 16, 32, 48…); a repeated number with the same times adds rows | numbering high; repeats come from the public description, unseen |
| block 2 / 5 | in / out, u24 big-endian frame counts, absolute | high; the rate is not stored (25 fps fits every file) |
| block 8 | alignment: 0x14 on every cue, 0x04 on the title block; 0x50 left / 0x52 right / 0x54 centre in the public description | low: only left and right reach the cue (`align:`) |
| block 15 | 0x16 on every block, probably the row (22, the bottom) | low: kept in `Subtitle::row`, not used |
| block 14 / 72 | bit 0x80 = this row (top / bottom) is in the Latin set | medium: set on both title-block rows, which are Latin; no cue has it |
| block 16 / 73 | top and bottom rows, 51 bytes each, padded with 0x7F | high |
| block 14, 70, 71 | 0x36, 0xA6, 0x19 on the last cue of every Hebrew file (a credit) | unknown: ignored |
| Hebrew 0x40–0x5A | א … ת in Unicode order, final forms included (U+05D0 + byte − 0x40) | high |
| Hebrew 0x20–0x3F | ASCII; seen: space `! " ' , - . / :  ?` and digits (`"` doubles as gershayim, `'` as geresh) | high |
| Hebrew order | the row is the screen read from right to left: Hebrew, dialogue dashes, quotes and end punctuation in reading order; every digit run (with one separator between digits: `2,500`, `14:30`, `24/7`) reversed. Runs follow the Unicode bidirectional algorithm (W4, W5, N1) | high: wherever the context shows the value (years, prices, clock times, sizes, idioms), the turned number is right, and no turned number keeps a leading zero or a broken thousands group |
| Hebrew, other bytes | none occur in the reference files. Small Latin letters, brackets and `# $ % & * + ; < = >` are read by the same rule and flagged (`Warning::UnverifiedOrder`); everything else (0x5B–0x60, 0x7B–0x7E, controls, 0x80–0xFF) is U+FFFD with `Warning::Undecoded` | open: the public description puts Latin capitals at 0x91–0xAA and small letters at 0xB1–0xCA, and italics / colour codes at 0x88 / 0x98 / 0xF1–0xF8; not decoded until a file shows them |
| Russian | `a` а `b` б `v` в `g` г `d` д `e` е `` ` `` ж `z` з `i` и `j` й `k` к `l` л `m` м `n` н `o` о `p` п `r` р `s` с `t` т `u` у `f` ф `h` х `c` ц `~` ч `{` ш `}` щ 0x1D ъ `x` ы `y` ь `\|` э `w` ю `q` я; capitals 0x20 lower (`@` Ж, `[` Ш, `]` Щ, `\` Э, `^` Ч); ASCII digits and punctuation, left to right | high for every letter seen (the file reads as plain Russian); `]` Щ by symmetry, unseen; no Ё, no capital Ъ |

Times are never shifted: a programme whose first frame is 01:00:00:00 has its first cue after
01:00:00:00. An importer places the cues by mapping the source's start timecode to its first
frame.

### SCC writer

Each caption is sent as a pop-on load (RCL, ENM, then per row a preamble address code and tab
offset for centring, then text; control codes doubled) into non-displayed memory, followed by End
Of Caption exactly on the caption's first frame. The load is placed in the latest run of free
frames before the caption, so back-to-back captions load while the previous one is on screen. An
Erase Displayed Memory clears the caption at its out point unless the next caption replaces it.
Text is wrapped at 32 columns, bottom-aligned (last row 15) and centred. Characters come from the
608 basic, special and extended sets (extended characters are preceded by a basic fallback
character, as the standard requires); others are dropped. If there is no room to load a caption
before its in point (for example a caption at 00:00:00:00), it appears as soon as it is loaded.

## Burn-in

`burn::render_caption` lays out a caption with the track style (font size relative to a 1080-line
frame, colour, background box, outline, alignment, top/middle/bottom anchor and margin; WebVTT
`line:N%` and `align:` settings override them), wraps lines to 90% of the frame width and
rasterises it to a small premultiplied linear-light RGBA overlay. `filmcraft-render` composites the
overlays of visible caption tracks over the finished frame (Program monitor and export burn-in).

Text is set in **Inter SemiBold** (`assets/fonts/Inter-SemiBold.ttf`, SIL OFL 1.1, attributed in
`ATTRIBUTION.md`) by the `filmcraft-text` engine: shaped with kerning and ligatures, bidi-ordered,
and drawn from its sub-pixel positioned glyph cache.

Specifications used: SMPTE ST 334-2 (CDP structure), CEA-608 / CEA-708 (public descriptions of
the caption commands), the MacCaption MCC format description (line syntax and abbreviations),
EBU Tech 3264-E (1991), ISO/IEC 6937, W3C TTML1 (2nd ed.), TTML2 and IMSC 1.1. Cavena 890 has no
specification: public field-level descriptions of the format (header and block offsets,
character-set codes, 0x7F padding, the short last block), Unicode Standard Annex #9 (the
bidirectional algorithm's run rules) and our own byte analysis of reference files (see the table
above). No other implementation's code was consulted.

## Tests

- Unit tests per module (timestamps, decoding, sloppy SRT, WebVTT blocks and voices, CEA-608
  tables, PAC round trips, known SCC pop-on and roll-up streams, layout).
- `tests/roundtrip.rs`: property tests — SRT (also with BOM + CRLF), WebVTT (ids, speakers,
  settings, STYLE blocks), SCC and MCC (drop-frame and non-drop, gaps and back-to-back captions;
  MCC also through CEA-708 alone), EBU STL (25 and 29.97 fps, accents, extension blocks), TTML
  (23.976 / 25 / 29.97 / 59.94, ids, speakers) round-trip frame-exactly; DFXP round-trips after
  snapping to frames; readers never panic on random bytes (also a valid STL header followed by
  random blocks, and random MCC lines); SRT → VTT → SCC keeps text and frames.
- `tests/cavena890.rs`: synthetic Cavena 890 files built byte by byte — Hebrew digit runs turned
  back (amounts, clock times, years, `2,000`, `24/7`, lists), dialogue dashes, quotes, two-row
  cues, the Russian table in both cases, a Latin file and a Latin-flagged row, the title block and
  header fields, short and whole last blocks, trailing bytes, frame-exact times at 25 fps and
  another rate, the absolute-timecode note, the language on the caption track, alignment,
  repeated block numbers, undecoded bytes and unverified Latin kept with typed warnings, count
  mismatches, detection (never an EBU STL file, never random bytes) and a reader that never
  panics; writing is refused.

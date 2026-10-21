# FilmCraft — LTS integration fork

Modified version based on FilmCraft by the ArtCraft team and community. This fork is maintained for
the LTS integration, where its web build runs as the **LTS Video editor**. It is not an official
ArtCraft build, and LTS did not write FilmCraft: the editor, its codecs and its tools are the work of
the FilmCraft contributors, credited in the app under About ▸ Contributors and About ▸ Models.

## What this fork changes

- **Host-managed browser sessions.** Started with the `embedded` query flag, the web build loads no
  demo, disables its OPFS storage and media retention, makes no automatic downloads, and offers the
  host checked command execution and a list of written outputs. See
  [docs/lts-embedded.md](docs/lts-embedded.md).
- **Embedded presentation.** With `embedded`, the editor names itself "LTS Video editor", shows no
  upstream community, website or repository links, and opens no external URL. Without the flag, the
  standalone app keeps the upstream name, wording and links.
- **Neutral product copy in both modes** where the product name reached user output or messages: the
  demo project is called "Demo Project", the built-in end-credits template no longer adds a product
  line to videos, and error and notice messages say "the editor".
- **Third-party notices.** About ▸ Third-party notices shows NOTICE, both licences, ATTRIBUTION.md and
  the licences of the fonts compiled into the editor, including egui's default fonts
  ([docs/notices/egui-default-fonts.txt](docs/notices/egui-default-fonts.txt)).

Technical identifiers are unchanged: crate and binary names (`filmcraft-*`), the `window.filmcraft`
browser API, the `.fcproj` and `.fcgt` formats, command and automation ids, storage paths and
environment variables. Exported files keep their FilmCraft tool tags as factual provenance.

## The editor

FilmCraft is an open-source, clean-room non-linear video editor written in pure Rust. It follows the
Premiere Pro workflow (the same panels, workspaces, tools and shortcuts) as an independent
implementation, and runs natively on macOS, Windows and Linux and in the browser via WebAssembly.

- **Editing:** Source and Program monitors, bins, a multi-track timeline, three-point editing, every
  trim tool and dynamic trimming, nesting, multicam, markers and unlimited undo, all on exact integer
  time (254,016,000,000 ticks per second).
- **Colour, effects and titles:** a Lumetri-style grading panel and scopes, the full video-effects bin
  and around 30 transitions on a GPU compositor that works in linear light, keyframes with value and
  velocity graphs, a text engine with OpenType shaping, and caption tracks.
- **Audio:** loudness meters to ITU-R BS.1770 and EBU R128, clip effects, track and clip mixers with
  inserts, sends and automation, and audio-clock playback.
- **Formats:** its own H.264, HEVC, VP9, AV1, ProRes, DNxHD/DNxHR, AAC and Opus code, written from the
  public specifications and tested against ffmpeg as an external oracle only, plus MP4/MOV, Matroska
  and WebM, MXF and MPEG streams. Export writes H.264 MP4, ProRes, DNxHR, MXF, PNG sequences, animated
  GIF and WAV.
- **Interchange:** Final Cut Pro 7 XML, FCPXML, OpenTimelineIO, CMX 3600 EDL, AAF and OMF.
- **Built for agents:** every menu item, button and drag is a command with an id and typed
  parameters; the UI, the CLI, a JSON control channel and an MCP server all dispatch the same commands.

## Build and test

Rust 1.95 or newer. From the repository root:

```sh
cargo run --release -p filmcraft                           # the desktop app, with a demo project
cargo run --release -p filmcraft -- --control 9876         # plus the JSON-lines control server
cargo run --release -p filmcraft-cli -- commands           # list every engine command
cargo run --release -p filmcraft-cli -- mcp                # MCP server (headless)
```

Quality gates: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`; `cargo xtask ci` runs them all with the layering, asset and wasm checks
(see [CONTRIBUTING.md](CONTRIBUTING.md) and [docs/contributing.md](docs/contributing.md)).

The web build targets `wasm32-unknown-unknown` (`apps/filmcraft-web`, see [docs/web.md](docs/web.md));
its LTS integration surface and build notes are in [docs/lts-embedded.md](docs/lts-embedded.md). The
embedded-presentation checks are `cargo test -p filmcraft-ui-egui --test embedded_presentation_ui` and
`node --test apps/filmcraft-web/tests/embedded_presentation.mjs`.

Japanese interface text needs the optional craft-fonts build input (`CRAFT_FONTS_DIR`, see
[AGENTS.md](AGENTS.md) §1.9). Without it the editor uses its bundled and the system's fonts; LTS
builds do not use it.

## Documentation

These documents come from upstream FilmCraft and are kept unchanged as reference, except where this
fork notes otherwise:

| | |
|---|---|
| [AGENTS.md](AGENTS.md) · [CLAUDE.md](CLAUDE.md) | Rules and working instructions for contributors and agents (with this fork's preamble) |
| [CONTRIBUTING.md](CONTRIBUTING.md) · [docs/contributing.md](docs/contributing.md) | Setup, quality gates, commit conventions, how to add commands, effects, codecs, panels and assets |
| [docs/architecture.md](docs/architecture.md) | Layers, data model, time base, command system, render and export pipeline |
| [docs/testing.md](docs/testing.md) | Unit, property and ffmpeg-oracle tests, accuracy criteria, benchmarks |
| [docs/agents.md](docs/agents.md) · [docs/control-protocol.md](docs/control-protocol.md) | Driving the editor over MCP and the control channel |
| [docs/project-files.md](docs/project-files.md) | `.fcproj` format, schema migrations, auto-save and crash recovery |
| [docs/graphics.md](docs/graphics.md) · [docs/captions.md](docs/captions.md) | Text engine, graphic clips and tools; caption tracks and formats |
| [ROADMAP.md](ROADMAP.md) | Upstream's assessment, gaps, milestones and estimates |

This fork publishes no release downloads.

## Architecture

A layered Cargo workspace:

| Layer | Crates |
|---|---|
| Codecs and containers | `h264`, `h264enc`, `hevc`, `vp9`, `prores`, `aac`, `opus`, `isobmff`, `matroska`, `bitstream` |
| Foundations | `time`, `geom`, `color`, `frame`, `media`, `text`, `project`, `audio-dsp` |
| Editing and interchange | `edit`, `codecs`, `captions`, `format` (project files), `interchange` |
| Rendering and output | `render`, `gpu`, `export` |
| Engine | `engine`: command registry, session, undo, jobs |
| Front ends | `ui-egui`, `automation` (MCP), `apps/filmcraft`, `apps/filmcraft-cli` |
| Test support | `testkit` (ffmpeg oracles, fixtures), `golden` (golden-image tests) |

Nothing below the front ends depends on a UI toolkit or OS API. `cargo xtask ci` checks formatting, lints, tests, the layering rules, asset attribution and the wasm build.

## License and credits

FilmCraft is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
Copyright (c) 2026 ArtCraft Team and the FilmCraft contributors. Required notices are in [NOTICE](NOTICE).

Bundled fonts, icons, images and other assets keep their own open licenses; each one is listed
with its author, source and license in [ATTRIBUTION.md](ATTRIBUTION.md). Builds made with the optional
craft-fonts input also embed its fonts (OFL-1.1), listed in that repository's ATTRIBUTION.md. egui's
default fonts, compiled into the interface, are covered in
[docs/notices/egui-default-fonts.txt](docs/notices/egui-default-fonts.txt). The licences of the other
Rust crates compiled into the editor are governed by `Cargo.lock`.

**Footage and music in the upstream screenshots (docs/images):** NASA's Apollo 11 film and television footage from images.nasa.gov (launch, Launch Control Center, lunar surface and recovery) and the Apollo 11 air-to-ground voice transcript, all US Government works in the public domain (NASA does not endorse this project); *Night of the Living Dead* (1968), *Carnival of Souls* (1962) and *Charade* (1963), all in the US public domain; *Earth Views from the ISS* by NASA; Chopin's Nocturne Op. 48 No. 1 and Ballade No. 1, performed for Musopen and released under CC0. The media itself is not in this repository. Sources and details for every asset are in [ATTRIBUTION.md](ATTRIBUTION.md).

FilmCraft is an independent implementation. It contains no Adobe code, icons, images, presets or LUTs, and no GPL or LGPL code; every icon is drawn in code and every bundled asset is openly licensed and attributed ([AGENTS.md](AGENTS.md)). ffmpeg is used only as an external test oracle.

The ArtCraft name, wordmark and logos are trademarks of the ArtCraft Team and are not covered by this license. This modified fork removes the upstream mark assets as required by [`docs/brand/LICENSE-brand.txt`](docs/brand/LICENSE-brand.txt); that license is retained for reference.

<sub>Adobe, Photoshop, Illustrator, Premiere Pro, Lightroom, Acrobat, After Effects and InDesign are trademarks or registered trademarks of Adobe Inc. in the United States and/or other countries. FilmCraft is an independent, open-source project and is not affiliated with, sponsored by or endorsed by Adobe Inc.; these names are used only to describe the workflows it is compatible with.</sub>

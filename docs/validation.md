# Validation record

Recorded on 2026-09-20 against the installation at
`/home/w/.wine/drive_c/otome_domain`. Installation files and Windows saves were
read only. Research and decoded samples are outside the repository.

Workspace build, formatting, and Clippy with warnings denied passed. All 36
synthetic tests passed. Four opt-in installed-game tests passed separately;
they remain ignored in the default workspace test run. The engine smoke test
returned the unsupported-syntax error documented below.

## Resource validation

The mounted archive index counts are:

| Archive | Entries |
|---|---:|
| data.xp3 | 1,889 |
| evimage.xp3 | 1,051 |
| fgimage.xp3 | 5,931 |
| video.xp3 | 5 |
| voice.xp3 | 23,204 |
| patch2.xp3 | 45 |

Patch resolution produces 32,120 names. A complete resolved-catalog scan
successfully decrypted/decompressed and verified Adler-32 checksums for 32,119
resources. The protection-notice entry has inconsistent declared and segment
lengths; reading it fails explicitly. Consequently, an unfiltered `verify`
returns a nonzero exit status. The scan did not suppress this error.

Checksum verification covers full resource bytes, not only recognizable
headers. It does not establish presentation fidelity or verify overridden
versions of resources that the resolver does not select.

The opt-in installed-game tests passed:

- All 193 resolved `.tjs` files passed checksum verification and text decoding.
- All 93 resolved `.scn` files passed checksum verification and PSB v2 parsing.
- `image/emotion/ang_1.tlg` decoded to a 100 by 4,600 RGBA image.
- `bgm/bgm01.ogg` decoded to 6,715,771 stereo PCM frames at 44,100 Hz.
- All 3,593 `.sli` files parsed: 26 links and 3,923 labels. The first BGM's
  smooth link jumps from frame 5,970,432 to frame 254,240 while flag 0 is zero.
- Rendering 2,400 frames across that BGM loop produced identical PCM with host
  buffer sizes of 1, 257, and 4,096 frames. The resulting source position was
  the loop destination plus 1,200 frames.

Media decoding has not been compared against original-engine pixels or audio.
The Cx profile reproduction tool produced a byte-identical JSON profile from
the pinned catalog. See [provenance.md](provenance.md).

The static inventory decoded 3,558,586 bytes of TJS source and reported 46
plugin-name string candidates. Three scripts still fail lexical inspection:
`sysscn/debugutil.tjs`, `sysscn/speech_ole.tjs`, and `system/utils.tjs`.
Unsupported lexical constructs remain; these diagnostics are included in the
inventory output. String candidates are not a runtime dependency trace.

## Runtime boundary

The unchanged `startup.tjs` is located and decoded from the installed archives.
Compilation stops with:

```text
startup.tjs:5:49: unsupported TJS construct try
```

This is an implementation gap, not a successful startup or budget exhaustion.
The startup guard also contains indexed object access, interpolation, functions,
and bound contexts that are not supported. Suppressing the guard or replacing
the original framework is not an accepted substitute for these semantics.

Synthetic session tests cover nested storage execution with shared globals,
source context for unsupported native calls, and an instruction budget shared
across recursion. They do not execute the game's framework.

## Reproduction

```sh
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings

KRKRZ_PROJECT_DIR=/home/w/.wine/drive_c/otome_domain \
  cargo test -p krkrz_assets --test installed_game -- --ignored --nocapture
KRKRZ_PROJECT_DIR=/home/w/.wine/drive_c/otome_domain \
  cargo test -p krkrz_runtime --test installed_game -- --ignored --nocapture

cargo run --release -p krkrz_cli --bin krkrz_tool -- \
  --project-dir /home/w/.wine/drive_c/otome_domain \
  --profile otome-domain verify

cargo run -p krkrz_cli --bin krkrz_engine -- \
  --project-dir /home/w/.wine/drive_c/otome_domain
```

The last two commands currently fail for the reasons documented above.

## Acceptance work still open

- Audit source revisions against the specified engine and TJS versions.
- Establish an isolated original-engine run and resolve its `MenuItem` failure.
- Differentially test TJS semantics and KAG parsing against upstream execution.
- Execute original startup and framework with object, plugin and PSB bindings.
- Reach title, first dialogue, first choice, first option and the next scene
  transition using one shared native/headless session.
- Integrate layer/text rendering, fonts, transitions, the PCM loop mixer,
  device playback, required effects and movies; implement SLI label expressions.
- Capture and compare state, resource selection, callback order, frames and
  audio against the original engine at each checkpoint.
- Implement original save/load operations in separate writable storage, restart
  before the choice and reproduce the selected branch after reload.
- Verify backlog, advance, auto/skip, settings, return to title and native Linux
  playthrough.

No title/gameplay checkpoint or save/load acceptance criterion has passed.

# Validation record

## 2026-09-21 integration update

The optimized runtime completed the unmodified warning and pretitle sequences,
rendered the title menu at 1280×720, and accepted native-order mouse events on
New Game (110,650). At 28–32 seconds the original scenario was `start.ks`, label
`*envplay`. This is a navigation checkpoint, not a completed story playthrough.
The subsequent 33.728-second run reached `Math.RandomGenerator` in `Action.tjs`.

The former lifetime allocation ceiling is replaced by a live-object arena and
explicit safe-point garbage collection. Monotonic object IDs prevent reclaimed
handles from aliasing new objects. Script graph, native references, external host
roots and finalizers are covered by focused VM/runtime tests. Native and replay
hosts collect between ticks; Session embedders explicitly provide retained Values.
Finalizer timing is deferred, rather than immediate reference counting.

The older sections below are historical component records. Their original startup
failure locations and missing-feature lists do not supersede this update.

Recorded on 2026-09-20 against the installation at
`/home/w/.wine/drive_c/otome_domain`. Installation files and Windows saves were
read only. Research and decoded samples are outside the repository.

Workspace build, formatting, and Clippy with warnings denied passed. All 100
synthetic tests passed, including 151 TJS cases, 37 ScriptsEx plugin cases,
25 serialization cases, 27 CSVParser cases, 15 System cases, 22 sound-control cases, 13 getSample cases,
11 decoded-WAV cases, 36 PSBFile cases, 80 TextRender cases, 39 layerExDraw/reached-language cases, 18 AlphaMovie API cases, 24 menu/reached-language cases, 21 WIN32Dialog cases and one plugin-alias case.
All 151 TJS cases matched
the installed Kirikiri Z 1.2.0.3 executable in separate isolated processes.
Habakiri matched 137 cases; fourteen interpolation/RegExp, real formatting and
octet differences are explicitly excluded from that Java comparison and covered
by the original-engine run.
All nine opt-in installed-game tests passed; they remain ignored by default.
The real startup run reaches the missing `windowEx.dll` plugin documented below.

All 37 ScriptsEx cases match the installed PackinOne component in isolated
original-engine runs. They cover object contexts, dictionary key/count reflection,
hidden/static flags, accessors, strict/numeric structural comparison, recursive
cloning, callback context and early exit, live array values, dictionary hash
collisions/deletion/lookup promotion, deferred rehashing, discarded native
results, and repeated plugin loading. Cyclic clone/comparison inputs fail with
an explicit Rust resource-limit error rather than unbounded recursion.

All 25 serialization cases match the installed engine and PackinOne component.
Coverage includes native Array text load/save, Array/Dictionary structured
saves, saveStruct plugin strings and files, octet values, hexadecimal real formatting,
constant-container identity, UTF-8/CP932, cipher/compression modes and byte
offsets. File comparisons use exact bytes except for zlib streams, where both
headers/lengths are validated and decompressed UTF-16 content is compared.
Synthetic restart tests reload saved state into a fresh Rust Session and verify
overlay precedence. Separate tests reject traversal, archive write targets and
symlinks escaping the writable directory. The installation is not a write target.
These are component tests, not evidence that the original game save UI works.

All 27 CSVParser cases match the installed component. They cover quoted and
multiline fields, separators, line numbers, callback targets and discarded
bound contexts, subclass dispatch, reentrant input changes, failed opens,
continued parsing, UTF-8/CP932 files, explicit finalizers, invalidation during
callbacks, and the older boolean storage API. A
separate budget test stops a callback that continually reinitializes the parser.
The parser deliberately preserves observed final-CR and byte-0xFF boundaries.
Malformed legacy encoding replacement and automatic instance collection remain open.

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

The static inventory decoded 3,558,586 bytes across all 193 resolved TJS scripts
without lexical errors and reported 52 plugin-name string candidates. Lexing is
not full compilation or execution; string candidates are not a dependency trace.

## Runtime boundary

The unchanged `startup.tjs` compiles and executes from the installed archives.
`system/Initialize.tjs` now compiles completely and executes its Window subclass
definition, dynamic property definitions, preprocessor configuration, archive
search paths, patch discovery and system-version logging. PackinOne and
KAGParserEx registration now succeed; AppConfig and Config execute unchanged,
then the application lock is acquired. Compatibility scripts and framework
definitions through `LineModeEx.tjs` load unchanged. KAG system loading stops at:

```text
startup.tjs:20:1 at VM instruction 169
system/Initialize.tjs:465:1 at VM instruction 990
system/Initialize.tjs:275:2 at VM instruction 11
MainWindow.tjs:16:7 at VM instruction 22
... k2compat plugin dispatch ...
unsupported Kirikiri native operation: Plugins.link(windowEx.dll)
```

No startup guard or original script was removed or replaced. The next reached
missing feature is the Window extensions plugin interface. Plugin loading registers
implemented ScriptsEx/saveStruct/CSVParser components and declarations for the
observed remaining exports. Layer, Process, TemporaryFiles and KAGParser native
construction and unfinished methods still raise explicit uncatchable engine
errors; successful registration is not full plugin support. Window support is a shared state model, not OS
presentation: construction, caption, visibility, client dimensions, position,
primary-layer lookup and deferred resize/action callbacks have synthetic tests.
Explicit invalidation releases native Window state and cancels pending callbacks.
Window frame metrics, input, layer attachment and automatic collection remain incomplete.

The installed startup handoff test runs the original source with a recording
host and deliberately aborts on its first framework execution request. It
proves the handoff, not successful initialization. The real-session smoke test
above establishes the current failure separately.

Synthetic tests cover object identity through thrown values, nested handler and
scope unwinding, function contexts, inheritance, accessor dispatch, argument
expansion, UTF-16 strings and evaluation. Tests also verify that script catches
cannot hide instruction-budget exhaustion, missing native/plugin implementations,
or unsupported compiled scripts. The object arena retains allocations until
VM destruction and stops after 100,000 objects. Explicit `isvalid`/`invalidate`
semantics now include script finalizers, native cleanup, recursive and throwing
finalizers, invalid bound contexts, and property finalizers. Automatic
reference-counted finalization and collection remain open.
Function hoisting, full native APIs and full source/bytecode compatibility remain
open. Default parameters now bind in declaration order, evaluate only for void
arguments and use the function context; unnamed argument tails also forward.
Switch comparison/fallthrough/scopes, do/while, comma evaluation, swap target
reevaluation and ordered per-instance class-body execution match the target
corpus. Base expressions are deferred until instance initialization and missing
class-member dispatch, and `super` reevaluates its base expression. Thus unused
MIDI/CDDA subclasses can be defined without inventing native classes absent from
the target engine. Class-body `super` is rejected during compilation, matching the target.
Casts, mutable const declarations, comma aliases, trailing omitted arguments
and octet parsing/indexing/equality/concatenation have target-engine comparisons. Preprocessing, regex operations and discarded `!`
evaluation of dynamic property declarations now execute. Regex handling is
bounded, rejects isolated UTF-16 surrogates, and is not a complete Oniguruma
implementation. Local-variable deletion remains an explicit unsupported case.
Constant-expression error timing has not been matched to upstream constant folding.

The independent Java reference has 32-bit integers, unlike the Rust VM's 64-bit
integers. The shared corpus avoids that difference. Differential agreement is
limited to these cases. The separate original-engine run establishes exact-version
agreement for this corpus, not gameplay compatibility.
See [community-implementations.md](community-implementations.md).

## Isolated original-engine checks

`tools/check_original_tjs.py` copies the installed executable to a separate cache,
packages each synthetic fixture as a Cx-encrypted XP3, and runs each case in a new
process under a dedicated Wine prefix. The game installation and existing Wine
prefix are not changed. The report records version `1.2.0.3` and executable SHA-256:

`8f50f48489647638a6d14639c1398fa7f731d262c1d4edc7a65d4362be2a9768`

The executable's startup log identifies TJS2 `2.4.28`. No executable, DLL, or game
script is bundled. Wine/Xvfb are optional reference-test dependencies only.

```sh
cargo build -p krkrz_cli --bin krkrz_tool
xvfb-run -a python3 tools/check_original_tjs.py /path/to/otomedomain.exe
```

The 151-case TJS fixture SHA-256 is
`0d73ef6f26f03c10c160c523bc0525d754e8439444ce7d3fa625c2e082cbf4a3`.
Use `--cache` to override the external cache and `--case PREFIX` to select cases.
For the plugin corpus:

```sh
xvfb-run -a python3 tools/check_original_tjs.py /path/to/otomedomain.exe \
  --fixtures crates/krkrz_runtime/tests/fixtures/scripts_ex.json \
  --plugin ScriptsEx.dll=/path/to/plugin/PackinOne.dll --runtime
```

The oracle copies PackinOne under the synthetic fixture's `ScriptsEx.dll` name.
Its SHA-256 is
`89cae5df61a10bbcdd7540cfff9cf800aef064d378bf5808a3b63df673ff41f5`.
The Rust session loads its ScriptsEx component directly; it does not execute
the DLL. `scripts_ex-report.json` and `semantics-report.json` in the external
cache record binary/fixture hashes, results and failures. Array member reflection,
octet MD5, and `foreach` on other native object kinds remain unsupported.
PackinOne registration is implemented; other component operations remain open.

For serialization, use a fresh Japanese-locale reference prefix. Wine's Windows
ANSI code page depends on the prefix and launch locale. Keep
`LC_ALL=ja_JP.UTF-8` on later runs with that prefix too. A Western locale converts
Japanese plugin output to question marks; that is not a valid CP932 baseline.
The oracle reads source as UTF-8, so the CP932 case compares written bytes without
asking the UTF-8-configured reference reader to decode them.

```sh
xvfb-run -a env LC_ALL=ja_JP.UTF-8 LANG=ja_JP.UTF-8 \
  python3 tools/check_original_tjs.py /path/to/otomedomain.exe \
  --cache /path/to/external/original-tjs-japanese \
  --fixtures crates/krkrz_runtime/tests/fixtures/serialization.json \
  --plugin saveStruct.dll=/path/to/plugin/PackinOne.dll --runtime
```

The successful serialization fixture SHA-256 is
`aab80fe0619a0bef66355b182125db973eb25d78e2ec3d855e86bfcff9d546d3`.
`serialization-report.json` records the 25 comparisons and zero failures.
The original DLL is aliased as saveStruct only in the isolated test directory.
The Rust implementation runs its own serializer and does not load that DLL.
An empty-octet plugin serialization probe crashed the original DLL; Rust emits a
valid empty octet literal. That recovery is not counted as reference agreement.
Native empty-octet save/reload and plugin nonempty-octet formatting do agree.
Top-level octet conversion-error probes also triggered a reference failure; the
checked conversion fixtures use function scope, where exceptions are catchable.

Remaining serialization limits include CP932 fallback escaping,
full native-object enumeration and exact pointer comments for discarded
non-container objects. Plugin cycles fail explicitly. Output is bounded and
writes replace files atomically; unlike upstream, an error cannot leave a
partially overwritten save. Unmarked read encoding detection is a tooling
convenience, not a Windows locale implementation. Only the tested text mode
combinations are supported. Constant containers currently materialize on first
execution; exact compiler-time creation/error timing remains open.

For CSVParser, use the same Japanese prefix and oracle command with
`--fixtures crates/krkrz_runtime/tests/fixtures/csv.json`,
`--plugin csvParser.dll=/path/to/plugin/PackinOne.dll` and
`--plugin saveStruct.dll=/path/to/plugin/PackinOne.dll`. The saveStruct alias
prepares synthetic CSV files in several cases. The CSV fixture SHA-256 is
`edb0d69a662e1d65e320267892cb194eee611f2cce59f3cf9611d56d9b642430`.
`csv-report.json` records 27 comparisons and zero failures.

All 15 `system.json` cases match the original engine. They cover 395 numeric
constants, absent stretch names added in newer upstream revisions, layer-mode
aliases, byte-based cache limits/clamping and repeated application locks. The
fixture SHA-256 is
`50b5cbc961b0b64fcf5d35831596a173b94457238d398434f18120a09fbc423b`.
`krkrz_tool exec --runtime` uses the same native properties as startup. Separate
Rust tests cover LRU eviction, shrinking/disabling the cache, active-image
lifetimes, cross-process lock exclusion and release at Session destruction.
Linux auto-cache sizing follows upstream memory tiers with a 512 MiB cap;
it need not equal the 32-bit Windows host's available-memory result. Image cache
use by Layer decoding and Windows mutex interoperation remain open.

All 22 `sound.json` cases match the original engine: constructor arguments,
unloaded state, clamping/coercion, static member flags, subclass dispatch,
explicit finalization, action event payloads/return values, immediate and stopped
fades, replacement fades, and absence of MIDI/CDDA classes. Fixture SHA-256:
`0b40fb397b6dadff80a563e9b033ad04015354873c525c60ec332e0d33e516c4`.
Use the oracle with `--fixtures crates/krkrz_runtime/tests/fixtures/sound.json
--runtime`. `sound-report.json` records 22 comparisons and zero failures.
Separate Rust tests exercise deterministic 60 ms fade beats, delay quantization,
and cancellation on invalidation; timed behavior follows the upstream source and
has not yet been compared with captured original-engine audio/timing.

The getSample replacement follows Kirikiroid2's community source. All thirteen
`get_sample.json` cases match the installed DLL: lazy settings/defaults, implicit
visualization enablement, zero samples, callback arguments and errors, result
bounds, repeat loading, and legacy discarded/nonpositive results. Fixture SHA-256:
`2f5ccb14e67ca4f0cc1316f4977f053b4101c830e8f25e1883e61995bdc712d5`.
The DLL SHA-256 is
`e190ae1b17f82b106fa466e9ab985364e018c8acd46b39b2b33b895133c2f30c`.
Scoped buffer handles replace raw pointers. Legacy unwritten malloc samples are
zero-filled; that safety change is not reference agreement.

WaveSoundBuffer now opens WAVE/Vorbis storage and SLI metadata. All eleven synthetic
`wave.json` cases match the reference. They compare loaded status, reopen/failed-open callbacks, pause, output
format, and sample measurements against the installed executable/plugin.
Fixture SHA-256:
`7fc997993eb882394e9fcd68b766f653e0f9059b3fcf3db690f6e051f177f610`.
The synthetic `tone.wav` SHA-256 is
`cf0bb2eaddf44403a3ddfcf33b2daaa706fa53fcc6575410a97c0d2e781fefd5`.
The deterministic source API returns unscaled 16-bit PCM at the source rate;
Rust tests check exact samples, preview without advancement, looping, labels,
EOF dispatch, callback cancellation and invalidation. An additional opt-in test
opens the installed first BGM through the native TJS object and compares its PCM
and position across the SLI crossfade with the independently constructed source.
WAVE parser tests cover all four integer depths, chunk order/padding, duplicates,
truncation, malformed alignment and frame limits.

Device mixing, volume/pan gains, resampling, clock synchronization, filters,
script-visible loop flags, 3D controls and negative visualization offsets remain
open. Source-position reporting is deterministic; DirectSound's buffered/stale
cursor behavior is not reproduced. Only 16-bit output format has reference checks.
Visualization supports forward source previews with EOF padding, not a complete
DirectSound ring. Querying totalTime before creating an output format returns an
error instead of reproducing the original's possible divide-by-zero crash.
No game audio playback checkpoint has passed.

Reproduce plugin and decoded-WAV comparisons with:

```sh
xvfb-run -a python3 tools/check_original_tjs.py /path/to/otomedomain.exe \
  --fixtures crates/krkrz_runtime/tests/fixtures/get_sample.json --runtime \
  --plugin getSample.dll=/path/to/plugin/getSample.dll
xvfb-run -a python3 tools/check_original_tjs.py /path/to/otomedomain.exe \
  --fixtures crates/krkrz_runtime/tests/fixtures/wave.json --runtime \
  --plugin getSample.dll=/path/to/plugin/getSample.dll \
  --asset tone.wav=crates/krkrz_runtime/tests/fixtures/tone.wav
```

The oracle copies synthetic assets into both isolated directories and records
asset hashes. It removes TJS's leading plus from positive real output to produce
valid JSON. Native floating-point text precision still limits direct comparisons;
nonzero sample peaks are checked by bounds in the reference corpus and by exact
PCM-derived values in Rust tests.

The single `plugins.json` case confirms that loading `layerExImage.dll` after
PackinOne returns void and preserves the existing Layer method identity. Its
fixture SHA-256 is
`1d46b2d889f5ad66e91dc6941620187fd7fb1eea538e1da67c095a84cd4a9811`.
`plugins-report.json` records one comparison and zero failures. The Rust alias
requires PackinOne to be loaded; it does not implement the unfinished effects.

Separate probes that invalidate a builtin class and then invoke its constructor
crashed the original engine. Rust rejects that operation without crashing;
this safety test is not counted as reference agreement.

The checked-in synthetic plugin probe can also inventory registrations from copied
plugins. It does not establish that the Rust engine implements those interfaces.

## PSBFile native binding

All 36 synthetic cases in `psb_file.json` match the installed PSBFile plugin.
They cover scalar/null/octet mapping, nested views, fresh dispatch identity,
read-only writes/deletion, array bounds, missing keys, reflection, cloning,
comparison, subclassing, explicit finalization, repeated opens and plugin loading.
The saveStruct plugin enumerates PSB dictionaries but serializes PSB arrays as
empty arrays; this observed native-Array-accessor quirk is preserved.

Fixture SHA-256: `2dea1657469b0f5cfdf05c6b2ba3a31d3b8a3d68ba533e99ce75199ab3946b32`.
Plugin SHA-256: `1a1fdb2a7565a6a3bb64dabdecdb0e2df20d402edbd0c71cb5ca3c74f337653e`.
The fully synthetic PSB files are reproducible with `tools/make_psb_fixtures.py`:

- `list.psb`: `6c0bd354a0863336661ffb49d7018f040bd4c3c34cb23acea6a5b42a409920a0`
- `object.psb`: `f1dd5eac4a17e5eb58b7f6468fed234a98b70eb2edb9384e195110657d34bf41`

```sh
xvfb-run -a python3 tools/check_original_tjs.py /path/to/otomedomain.exe \
  --fixtures crates/krkrz_runtime/tests/fixtures/psb_file.json --runtime \
  --plugin psbfile.dll=/path/to/plugin/psbfile.dll \
  --plugin ScriptsEx.dll=/path/to/plugin/PackinOne.dll \
  --plugin saveStruct.dll=/path/to/plugin/PackinOne.dll \
  --asset list.psb=crates/krkrz_runtime/tests/fixtures/list.psb \
  --asset object.psb=crates/krkrz_runtime/tests/fixtures/object.psb
```

An additional opt-in installed-game test executes the unchanged `StorageData`
script and reads `scn/ra01_0.txt.scn`, comparing scene count, text count and label
lookup with the format decoder. This is a native data-access test, not a story
playback checkpoint. Invalidating a PSBFile revokes its outstanding Rust views;
a corresponding original-plugin stale-view probe timed out, so safe rejection
is not counted as reference agreement. Negative array indexes are rejected.
Automatic native object collection, newer PSB versions and encrypted headers
remain open.

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
- Resolve the original full-game `MenuItem` environment failure. Isolated
  synthetic TJS and plugin probes now run successfully, but are not a game baseline.
- Expand TJS differential coverage and test KAG parsing against upstream execution.
- Execute original startup and framework with object, plugin and PSB bindings.
- Reach title, first dialogue, first choice, first option and the next scene
  transition using one shared native/headless session.
- Integrate layer/text rendering, fonts, transitions, the PCM loop mixer,
  device playback, required effects and movies; implement SLI label expressions.
- Capture and compare state, resource selection, callback order, frames and
  audio against the original engine at each checkpoint.
- Integrate the implemented serializer and separate writable storage with the
  original save/load scripts, restart before the choice and reproduce the
  selected branch after reload.
- Verify backlog, advance, auto/skip, settings, return to title and native Linux
  playthrough.

No title/gameplay checkpoint or save/load acceptance criterion has passed.

## TextRenderBase native binding

All 80 synthetic cases in `text_render.json` match the installed TextRender DLL.
Coverage includes the observed defaults and property types, callback arguments,
font/style resets, character dictionaries and snapshots, line wrapping, basic
ruby, horizontal/vertical alignment, font/time scaling, inline commands,
key-wait positions, embedding, delay ordering and appended text. Repeated `done()`
calls preserve the target's cumulative alignment behavior. The corpus uses fixed
script-provided font metrics; it does not verify glyph rasterization.

Fixture SHA-256:
`3e42dd759f139aa604c57c71fa93e055cb67c5988e039157352c4c89cf85ef71`.
Installed `textrender.dll` SHA-256:
`8ea4a3f9ae25e3a221f33a2a58abd40155c5bb74f1d9e84cca8a69aa043d0274`.
The report records 80 comparisons, zero failures and engine version `1.2.0.3`.

```sh
cargo build -p krkrz_cli --bin krkrz_tool
xvfb-run -a python3 tools/check_original_tjs.py /path/to/otomedomain.exe \
  --fixtures crates/krkrz_runtime/tests/fixtures/text_render.json --runtime \
  --plugin textrender.dll=/path/to/plugin/textrender.dll \
  --plugin ScriptsEx.dll=/path/to/plugin/PackinOne.dll \
  --plugin saveStruct.dll=/path/to/plugin/PackinOne.dll
```

Three original results in `text_render_pending.json` retain known unfinished
behavior: multiple hanging punctuation characters and two uneven ruby groups.
Its SHA-256 is
`d4357cef01a9296aa5ea4322a498861a3b6ac8e5a9010fc1962bc141c8463eb9`.
Vertical ruby is also explicitly unsupported.
Rust currently raises explicit unsupported-operation errors for the three pending cases;
that test is not counted as reference agreement. Graphical characters,
configurable kinsoku limits, word-break/width-time options and full scrolling
also remain open. Separate tests check callback invalidation, execution budgets,
malformed commands and isolation from script extensions of native arrays.

The opt-in installed test loads the unchanged `system/textrender.tjs` wrapper,
supplies a fixed-width metric callback, and verifies wrapped character positions
and text. It does not establish actual font metrics, rendered pixels or a game
checkpoint. All eight installed-game tests passed after this change. The
workspace at the TextRender milestone had 79 synthetic test functions; the application-lock test also invokes
one of those in a child process, producing an additional passing result line.


## layerExDraw geometry and reached TJS syntax

All 39 cases in `layer_draw.json` match the installed executable and layerExDraw
DLL. Thirty-two cover plugin registration and geometry: constants, exact export
names, points, rectangles, copied bounds/locations, static Union, float32 affine
matrices, composition order, inverse/status behavior, subclassing, converter
accessors and their mutations, explicit finalization, and repeated plugin loading.
Seven cover TJS syntax reached after registration: `&&=`, `||=`, RHS-first
assignment evaluation and unary-dot resolution. Logical assignments evaluate
both operands and store an integer boolean; they do not short-circuit like the
ordinary logical operators.

Fixture SHA-256:
`5fb66dff28658520592de38dc0a9c5b3ac8211cf7b5e1987dae0094b9d8acbf1`.
DLL SHA-256:
`c635fe5fbc49825bcc48cbb0733e3e06d9a172829e15b9624a4c22695e830a12`.
The interface provenance is recorded in `layer-draw-interface.json`.

```sh
cargo build -p krkrz_cli --bin krkrz_tool
xvfb-run -a python3 tools/check_original_tjs.py /path/to/otomedomain.exe \
  --fixtures crates/krkrz_runtime/tests/fixtures/layer_draw.json --runtime \
  --plugin layerExDraw.dll=/path/to/plugin/layerExDraw.dll \
  --plugin ScriptsEx.dll=/path/to/plugin/PackinOne.dll \
  --plugin saveStruct.dll=/path/to/plugin/PackinOne.dll
```

Separate tests verify native-state cleanup on invalidation, shared callback
budgets and rejection of unfinished drawing/resource operations. Null PointF
conversion and a void Matrix-argument probe timed out in the isolated original
process. They are excluded from the agreement corpus; Rust rejects those inputs
without dereferencing native pointers. No GdiPlus image/font/path/appearance
resource or Layer drawing result is claimed as supported by these geometry tests.
The 83 workspace tests and all eight opt-in installed-game tests pass.


## AlphaMovie container and API

All four installed `image/emotion/suc_*.amv` resources pass checksum and complete
container validation. Each declares 25 frames at FPSScale=1/FPSRate=30 and has
40 rectangle packets, including empty packets and repeated sequence numbers.
Canvas sizes are 175x180, 350x360, 525x540 and 700x720. This validates indexing,
not decoded pixels. Synthetic tests cover both DCT-alpha and DEFLATE-alpha
container forms, truncated headers/payloads, invalid lengths, dimensions and
packet tags. The CLI `amv` command exposes the checked index.

The 18 synthetic `alpha_movie.json` cases match the original DLL's metadata,
property types, read-only fields, loop flags, preload range (1–30), 32-bit
conversion, transport flag, failed-open state, explicit finalize/constructor,
ignored out-of-range seeks and case-sensitive repeated plugin loading. The
checked-in `empty.amv` contains no image content; regenerate it at a new path
with `tools/make_amv_fixtures.py`. Metadata uses the original FPS field order,
which differs from the names in the community parser.

Fixture SHA-256:
`aa370eb3214af6cc1bbc15a3803e873e073f01a8d2d3a1b6b524f428fbaf3c46`.
Synthetic AMV SHA-256:
`ad0587c3c015515e989d472eb7c4be4e9618f0b5c6b5ca705c6b0a26bf7cc21e`.
DLL SHA-256:
`968359e426a83096ba6505002b0cf5df14c925c6e5807c9607de0602308e1b08`.

```sh
cargo build -p krkrz_cli --bin krkrz_tool
xvfb-run -a python3 tools/check_original_tjs.py /path/to/otomedomain.exe \
  --fixtures crates/krkrz_runtime/tests/fixtures/alpha_movie.json --runtime \
  --plugin AlphaMovie.dll=/path/to/plugin/AlphaMovie.dll \
  --asset empty.amv=crates/krkrz_runtime/tests/fixtures/empty.amv
```

Additional Rust tests verify budget enforcement in container opening, native
storage release on subclass invalidation, and uncatchable errors for unfinished
pixel decoding, positive in-range seeks, movie queuing and Layer positioning.
An original `play()` call without an open movie timed out in the isolated
process. Rust rejects it safely; this behavior is not counted as a match.
`play`/`stop` currently change transport intent only. No rendered frame,
decoding completion, movie timing or game checkpoint is claimed.

## MenuItem model and shared TJS fixes

Twenty-four synthetic cases match the installed menu DLL and Kirikiri Z 1.2.0.3
with zero differences. They cover default properties, the cached private window
root class, insertion-history child order versus display indices, reparenting,
mutable cached child arrays, radio groups, direct action dispatch, recursive
invalidation, repeated construction, and case-sensitive repeated plugin links.
Controlled shortcut maps test modifier ordering and key conversion without
assuming that Wine/X11 key names match a Linux host keyboard layout.

These probes also verified bound `this` identity in class and native callbacks,
hex/octal string escapes, and omission of zero-valued escaped characters in
ordinary and interpolated literals. Both menu and Window event dictionaries now
return bound target handles. The corpus includes those regressions.

Five Rust integration tests run the corpus and check deferred event batches,
invalidation during delivery, disabled ancestors, unattached items, callback
budget exhaustion, safe rejection of invalid indices/cycles, and explicit errors
for missing native menu presentation. A rebuilt installed-game startup gets past
menu loading and reaches `Plugins.link(win32dialog.dll)` through
`k2compat_modeless.tjs`. This is not a title or gameplay checkpoint.

Fixture: `crates/krkrz_runtime/tests/fixtures/menu.json` (24 cases), SHA-256
`b873ba9ec7b30874ab029e5321dfe4df1e927aad15e754b276142e99f8382761`.
Installed menu DLL SHA-256:
`ecc8eba384e2dd2bb7ada8655e63bd9a3962b899dbe4fd8efdf6e1734603a49a`.

```sh
xvfb-run -a python3 tools/check_original_tjs.py /path/to/otomedomain.exe \
  --fixtures crates/krkrz_runtime/tests/fixtures/menu.json --runtime \
  --plugin menu.dll=/path/to/plugin/menu.dll
```

Menu bars, popups, HMENU access, keyboard accelerator delivery, host-specific
key-name tables, HWND proxy objects and automatic collection remain unfinished.
Native menu presentation calls fail explicitly; no title/playthrough, frame/audio
comparison or original save/load acceptance has passed.

## WIN32Dialog data model

Twenty-one synthetic cases compare the Rust binding with the installed
`win32dialog.dll`. They cover 1,012 observed constants, required constructor
arguments, modeless coercion, closed-state metadata/progress errors, default
callbacks, repeated construction, explicit finalization, template field getter
order, void-valued properties, copied templates, Blob endianness/truncation,
invalidation, repeated plugin linking, and static method/nested-class visibility.
The newer source includes absent APIs (`setActive`, Blob dword-long methods and
several constants); those are not declared in the installed-game profile.

Five Rust tests check the corpus, buffer bounds and allocation limits, explicit
errors for raw pointers/platform operations, reentrant invalidation during
property reads, shared execution budgets, exception messages and traces, and
owned UTF-16 template layout with four-byte alignment after source invalidation.
Caught native errors now put their message in `message` and the full Rust/VM
context in `trace`; unhandled errors retain their storage and instruction context.

Fixture SHA-256 (`crates/krkrz_runtime/tests/fixtures/dialog.json`):
`60e07e8de88c67d0dcdad020c98d4e586ae9e92482eb2323ce7a591e35b68708`.
Installed DLL SHA-256:
`e3e877c24614bf88e49fcfa979e99100188226824a96f71ab67ed670949517ad`.
Registration data SHA-256 (`crates/krkrz_runtime/data/win32dialog.json`):
`c300aa1544a839969b7c9eb280c646f8d1c4afe8cde33353e854cacc56585e4c`.

```sh
xvfb-run -a python3 tools/check_original_tjs.py /path/to/otomedomain.exe \
  --fixtures crates/krkrz_runtime/tests/fixtures/dialog.json --runtime \
  --plugin win32dialog.dll=/path/to/plugin/win32dialog.dll
```

Templates use the community source's extended dialog layout. The Rust layout
unit test does not establish native dialog visual fidelity. Blob allocation is
limited to 16 MiB, assembled templates to 16 MiB and each template string to one
million UTF-16 units; invalid bounds are rejected before memory access. No raw
process pointer is exposed. Modal/modeless display, control interaction, resource
DLL loading, icons, drawing and progress dialogs still require a Linux host
implementation. Startup passes dialog registration but stops at
`Plugins.link(windowEx.dll)` in `MainWindow.tjs:16` via `k2compat.tjs`.
No title, story, native playthrough or save/load acceptance has passed.

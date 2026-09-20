# Rust Kirikiri Z compatibility engine

This repository contains an experimental native Linux port of Otome Domain.
The unchanged game archives now reach the title menu and playable opening
dialogue: native-order mouse clicks advance the first five text entries. The
original Load menu restores copied Windows saves, including a choice that
advances into its selected branch. Native X11 input also reaches and advances
opening dialogue. **Full-game compatibility is not established yet:** complete
scene traversal and in-game movies are still being verified. Saving from the
original UI, restarting, restoring and advancing the saved dialogue also pass.

The Rust TJS interpreter and Kirikiri services run the original scripts. Native
winit/wgpu presentation, mouse/keyboard input and CPAL audio share the same
session as deterministic headless replay. The game installation remains read-only;
saves use a separate directory.

## Build and verify

```sh
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

Movie playback and the movie integration tests require `ffmpeg` and `ffprobe`
on PATH. MPEG frames stream through a bounded decoder; stereo PCM is mixed on
the session clock.

The workspace builds independently of sibling repositories. `Cargo.lock` pins
Rust dependencies. Research checkouts, extracted scripts and media belong outside
the source tree, for example `$XDG_CACHE_HOME/krkrz_rs`.

## Developer tools

The following examples read the existing installation without modifying it:

```sh
cargo run -p krkrz_cli --bin krkrz_tool -- \
  --project-dir /path/to/otome_domain list --contains startup
cargo run -p krkrz_cli --bin krkrz_tool -- \
  --project-dir /path/to/otome_domain --profile otome-domain verify --contains .tjs
cargo run -p krkrz_cli --bin krkrz_tool -- \
  --project-dir /path/to/otome_domain --profile otome-domain script startup.tjs
cargo run -p krkrz_cli --bin krkrz_tool -- \
  --project-dir /path/to/otome_domain --profile otome-domain psb scn/ra01_0.txt.scn
cargo run -p krkrz_cli --bin krkrz_tool -- \
  --project-dir /path/to/otome_domain --profile otome-domain inventory
cargo run -p krkrz_cli --bin krkrz_tool -- eval '3 / 2'
```

Other commands: `extract NAME NEW_FILE`, `image NAME NEW_PNG`, `audio NAME`,
`loops NAME`, `kag NAME`, `compile NAME`, and `exec NAME`. Extraction is always explicit and refuses to
replace files or write inside the installation. `inventory` lists **static
candidates**, including lexer diagnostics; it does not establish reachability.
`compile` emits this implementation's diagnostic register instructions, not
Kirikiri's original bytecode format. `exec` runs a source file in the standalone
TJS VM and prints a JSON value; Kirikiri services are provided by the engine
command or `exec NAME --runtime`. The runtime option uses the same Session as
the engine, including plugin registration and shared execution budgets.
Add `--advance-ms 1000 --step-ms 16` to deliver deterministic host ticks after
execution, and repeat `--inspect EXPRESSION` to inspect resulting script state.
Use `--frame NEW_PNG` to capture the completed layer tree (`--frame-window EXPRESSION` selects a window).
Use `exec NAME --runtime --save-dir PATH` to select a separate writable directory.

For reproducible input, add `--replay /tmp/replay.json --epoch-ms 0`. The replay
file is a JSON array with absolute session timestamps in milliseconds. Actions
at equal timestamps run in file order. The host ticks at time zero and advances
in steps of at most `--step-ms`, ending exactly at each action timestamp. An
optional `--advance-ms` must be at least the final action timestamp and advances
the session after the replay.

For example, save this as `/tmp/replay.json`:

```json
[
  {"at_ms": 1000, "action": "checkpoint", "name": "before", "inspect": ["Window.mainWindow.visible"], "frame": "before.png"},
  {"at_ms": 1100, "action": "pointer_move", "x": 640, "y": 360},
  {"at_ms": 1120, "action": "pointer_down", "x": 640, "y": 360, "shift": 8},
  {"at_ms": 1140, "action": "click", "x": 640, "y": 360},
  {"at_ms": 1140, "action": "pointer_up", "x": 640, "y": 360},
  {"at_ms": 2000, "action": "checkpoint", "name": "after", "frame": "after.png"},
  {"at_ms": 2000, "action": "assert", "expression": "Window.mainWindow.visible"}
]
```

```sh
cargo run -p krkrz_cli --bin krkrz_tool -- \
  --project-dir /path/to/otome_domain --profile otome-domain \
  exec startup.tjs --runtime --save-dir /tmp/replay-save --budget 1000000000 \
  --epoch-ms 0 --replay /tmp/replay.json --advance-ms 2200
```

Coordinates are client pixels; choose them from the captured frame for the
screen being tested. Every action accepts `window` (a TJS expression, default
`Window.mainWindow`). Pointer actions use `x`, `y`, optional `shift` (default
zero), and down/up accept `button` (0 left, 1 right, 2 middle; default 0).
`click` and `double_click` are explicit events; down/up do not synthesize them.
The example follows the native host's down, click, up ordering. Other actions
are `pointer_leave`, `wheel` (`x`, `y`, `delta`, optional `shift`),
`key_down`/`key_up` (numeric Kirikiri virtual `key`, optional `shift`),
`text` (`text` string), and `focus` (`focused` boolean).
Each `checkpoint` emits and flushes one JSON line to stderr, including the
evaluated `inspect` expressions and optional PNG `frame` path. Relative frame
paths resolve beside the replay file. Frame files must be new and outside the
game installation; parent directories must exist. An `assert` evaluates its
TJS `expression` and fails the command if false. Checkpoints preserve evidence
before a later failure; stdout retains the usual result or `--inspect` output.
Replay syntax, timestamp order, and frame destinations are checked before startup.

The checked-in Otome Domain replays cover opening dialogue and saved games.
Run `tools/replays/otome-save-opening.json` with a fresh, separate `--save-dir`,
then run `tools/replays/otome-load-opening.json` in a new process with that same
directory. Both use `--epoch-ms 0 --step-ms 16 --budget 100000000000` and the
release build. The first writes slot zero through the Save menu; the second
restores it through Load and advances one dialogue entry. To test the supplied
Windows choice save instead, copy its saves into a separate directory and use
`tools/replays/otome-load-existing-save0.json`. These replays write to their
selected save directory, so keep it separate from original saves.
`tools/replays/otome-settings-auto.json` checks message speed, BGM volume and
mute controls, and automatic dialogue advance through the original menus.
It uses a fresh save directory and the installation's default settings.
`tools/replays/otome-backlog-auto.json` checks dialogue history, Escape to
close it, and automatic playback afterward.
`tools/replays/otome-gallery-music.json` uses copied saves with Gallery unlocked
to check track selection, pause/resume, next track, stop, and return to title.

An unfiltered `verify` deliberately returns failure for the installation's
malformed protection-notice entry. It verifies all other resolved resources
before reporting the failure. The entry is not hidden or treated as valid.

## Native Linux host

```sh
cargo run --release -p krkrz_cli --bin krkrz_engine -- \
  --project-dir /path/to/otome_domain \
  --save-dir /path/outside/the/game/saves \
  --budget 100000000000
```

`--project-dir` defaults to the current directory. The executor selects the
Otome Domain Cx profile when `otomedomain.exe` is present; `--otome-domain`
selects it explicitly. It never executes the Windows executable or DLLs.

The native window automatically uses the icon embedded in `otomedomain.exe`.
Other projects use an executable matching the directory name, or the only EXE
in that directory. `--icon /path/to/icon.ico` overrides this choice (PNG, JPEG,
BMP and TLG also work). On Wayland this uses `xdg_toplevel_icon_v1` when the compositor
supports it. The frontend uses winit **0.31.0-beta.3**, the latest prerelease,
because stable 0.30.13 does not include that protocol. See
[dependency validation](docs/dependency-upgrade.md) for compatibility checks.

The save directory defaults to `$XDG_DATA_HOME/krkrz_rs/<project-path-hash>`
(or `$HOME/.local/share/krkrz_rs/<project-path-hash>`). Save paths within the
installation are rejected. Native Array/Dictionary structured saves, Array text
load/save, and the standalone saveStruct plugin now write through a separate
overlay. Game-relative writes are redirected there; reads prefer that overlay.
Directories are created when data is written. Existing Windows saves remain
untouched. Original game save/load acceptance is still open.

Use an optimized build for the original game; the debug interpreter is much slower.
Optional `--trace /path/outside/the/game/new-trace.json` records execution for diagnosis.

The engine currently returns an explicit error for unsupported source/native
operations or compiled scripts. The
instruction budget covers nested script calls and dynamic evaluation. Optional
traces retain source location and operation; trace files must be new files
outside the installation.

## Implementation status

| Crate | Implemented | Still required for the slice |
|---|---|---|
| `krkrz_core` | Storage-name normalization, resource limits, source/input types, separate save path selection | Full Kirikiri URI/media normalization |
| `krkrz_assets` | Chained XP3 indexes, compressed/raw segments, range reads, bounded segment caches, Cx interpreter/profile, patch ordering, explicit search paths, case-insensitive loose-file reads, qualified archive paths, text decoding/encoding, PSB v2, PNG/JPEG/TLG5/TLG6, integer WAVE/Vorbis PCM, SLI loops and labels, AlphaMovie rectangle pixel decoding | Remaining video codecs |
| `krkrz_tjs` | UTF-16 values, object dispatch, arrays/dictionaries, functions and bound contexts, classes/inheritance, properties, exceptions, switch/do/while/for control flow, comma/swap and eager logical-assignment operators, global unary-dot lookup, ordered class initializers and deferred base resolution, explicit invalidation/finalization, interpolation, default parameters, argument forwarding/rest/spread, octet values, `instanceof`, preprocessing, RegExp, native class/accessor registration, constant containers, hexadecimal reals, structured serialization, register interpreter, budgets, exact-engine differential tests | Full grammar and built-ins, function hoisting, exact reference-counted finalization timing, original bytecode loader |
| `krkrz_kag` | UTF-16 KAGParserEx tokenization, ordered attributes, labels, escaped brackets and multiline tags; runtime bindings provide expressions, macros, conditions, jumps/calls and label-based state restoration | Complete game-framework integration and replay validation |
| `krkrz_runtime` | Shared session services, nested script execution, storage/script/debug natives, isolated save overlay, ScriptsEx/saveStruct/CSVParser components, PSBFile views, TextRenderBase layout/timing, GdiPlus geometry, AlphaMovie playback/Layer delivery, MenuItem state/tree/click dispatch, WIN32Dialog templates/bounded buffers/closed state, engine constants, application locks, bounded decoded-image cache, Window state and deferred resize callbacks, WaveSoundBuffer decoding/control/fade state and getSample, deterministic scheduler, Fontations/Zeno font rasterization and Layer.drawText, image loading/blits, CPU source-over compositor, PCM loop mixer with conditional links/labels/crossfades | Kirikiri object/plugin APIs, framework integration, remaining transitions/effects/callback integration, SLI label expressions, original saves and replay |
| `krkrz_cli` | Archive/script/PSB/KAG/media inspection, extraction, verification, deterministic input replay and assertions, frame capture, native winit/wgpu window and CPAL audio | Frame/audio comparison against the original engine, complete native playthrough |

The native host presents CPU-composited frames through wgpu and forwards winit
input into the shared session. Use `--no-audio` to disable device output and
`--epoch-ms` to fix the script clock origin. CPAL plays the session's 48 kHz stereo
mix, including gain, pan, resampling, SLI labels and EOF callbacks. The separate
`Session::render_sound_source` API returns source-rate unscaled PCM for inspection.
The getSample replacement adds lazy sample settings and peak-square/legacy
measurements through scoped visualization buffers. Script-visible SLI flags and cached label dictionaries are connected to the
source streams. Negative look-ahead, 3D controls and hardware playback remain open.

The interactive and replay hosts collect unreachable TJS objects between ticks.
Collection traces script graphs and native references, runs finalizers, and keeps
object IDs unique for the session. The 100,000-object limit applies to live
objects. Embedders call `Session::collect_garbage` explicitly and must supply any
Values they retain outside the session. Collection is deferred to host safe
points; it does not reproduce immediate reference-counted finalization timing.
The interpreter deliberately rejects unsupported constructs; it does
not replace TJS with JavaScript. KAG cursor restoration is not a game save.

The standalone `ScriptsEx.dll` replacement now provides dictionary reflection,
object-context inspection, flagged property access, structural comparison,
recursive cloning, array/dictionary callbacks and deferred rehashing. It follows
the community source and the installed PackinOne component's behavior. Array
member reflection and octet MD5 remain explicit unsupported operations, as do
some object kinds in `foreach`. PackinOne registration now combines the three
implemented components and declares observed exports for unfinished operations.
Process/TemporaryFiles construction and unfinished methods fail explicitly.
Layer now supplies image buffers, clipping, pixels, dimensions, sibling ordering,
parent/child links and Font state. Timer and AsyncTrigger share the session event queue.
KAGParserEx now executes expressions, macros, conditions, inline-script callbacks and
scenario calls. Native assign/store/restore follow the original label-based format
and have differential and fresh-session serialization tests.
After PackinOne has loaded, `layerExImage.dll` resolves to its existing exports,
as in the installed engine. The image-effect operations still need implementation.

The `saveStruct.dll` component provides `save2`, `saveStruct2` and
`toStructString`. Native saves use Kirikiri UTF-16, optional text cipher/zlib
modes and byte offsets. The plugin writes UTF-8 or Japanese CP932. Twenty-five
synthetic serialization cases match the original engine and plugin, including
saved bytes (decompressed content for zlib). This does not establish original
game save compatibility. Unsupported CP932 fallback characters and plugin
cycles fail explicitly. Failed serialization leaves the previous save intact;
upstream can leave a partial file on some error paths.

The `csvParser.dll` component supplies CSVParser instances and subclasses,
UTF-16 string input, UTF-8/CP932 storage input, custom separators, multiline
fields, logical line numbers and `doLine` callbacks. It follows the older
PackinOne API; the second storage argument is a boolean, not a text-stream
mode. Explicit invalidation releases parser state, including during row callbacks.
Invalid legacy encoding replacement rules and automatic object collection remain
open. Startup can now load these components together through PackinOne.

The `psbfile.dll` replacement exposes PSB v2 documents as read-only TJS views,
including arrays, dictionaries and resource octets. Thirty-six cases match the
installed plugin, including identity, reflection, cloning, invalidation and
serialization. The original `StorageData` script reads a binary game scenario
through this interface. Stale views fail safely after their owner is invalidated.
This does not establish story playback or original save compatibility.

System constants match the installed engine, including its older stretch-mode
exports. Graphic-cache limits are bytes and affect a bounded LRU cache; Layer
decoding/rendering integration remains open. Application locks use per-user
files under `$XDG_CACHE_HOME/krkrz_rs/app-locks` (or `$HOME/.cache`) and release
when the Session ends. These locks coordinate Rust processes, not Windows/Wine.

## Installed-game validation

Synthetic fixtures run by default. To opt into tests that use your installation:

```sh
KRKRZ_PROJECT_DIR=/path/to/otome_domain \
  cargo test -p krkrz_assets --test installed_game -- --ignored --nocapture
KRKRZ_PROJECT_DIR=/path/to/otome_domain \
  cargo test -p krkrz_runtime --test installed_game -- --ignored --nocapture
```

See [community-implementations.md](docs/community-implementations.md) for the
community source map and differential-test command, [validation.md](docs/validation.md) for observed results and outstanding
acceptance work, [references.json](docs/references.json) for pinned research
sources, and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for adapted code
notices. Proprietary scripts, scenarios, media and saves are not included.

The `textrender.dll` replacement implements native layout, style and timing
state, font-metric callbacks, basic ruby, wrapping, alignment, character snapshots
and key-wait positions. Eighty synthetic cases match the installed DLL. The
unchanged TextRender wrapper runs with fixed test metrics; actual font metrics,
glyph rendering and dialogue presentation still require integration. Vertical and uneven ruby
groups, multiple hanging punctuation characters, graphical characters and several
layout options remain explicit unsupported operations.

The `layerExDraw.dll` replacement registers the installed interface and implements
GdiPlus PointF, RectF and Matrix operations. Its synthetic corpus has 32 geometry
and registration cases plus seven reached TJS language cases. GDI+ fonts, images,
paths, appearance resources and Layer drawing remain explicit unsupported calls.
Plugin loading alone does not establish rendered output.

The AlphaMovie binding decodes AJPM rectangles, delivers their pixels and position
to Layers, and supports playback, looping and frame seeks. All 160 packets in the
four installed AMVs decode. Synthetic DCT/DC and DEFLATE alpha pixels match the
original DLL; the reused integer IDCT differs by at most two RGB levels and one
alpha level in the AC fixture. Decoding is synchronous, so `frame` reports the
last decoded sequence rather than the original worker's preload cursor. Queued
movies (`setNextMovieFile`) remain an explicit unsupported operation.
`krkrz_tool amv NAME` inspects the container and rectangle-packet index.

The menu binding follows the public `krkrz/menu` source and original-DLL probes.
It supports item properties, insertion and display ordering, radio groups, child
array caching, shortcut conversion, and deferred click callbacks. Native menu
bars, popups and OS shortcut delivery are not implemented. Shortcut name tables
use stable English defaults until a platform keyboard adapter is available.

The WIN32Dialog binding registers the installed plugin's observed constants and
interfaces. It supports closed-dialog state, Header/Items storage and copied
binary templates, and bounded Blob byte/word/dword access. Native dialogs,
controls, OS handles, drawing, message boxes and pointer-based buffer operations
remain explicit unsupported operations.

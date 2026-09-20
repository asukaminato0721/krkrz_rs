# Rust Kirikiri Z compatibility engine

This repository contains an initial implementation toward the Otome Domain
playable slice. **The game is not playable yet.** The original `startup.tjs`
now completes framework initialization, including game menus, embedded fonts,
message layers and audio objects. Deterministic ticks deliver timer, asynchronous,
continuous and paint callbacks. CPU layer composition and crossfade transitions now render the warning screens
after ten seconds of deterministic ticks. Further execution enters `title.ks`
and currently stops at the missing TJS `Date` class during autosave.
Native presentation, required movies/effects, title interaction, New Game,
first choice and original save/load acceptance remain incomplete.

## Build and verify

```sh
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

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

An unfiltered `verify` deliberately returns failure for the installation's
malformed protection-notice entry. It verifies all other resolved resources
before reporting the failure. The entry is not hidden or treated as valid.

## Startup executor

```sh
cargo run -p krkrz_cli --bin krkrz_engine -- \
  --project-dir /path/to/otome_domain \
  --save-dir /path/outside/the/game/saves \
  --budget 100000000 --trace /path/outside/the/game/new-trace.json
```

`--project-dir` defaults to the current directory. The executor selects the
Otome Domain Cx profile when `otomedomain.exe` is present; `--otome-domain`
selects it explicitly. It never executes the Windows executable or DLLs.

The save directory defaults to `$XDG_DATA_HOME/krkrz_rs/<project-path-hash>`
(or `$HOME/.local/share/krkrz_rs/<project-path-hash>`). Save paths within the
installation are rejected. Native Array/Dictionary structured saves, Array text
load/save, and the standalone saveStruct plugin now write through a separate
overlay. Game-relative writes are redirected there; reads prefer that overlay.
Directories are created when data is written. Existing Windows saves remain
untouched. Original game save/load acceptance is still open.

The engine currently returns an explicit error for unsupported source/native
operations, compiled scripts, or the absent native presentation loop. The
instruction budget covers nested script calls and dynamic evaluation. Optional
traces retain source location and operation; trace files must be new files
outside the installation.

## Implementation status

| Crate | Implemented | Still required for the slice |
|---|---|---|
| `krkrz_core` | Storage-name normalization, resource limits, source/input types, separate save path selection | Full Kirikiri URI/media normalization |
| `krkrz_assets` | Chained XP3 indexes, compressed/raw segments, range reads, bounded segment caches, Cx interpreter/profile, patch ordering, explicit search paths, case-insensitive loose-file reads, qualified archive paths, text decoding/encoding, PSB v2, PNG/JPEG/TLG5/TLG6, integer WAVE/Vorbis PCM, SLI loops and labels, checked AlphaMovie containers | MPEG/AlphaMovie pixel decoding |
| `krkrz_tjs` | UTF-16 values, object dispatch, arrays/dictionaries, functions and bound contexts, classes/inheritance, properties, exceptions, switch/do/while/for control flow, comma/swap and eager logical-assignment operators, global unary-dot lookup, ordered class initializers and deferred base resolution, explicit invalidation/finalization, interpolation, default parameters, argument forwarding/rest/spread, octet values, `instanceof`, preprocessing, RegExp, native class/accessor registration, constant containers, hexadecimal reals, structured serialization, register interpreter, budgets, exact-engine differential tests | Full grammar and built-ins, function hoisting, automatic object finalization/collection, original bytecode loader |
| `krkrz_kag` | UTF-16 KAGParserEx tokenization, ordered attributes, labels, escaped brackets and multiline tags; runtime bindings provide expressions, macros, conditions, jumps/calls and label-based state restoration | Complete game-framework integration and replay validation |
| `krkrz_runtime` | Shared session services, nested script execution, storage/script/debug natives, isolated save overlay, ScriptsEx/saveStruct/CSVParser components, PSBFile views, TextRenderBase layout/timing, GdiPlus geometry, AlphaMovie metadata/transport controls, MenuItem state/tree/click dispatch, WIN32Dialog templates/bounded buffers/closed state, engine constants, application locks, bounded decoded-image cache, Window state and deferred resize callbacks, WaveSoundBuffer decoding/control/fade state and getSample, deterministic scheduler, private FreeType font rasterization and Layer.drawText, image loading/blits, CPU source-over compositor, PCM loop mixer with conditional links/labels/crossfades | Kirikiri object/plugin APIs, framework integration, wgpu/winit/CPAL, transitions/effects/callback integration, SLI label expressions, original saves and replay |
| `krkrz_cli` | Archive/script/PSB/KAG/media inspection, extraction, verification, static inventory, primitive evaluation, startup diagnostics | Deterministic interactive replay, input-driven checkpoints, frame/audio comparison, native play |

The compositor, scheduler, Window state and PCM mixer are tested components, not yet a game player. Window support currently covers construction, caption, visibility, client size, position, primary-layer lookup and resize/action callbacks in headless sessions. Explicit invalidation releases its native state and cancels pending callbacks. It does not create an OS window.
WaveSoundBuffer opens integer PCM WAV and Ogg Vorbis storage, reads SLI metadata,
and exposes decoded streams through `Session::render_sound_source`. Source pulls
support pause, seek, looping, labels and deferred EOF callbacks. Volume/pan/fade
controls exist, but device mixing, gain, resampling and audio-clock integration
are still required. This source-rate API returns unscaled PCM, not device output.
The getSample replacement adds lazy sample settings and peak-square/legacy
measurements through scoped visualization buffers. Script-visible SLI flags and cached label dictionaries are connected to the
source streams. Negative look-ahead, 3D controls and hardware playback remain open.
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

The AlphaMovie binding opens checked AJPM storage and implements metadata and
transport settings. It does not decode movie pixels or write Layers. Frame
seeking, movie queuing, positioning and `showNextImage` fail explicitly.
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

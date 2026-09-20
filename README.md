# Rust Kirikiri Z compatibility engine

This repository contains an initial implementation toward the Otome Domain
playable slice. **The game is not playable yet.** Original startup now
executes its framework, including preprocessing and archive setup. Execution
currently stops at `system/Initialize.tjs:301` because `PackinOne.dll` has no
registered Rust replacement yet. The title,
New Game, first choice, scene transition, and original save/load acceptance
criteria have not been met. Execution failures never count as checkpoints.

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

An unfiltered `verify` deliberately returns failure for the installation's
malformed protection-notice entry. It verifies all other resolved resources
before reporting the failure. The entry is not hidden or treated as valid.

## Startup executor

```sh
cargo run -p krkrz_cli --bin krkrz_engine -- \
  --project-dir /path/to/otome_domain \
  --save-dir /path/outside/the/game/saves \
  --budget 1000000 --trace /path/outside/the/game/new-trace.json
```

`--project-dir` defaults to the current directory. The executor selects the
Otome Domain Cx profile when `otomedomain.exe` is present; `--otome-domain`
selects it explicitly. It never executes the Windows executable or DLLs.

The save directory defaults to `$XDG_DATA_HOME/krkrz_rs/<project-path-hash>`
(or `$HOME/.local/share/krkrz_rs/<project-path-hash>`). Save paths within the
installation are rejected. No save serializer is implemented yet, and startup
does not create a save directory. Existing Windows saves remain untouched.

The engine currently returns an explicit error for unsupported source/native
operations, compiled scripts, or the absent native presentation loop. The
instruction budget covers nested script calls and dynamic evaluation. Optional
traces retain source location and operation; trace files must be new files
outside the installation.

## Implementation status

| Crate | Implemented | Still required for the slice |
|---|---|---|
| `krkrz_core` | Storage-name normalization, resource limits, source/input types, separate save path selection | Full Kirikiri URI/media normalization |
| `krkrz_assets` | Chained XP3 indexes, compressed/raw segments, range reads, bounded segment caches, Cx interpreter/profile, patch ordering, explicit search paths, case-insensitive loose-file reads, qualified archive paths, text decoding, PSB v2, PNG/JPEG/TLG5, Vorbis PCM, SLI loops and labels | TLG6, PSB plugin object bindings, font integration, MPEG/AlphaMovie |
| `krkrz_tjs` | UTF-16 values, object dispatch, arrays/dictionaries, functions and bound contexts, classes/inheritance, properties, exceptions, loops, interpolation, argument forwarding/rest/spread, `instanceof`, preprocessing, RegExp, native class/accessor registration, register interpreter, budgets, exact-engine differential tests | Full grammar and built-ins, function hoisting/default arguments, object finalization/collection, original bytecode loader |
| `krkrz_kag` | Ordered tags/attributes, labels, text/escaped brackets, multiline tags, explicit jump/call/return and restorable cursor | Macros, conditionals, parameter expansion, script-driven parser bindings, complete KAGParserEx state |
| `krkrz_runtime` | Shared session services, nested script execution, storage/script/debug natives, Window state and deferred resize callbacks, deterministic scheduler, CPU source-over compositor, PCM loop mixer with conditional links/labels/crossfades | Kirikiri object/plugin APIs, framework integration, wgpu/winit/CPAL, text/transitions/effects/callback integration, SLI label expressions, original saves and replay |
| `krkrz_cli` | Archive/script/PSB/KAG/media inspection, extraction, verification, static inventory, primitive evaluation, startup diagnostics | Deterministic interactive replay, input-driven checkpoints, frame/audio comparison, native play |

The compositor, scheduler, Window state and PCM mixer are tested components, not yet a game player. Window support currently covers construction, caption, visibility, client size, position, primary-layer lookup and resize/action callbacks in headless sessions. It does not create an OS window.
The interpreter deliberately rejects unsupported constructs; it does
not replace TJS with JavaScript. KAG cursor restoration is not a game save.

The standalone `ScriptsEx.dll` replacement now provides dictionary reflection,
object-context inspection, flagged property access, structural comparison,
recursive cloning, array/dictionary callbacks and deferred rehashing. It follows
the community source and the installed PackinOne component's behavior. Array
member reflection and octet MD5 remain explicit unsupported operations, as do
some object kinds in `foreach`. This component is not yet a replacement for
the complete PackinOne bundle, so original startup still stops at that plugin.

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

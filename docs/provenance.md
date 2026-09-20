# Reference provenance

`references.json` records full commit IDs for Kirikiri Z, its historical
monolithic branch, KAGParserEx, GARbro, Kirikiri SDL2, Kirikiroid2, Habakiri and layerExImage. Research
checkouts are external inputs, not build dependencies. The pins are research
revisions; they have not yet been proven to correspond exactly to the installed
Kirikiri Z 1.2.0.3 / TJS2 2.4.28 binary. That version audit remains open.

Implementation references:

- `krkrz/base/XP3Archive.cpp`, `StorageIntf.cpp`: index chains, segment structure,
  normalization, search-path order. In-archive ASCII case folding is deliberate.
- `GARbro/ArcFormats/KiriKiri/KiriKiriCx.cs`: Cx generation and interpretation.
  Stage fallback preserves PRNG state, instruction-length accounting follows the
  reference, and all arithmetic wraps at 32 bits. No executable x86 is generated.
- `GARbro/ArcFormats/Resources/Formats.dat`: only the `Otome＊Domain` Cx profile
  is bundled. No encrypted game content is bundled.
- `GARbro/ArcFormats/Emote/ArcPSB.cs`: PSB v2 headers, name tries, arrays, strings,
  dictionaries and resource references. Rust adds offset, allocation and depth
  validation. Other PSB versions/encrypted headers are explicit errors.
- `GARbro/ArcFormats/KiriKiri/ImageTLG.cs`: TLG5 channel decompression/composition,
  derived from W.Dee's TLG implementation.
- `krkrz/base/TextStream.cpp`: UTF-16 text modes 0, 1 and zlib-compressed mode 2.
- `krkrz/sound/WaveLoopManager.cpp`, `WaveLoopManager.h`: SLI metadata,
  conditional-link priority, sample-frame labels and integer PCM crossfades.
- `krkrz/tjs2/tjsVariant.cpp`, `tjsVariant.h`: primitive conversion/equality rules.
- `krkrz/tjs2/tjsInterCodeExec.cpp`, `tjsObject.cpp`: exception handling, bound
  contexts, property dispatch, `typeof`, `instanceof`, string methods and explicit
  invalidation/finalization. Native cleanup follows `tjsNative.cpp`; automatic
  reference-counted lifetime is not yet implemented.
- `krkrz/tjs2/syntax/tjspp.y`, `tjsCompileControl.cpp`, `tjsLex.cpp`: eager
  32-bit preprocessing, conditional exclusion and string-literal boundaries.
- `krkrz/tjs2/tjsRegExp.cpp`: RegExp flags, capture arrays, literal replacement
  strings, callback replacement and state. Rust uses the fancy-regex matcher.
- `krkrz/tjs2/tjsNative.cpp`, `visual/WindowIntf.cpp`, `base/EventIntf.h`,
  `base/ScriptMgnIntf.cpp`: native classes, accessor contexts, window events and
  dynamic-script contexts/source positions. `runtime/data/core_constants.tjs`
  adapts its initialization constants: the 1.2.0.3 reference lacks sixteen newer
  stretch names and uses `stRefNoClip=16`, so the Rust exports match that version.
  The system corpus checks 395 numeric constants, sixteen absent names and
  representative `imageTagLayerType` mappings.
- `krkrz/base/SystemIntf.cpp`, `base/win32/SysInitImpl.cpp`,
  `visual/GraphicsLoaderIntf.cpp`: byte-based cache limits and automatic memory
  tiers. Rust uses Linux physical-memory discovery, bounded at 512 MiB.
  `base/win32/SystemImpl.cpp`: application-lock creation/repeated-name behavior;
  Rust uses OS file locks with Session lifetime. Windows mutex interoperation is
  not implemented.
- `krkrz/tjs2/syntax/tjs.y`, `tjsInterCodeGen.cpp`, `tjsLex.cpp`: parenthesized
  casts, mutable const declarations, default arguments, unnamed argument tails,
  comma aliases, trailing omissions, octet parsing, switch/do/while, comma/swap
  evaluation, ordered class-body initialization and deferred superclass getters. Native export declarations
  come from the isolated binary probe; see [native-exports-provenance.json](native-exports-provenance.json).
- `Habakiri/core/src/jp/kirikiri/tjs2`: external execution reference for the
  synthetic TJS corpus. No Java interpreter code is included in the workspace.
  See [community-implementations.md](community-implementations.md) for coverage
  and known reference differences.
- `KAGParserEx/readme.txt`, `KAGParser.cpp`: ordered attributes and multiline tags.

Catalog SHA-256:
`54039fde222592c911536b0bd3f4d931bd580c66afd96eaefece34ea872f2982`

Bundled profile SHA-256:
`24f60420a19d2bb9c3f2e3e9e4e66ed215253abfe0f2a10c9c9bb91cf64fd3c1`

Reproduce the profile using the pinned GARbro catalog and a data-only NRBF reader:

```sh
uv run --with nrbf tools/export_cx_profile.py /cache/GARbro/ArcFormats/Resources/Formats.dat /tmp/otome-profile.json
cmp crates/krkrz_assets/data/otome_domain_cx.json /tmp/otome-profile.json
```

The Python tool is only for provenance reproduction. Rust binaries embed the
small JSON profile and have no Python, .NET, Wine or sibling-checkout dependency.

The optional original-engine oracle copies a user-supplied executable to an
isolated cache and executes only synthetic test scripts in a separate Wine
prefix. The exact tested binary hash and observed versions are recorded in
[validation.md](validation.md). This does not close the upstream source-revision
audit or the full-game original-engine baseline requirement.

The historical `krkrz/krkr2` repository also contains the `scriptsEx`,
`saveStruct` and `csvParser` plugin sources under
`kirikiri2/trunk/kirikiri2/src/plugins/win32`. The pinned `krkr2-plugins`
revision is recorded in `references.json`. The ScriptsEx Rust port follows
that component's source and checks its behavior against the installed PackinOne
bundle. The latter is copied into the isolated oracle directory as `ScriptsEx.dll`
for the comparison; this is the original bundle binary, not an independently
built ScriptsEx DLL. Both engine and DLL SHA-256 values appear in the report.

Native serialization follows upstream `tjsArray.cpp`, `tjsDictionary.cpp`,
`tjsVariant.cpp` and `TextStream.cpp`. Constant-container save syntax also uses
upstream lexical and compiler behavior. Kirikiroid2's `saveStruct.cpp` and the
historical `saveStruct/Main.cpp` provide the plugin reference. Neither checked
source is an exact match for the installed PackinOne component: its string
escaping, real formatting, default line endings and ignored later formatting
options were established by isolated original-plugin comparisons. The Rust
component deliberately exposes only the observed API, not all newer options.

CSVParser follows historical `csvParser/Main.cpp` (Go Watanabe, Kirikiri
license). The installed PackinOne predates that revision's string text-stream
read mode: its second storage parameter is always converted to a boolean.
The Rust component follows the installed version, including quoted-field
continuations, callback context, line numbering, final-CR and byte-0xFF reader
behavior. It does not substitute a generic RFC 4180 CSV library.

The pinned `wamsoft/layerExImage` checkout supplies a reference for six Layer
image effects. Only export registration and the installed PackinOne alias have
been checked so far. No CxImage algorithm from that repository is included in
the Rust workspace. Future adaptations must preserve that code's notices.

WaveSoundBuffer control state and fades follow upstream `sound/WaveIntf.cpp`,
`sound/SoundBufferBaseIntf.cpp`, `sound/win32/WaveImpl.cpp` and the 60 ms beat in
`sound/win32/SoundBufferBaseImpl.h`. The native binding now connects bounded
WAVE/Vorbis decoding and SLI streams to deterministic source pulls. Visualization
mono conversion follows `sound/WaveIntf.cpp`. The independent WAVE reader
supports integer PCM at 8/16/24/32 bits, with checked chunks and frame limits.
Kirikiroid2's `src/plugins/getSample.cpp` supplies the reference for the Rust
getSample binding, lazy defaults, look-ahead and peak-square calculation.
Scoped integer handles replace script-supplied pointers. The legacy method's
unwritten malloc bytes are deliberately replaced by zeros; this safety behavior
is not claimed to match the original. `tone.wav` is synthetic: 4,410 mono frames
at 44,100 Hz, signed 16-bit samples repeating [0, 8192, -16384, 32767], with a
canonical RIFF PCM header. No game audio is bundled.

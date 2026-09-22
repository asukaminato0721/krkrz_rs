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
  come from the isolated binary probe.
- `Habakiri/core/src/jp/kirikiri/tjs2`: external execution reference for the
  synthetic TJS corpus. No Java interpreter code is included in the workspace.
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

The PSBFile TJS binding is independently implemented from isolated original-plugin
observations using synthetic PSB v2 files. The format decoder retains its GARbro
provenance. `tools/make_psb_fixtures.py` builds the fixtures without reading the
installation. Read-only native views preserve fresh nested identities and owner
invalidation; unsafe stale-view access and negative indexes fail explicitly.
SaveStruct handling of these views follows the observed plugin behavior and
Kirikiroid2's native Array count accessor versus generic dictionary enumeration.

Two TextRender replacements were located and pinned: `3c1u/TextRender` (MIT or
Apache-2.0) and AetherKiri's `cpp/plugins/textrender.cpp`. Their APIs and defaults
differ from the installed plugin. The Rust TextRenderBase binding is an independent
implementation based on synthetic original-DLL observations; these community
implementations supplied research leads, not copied code. The binding preserves
observed float32 arithmetic, callback arguments, native-array identity,
alignment, repeated finalization and appended-text timing. It does not yet supply
glyph rasterization or the complete layout API.

The layerExDraw source was found in `wamsoft/layerExDraw`, pinned at
`c87f273a0b4e0b27bcf2d98e8e6de469d2716888`. Its `main.cpp`, `Path.cpp` and
`LayerExDraw.cpp` provide GDI+ registration and drawing references. The Rust
binding now implements PointF, RectF and Matrix and declares other installed
exports with explicit errors for their unimplemented operations. Registration
and behavior are checked against the installed DLL, whose API omits newer
community exports. Numeric conversion follows the pinned Kirikiroid2
`src/plugins/ncbind/ncbind.hpp`, including repeated property reads and copied
output parameters. The source's Kirikiri license is noted in
`THIRD_PARTY_NOTICES.md`; drawing algorithms have not been ported.

Logical compound assignments and unary-dot scope resolution follow upstream
`tjsInterCodeGen.cpp`. Original-engine cases confirm eager RHS-first assignments
and global lookup for unary dot outside a lexical `with`, including functions
defined inside `with`. These behaviors let the original StandImage and particle
framework definitions compile without modifying game scripts.

AlphaMovie container layout research used `xmoezzz/amv_decoder` revision
`9f5da0194b9dc4d4e45313243e9eb05a8e040104` (`src/amv.rs`, MPL-2.0).
The checked reader and native binding are independent implementations; no MPL
source was copied or linked. Original-DLL probes establish the FPS field order,
properties, settings and transport state. The older `AlphaMovieDecoder` repository
is a TJS front end to bundled original DLLs; neither its binaries nor its script
are adopted. Only synthetic AMV data generated by `tools/make_amv_fixtures.py`
is checked in. Extracted game media remains in the external cache.

MenuItem state and event behavior follow `krkrz/menu` revision
`6818626cd4df71fa318412e2148e1d731b7d6662`, especially `MenuItemIntf.cpp`,
`WindowMenu.cpp`, `Main.cpp` and `ObjectList.h`. The Rust port preserves the
Kirikiri notice in `THIRD_PARTY_NOTICES.md`; Windows presentation code is not
linked or executed by the Rust engine. Synthetic tests compare the installed
DLL's behavior. Keyboard names in those tests are controlled because the source
uses Windows keyboard-layout APIs. Bound callback identity and string-escape
corrections follow Kirikiri Z `tjsInterCodeExec.cpp` and `tjsLex.cpp`.
Built-in registration of `MenuItem`, `Window.menu` and `KAGParser` follows the Kirikiri 2
compatibility surface in the pinned Kirikiroid2 `src/core/base/ScriptMgnIntf.cpp`.
The first explicit menu plugin link adopts these existing classes; subsequent
links under different spellings retain the tested re-registration behavior.
Built-in ordinary menu items accept any object (including null) as their action
owner, following Kirikiroid2 `src/core/visual/MenuItemIntf.cpp`; a root's second
argument must be a Window. Explicitly linking menu.dll retains that plugin's
additional Window requirement for ordinary action owners.

The legacy read-only `Debug.console` accessor and its nested `Console` class
follow Kirikiroid2 `src/core/utils/win32/DebugImpl.cpp`. As in that port, `visible`
always reads false and accepts writes without opening a native console window.
The runtime also declares the legacy `Pad` class so windowEx can register its
extensions; constructing a Pad editor remains explicitly unsupported. WindowEx
resolves nested accessors before registering extensions. Synthetic fixtures use
conditional Pad/console fallbacks so they work with both this core surface and
the original Kirikiri Z reference that needs compatibility placeholders.

The dialog source reference is `wtnbgo/win32dialog` revision
`9658169f6af0159adb2739d22d9a6ebb8cec4981`, by miahmie, under the Kirikiri
license. `main.cpp`, `dialog.hpp`, `dialog_config.hpp` and the pinned ncbind
converters guide the Rust registration/data implementation. Native template fields
and binary layout are adapted with the notice in `THIRD_PARTY_NOTICES.md`.
Direct member probes against the installed DLL establish the supported export
profile, including seven legacy constants absent from the newer source. Export
candidate probing is not exhaustive class enumeration (the DLL rejects that).
No resource DLL or platform GUI code is executed by Rust.

The next Window-extension reference is `wtnbgo/windowEx` revision
`88c9be22ff8f9e6d42edbf4787092837f177a89a`, by miahmie. Its source is cached
outside the repository and has not been ported yet.
# Synthetic Windows Media fixture

`crates/krkrz_runtime/tests/fixtures/video_overlay.wmv` contains generated test
bars and a 440 Hz sine wave, with WMV2/WMA2 codecs in ASF. It contains no game data.
Generate it with:

```sh
ffmpeg -v error -nostdin \
  -f lavfi -i 'testsrc2=size=32x24:rate=25:duration=0.48' \
  -f lavfi -i 'sine=frequency=440:sample_rate=44100:duration=0.48' \
  -map 0:v -map 1:a -c:v wmv2 -b:v 200k -c:a wmav2 -b:a 64k \
  -fflags +bitexact -flags:v +bitexact -flags:a +bitexact -map_metadata -1 \
  -f asf crates/krkrz_runtime/tests/fixtures/video_overlay.wmv
```

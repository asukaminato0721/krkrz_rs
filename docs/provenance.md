# Reference provenance

`references.json` records full commit IDs for Kirikiri Z, its historical
monolithic branch, KAGParserEx, GARbro, Kirikiri SDL2 and Kirikiroid2. Research
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

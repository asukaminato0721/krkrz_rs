# Third-party notices

The Cx interpreter, Otome Domain Cx profile, PSB reader and TLG5 decoder are adapted from
GARbro, commit `b09ee4570ccb1daf6ac56710ee8934dc0b8baeb0`.
Sources: `ArcFormats/KiriKiri/KiriKiriCx.cs`, `ArcFormats/Resources/Formats.dat`,
`ArcFormats/Emote/ArcPSB.cs`, and `ArcFormats/KiriKiri/ImageTLG.cs`. Research checkouts and proprietary game data
are not included.

Copyright (C) 2014-2016 by morkt

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to
deal in the Software without restriction, including without limitation the
rights to use, copy, modify, merge, publish, distribute, sublicense, and/or
sell copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS
IN THE SOFTWARE.




## Kirikiri

TLG5 decoding derives from W.Dee’s TLG implementation through GARbro.
Text stream decoding and TJS value, object, and interpreter semantics follow
Kirikiri Z (`tjs2/tjsVariant.cpp`, `tjsInterCodeExec.cpp`, `tjsObject.cpp`).
Preprocessor, regex binding and native-class/window event behavior also follow
`tjsCompileControl.cpp`, `tjsInterCodeGen.cpp`, `syntax/tjs.y`, `tjsLex.cpp`,
`tjsRegExp.cpp`, `tjsNative.cpp`,
`visual/WindowIntf.cpp` and `base/EventIntf.h`. Engine constants are adapted from
`base/ScriptMgnIntf.cpp` (Copyright (C) 2000 W.Dee <dee@kikyou.info> and
contributors). Cache-limit behavior follows `visual/GraphicsLoaderIntf.cpp`
and `base/win32/SysInitImpl.cpp`; application-lock behavior follows
`base/win32/SystemImpl.cpp`.
ScriptsEx reflection, structural operations and traversal derive from
`krkrz/krkr2`, revision `dec49af97e174d31059c3ccd7efc700ba3c6b788`,
`kirikiri2/trunk/kirikiri2/src/plugins/win32/scriptsEx/Main.cpp`.
Its authors are Go Watanabe / ゆんゆん探偵; its readme specifies the
Kirikiri license reproduced below. Member enumeration and deferred rehashing
follow Kirikiri Z `tjs2/tjsObject.cpp` and `tjs2/tjsHashSearch.h`.
Native structured serialization, hexadecimal real formatting, constant literals
and text stream encoding follow Kirikiri Z `tjs2/tjsArray.cpp`,
`tjs2/tjsDictionary.cpp`, `tjs2/tjsVariant.cpp`, `tjs2/tjsLex.cpp` and
`base/TextStream.cpp`. The saveStruct component follows the historical
`saveStruct/Main.cpp` and Kirikiroid2 `src/plugins/saveStruct.cpp`, adjusted
against the installed plugin's output. The historical plugin lists Go Watanabe
and miahmie as authors and specifies the same Kirikiri license below.
CSVParser parsing, row callbacks and stream boundaries follow historical
`csvParser/Main.cpp`, by Go Watanabe, under the same Kirikiri license. The
installed PackinOne storage-mode behavior was established by comparison.
Habakiri is used only as an external differential-test reference. Its Java
sources are not bundled or linked into the Rust engine.
SLI parsing and PCM loop mixing are adapted from Kirikiri Z
`sound/WaveLoopManager.cpp` and `sound/WaveLoopManager.h`. Sound control state
and fades follow `sound/WaveIntf.cpp`, `sound/SoundBufferBaseIntf.cpp`,
`sound/win32/WaveImpl.cpp` and `sound/win32/SoundBufferBaseImpl.h`.
The getSample replacement follows Kirikiroid2
`src/plugins/getSample.cpp`, revision `d1c2b1259423542c893e0b65eaeb46c848848f2b`,
under the Kirikiri notice below. Visualization downmix follows Kirikiri Z
`sound/WaveIntf.cpp`. No original plugin binary is bundled or executed by Rust.
Original TLG copyright: Copyright (C) 2000-2005 W.Dee and contributors.
C# TLG port by morkt. The applicable Kirikiri notice follows.

Copyright (c), W.Dee and contributors All rights reserved.
Contributors
 Go Watanabe, Kenjo, Kiyobee, Kouhei Yanagita, mey, MIK, Takenori Imoto, yun
Kirikiri Z Project Contributors
W.Dee, casper, 有限会社MCF, Biscrat, 青猫, nagai, ルー, 高際 雅之, 永劫,
ゆんゆん探偵, りょうご（今は無きあの星）, AZ-UME, 京 秋人,
Katsumasa Tsuneyoshi, 小池潤, miahmie, サークル獏, アザナシ, はっしぃ,
棚中製作所, わっふる/waffle, ワムソフト, TYPE-MOON, 有限会社エムツー,
Takenori Imoto
Kirikiri Z 64bit Project Contributors
合資会社ワムソフト, Takenori Imoto, 他
----------------------------------------------------------------------------
ソースコード形式かバイナリ形式か、変更するかしないかを問わず、以下の条件を満
たす場合に限り、再頒布および使用が許可されます。

・ソースコードを再頒布する場合、上記の著作権表示、本条件一覧、および下記免責
  条項を含めること。
・バイナリ形式で再頒布する場合、頒布物に付属のドキュメント等の資料に、上記の
  著作権表示、本条件一覧、および下記免責条項を含めること。
・書面による特別の許可なしに、本ソフトウェアから派生した製品の宣伝または販売
  促進に、組織の名前またはコントリビューターの名前を使用してはならない。

本ソフトウェアは、著作権者およびコントリビューターによって「現状のまま」提供
されており、明示黙示を問わず、商業的な使用可能性、および特定の目的に対する適
合性に関する暗黙の保証も含め、またそれに限定されない、いかなる保証もありませ
ん。著作権者もコントリビューターも、事由のいかんを問わず、損害発生の原因いか
んを問わず、かつ責任の根拠が契約であるか厳格責任であるか（過失その他の）不法
行為であるかを問わず、仮にそのような損害が発生する可能性を知らされていたとし
ても、本ソフトウェアの使用によって発生した（代替品または代用サービスの調達、
使用の喪失、データの喪失、利益の喪失、業務の中断も含め、またそれに限定されな
い）直接損害、間接損害、偶発的な損害、特別損害、懲罰的損害、または結果損害に
ついて、一切責任を負わないものとします。

## layerExDraw and ncbind

The Rust layerExDraw API and geometry binding follow Wamsoft's layerExDraw,
commit `c87f273a0b4e0b27bcf2d98e8e6de469d2716888`, by Go Watanabe
(わたなべごう). Its readme specifies the Kirikiri license reproduced above.
Sources consulted: `main.cpp` and `Path.cpp`. Native converter behavior also
follows `src/plugins/ncbind/ncbind.hpp` in the pinned Kirikiroid2 checkout.
The binding is independently written in Rust and verified with synthetic
original-plugin observations. No GDI+, C++, Wine or Windows binary is included
or required by the Rust engine. Drawing algorithms have not been ported yet.

## MenuItem

The Rust MenuItem model is adapted from `krkrz/menu`, revision
`6818626cd4df71fa318412e2148e1d731b7d6662`, by W.Dee and contributors.
Sources: `MenuItemIntf.cpp`, `MenuItemIntf.h`, `WindowMenu.cpp`, `WindowMenu.h`,
`Main.cpp` and `ObjectList.h`. Copyright (C) 2000 W.Dee <dee@kikyou.info> and
contributors. The Kirikiri license reproduced above applies. No Windows native
menu code or original plugin binary is linked into the Rust engine.

## WIN32Dialog

The Rust WIN32Dialog template/data implementation follows `wtnbgo/win32dialog`,
revision `9658169f6af0159adb2739d22d9a6ebb8cec4981`, by miahmie.
Sources: `main.cpp`, `dialog.hpp` and `dialog_config.hpp`. The project's readme
specifies the Kirikiri license reproduced above. ncbind conversion and property
access behavior follows the pinned Kirikiroid2 `ncbind.hpp`. Registration values
and behavioral fixtures are observations of the installed plugin. No Windows
plugin binary or platform UI code is linked into the Rust engine.

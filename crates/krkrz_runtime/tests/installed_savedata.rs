//! Optional codec acceptance against copies of existing Windows save files.
//! Does not start the game, acquire its application lock, or write its saves.
use anyhow::{Context, Result, ensure};
use krkrz_runtime::Session;

#[test]
#[ignore = "requires KRKRZ_SAVEDATA_DIR pointing to existing Otome Domain Windows saves"]
fn original_windows_saves_roundtrip_without_value_changes() -> Result<()> {
    let source = std::path::PathBuf::from(
        std::env::var_os("KRKRZ_SAVEDATA_DIR").context("set KRKRZ_SAVEDATA_DIR")?,
    );
    let project = tempfile::tempdir()?;
    let saves = tempfile::tempdir()?;
    let names = [
        "data0.bmp",
        "data1.bmp",
        "data2.bmp",
        "data3.bmp",
        "data4.bmp",
        "data_anchor.ksd",
        "datasc.ksd",
        "datasu.ksd",
    ];
    let mut cases = String::new();
    for (i, name) in names.iter().enumerate() {
        let bytes = std::fs::read(source.join(name)).with_context(|| format!("read {name}"))?;
        let offset = if bytes.starts_with(b"BM") {
            u32::from_le_bytes(bytes.get(2..6).context("truncated BMP")?.try_into()?) as usize
        } else {
            0
        };
        ensure!(offset < bytes.len(), "missing appended save data in {name}");
        std::fs::write(saves.path().join(name), &bytes)?;
        // The output preserves the original BMP thumbnail, just as the game
        // appends Dictionary.saveStruct to saveLayerImage's BMP output.
        std::fs::write(
            saves.path().join(format!("roundtrip{i}.ksd")),
            &bytes[..offset],
        )?;
        cases.push_str(&format!("var d=Scripts.evalStorage(System.dataPath+'{name}','o{offset}');(Dictionary.saveStruct incontextof d)(System.dataPath+'roundtrip{i}.ksd','zo{offset}');var e=Scripts.evalStorage(System.dataPath+'roundtrip{i}.ksd','o{offset}');total+=equal(d,e,'{name}');\n"));
    }
    let script = format!(
        r#"
Plugins.link("saveStruct.dll");
function equal(a,b,path) {{
 if(typeof a!=typeof b) throw "type changed at "+path;
 if(typeof a!="Object" || a===null || b===null) {{if(a!==b)throw "value changed at "+path;return 1;}}
 if(a instanceof "Array") {{if(!(b instanceof "Array")||a.count!=b.count)throw "array changed at "+path;var n=1;for(var i=0;i<a.count;i++)n+=equal(a[i],b[i],path+"/"+i);return n;}}
 var aa=[],bb=[];aa.assign(a);bb.assign(b);if(aa.count!=bb.count)throw "dictionary changed at "+path;var n=1;for(var i=0;i<aa.count;i+=2)n+=equal(aa[i+1],b[aa[i]],path+"/"+aa[i]);return n;
}}
var total=0;
{cases}
return total;
"#
    );
    std::fs::write(project.path().join("audit.tjs"), script)?;
    let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000_000)?;
    let count = session.execute_storage("audit.tjs")?.integer()?;
    assert!(count > 1000, "expected nested real game state");
    for (i, name) in names.iter().enumerate() {
        let original = std::fs::read(source.join(name))?;
        if original.starts_with(b"BM") {
            let offset = u32::from_le_bytes(original[2..6].try_into()?) as usize;
            let roundtrip = std::fs::read(saves.path().join(format!("roundtrip{i}.ksd")))?;
            assert_eq!(
                &original[..offset],
                &roundtrip[..offset],
                "{name} thumbnail"
            );
        }
    }
    eprintln!(
        "checked {count} typed values across {} original saves",
        names.len()
    );
    Ok(())
}

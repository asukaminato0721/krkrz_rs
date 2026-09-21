use krkrz_runtime::{Session, file_dialog::FileSelection};
use krkrz_tjs::Value;
use std::path::Path;

fn quoted(path: &Path) -> String {
    serde_json::to_string(&format!("file://.{}", path.display())).unwrap()
}

fn session() -> (tempfile::TempDir, tempfile::TempDir, Session) {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let s = Session::open(project.path(), Some(saves.path()), None, 10_000_000).unwrap();
    (project, saves, s)
}

#[test]
fn system_directories_are_normalized_and_independent_of_save_override() {
    let (_project, saves, mut s) = session();
    for key in ["personalPath", "appDataPath"] {
        let path = s.evaluate(&format!("System.{key}")).unwrap().text();
        assert!(
            path.starts_with("file://./") && path.ends_with('/'),
            "{path}"
        );
        assert_ne!(
            krkrz_core::local_storage_path(&path).trim_end_matches('/'),
            saves.path().to_str().unwrap()
        );
        assert!(s.evaluate(&format!("System.{key}='changed'")).is_err());
        assert_eq!(s.evaluate(&format!("System.{key}")).unwrap().text(), path);
    }
}

#[test]
fn cancel_and_host_error_preserve_file_options() {
    let (_project, _saves, mut s) = session();
    s.evaluate("global.options=%[name:'original',filterIndex:2,filter:['BMP|*.bmp','All|*.*'],defaultExt:'.bmp',save:true]").unwrap();
    s.services.set_file_dialog_handler(|r| {
        assert!(r.save);
        assert_eq!(r.default_ext, ".bmp");
        assert_eq!(r.filter_index, 2);
        assert_eq!(r.filters, ["BMP|*.bmp", "All|*.*"]);
        Ok(None)
    });
    assert_eq!(
        s.evaluate("Storages.selectFile(options)").unwrap(),
        Value::Integer(0)
    );
    s.services
        .set_file_dialog_handler(|_| anyhow::bail!("backend failed"));
    assert!(
        format!(
            "{:#}",
            s.evaluate("Storages.selectFile(options)").unwrap_err()
        )
        .contains("backend failed")
    );
    assert_eq!(
        s.evaluate("options.name+':'+options.filterIndex").unwrap(),
        Value::string("original:2")
    );
}

#[test]
fn save_selected_bmp_appends_state_and_loads_it_without_redirecting_other_writes() {
    let (project, saves, mut s) = session();
    let external = tempfile::tempdir().unwrap();
    let target = external.path().join("画像.BMP");
    let selected = target.clone();
    s.services.set_file_dialog_handler(move |r| {
        assert!(r.save);
        Ok(Some(FileSelection {
            path: selected.clone(),
            filter_index: 1,
        }))
    });
    let script = r#"
        Plugins.link('saveStruct.dll');
        var options=%[save:true,filter:['BMP|*.bmp'],defaultExt:'.bmp'];
        if (!Storages.selectFile(options)) throw 'cancelled';
        var w=new Window(),p=new Layer(w,null),s=new Layer(w,p);
        s.setImageSize(2,2);s.fillRect(0,0,2,2,0xff123456);
        s.saveLayerImage(options.name,'bmp24');
        var state=%[name:'保存した状態',number:42];
        // A 2x2, 24-bit BMP is 54 header bytes plus two 8-byte scanlines.
        (Dictionary.saveStruct incontextof state)(options.name,'zo70');
        var loaded=Scripts.evalStorage(options.name,'o70');
        s.loadImages(options.name);
        (Dictionary.saveStruct incontextof state)(System.dataPath+'ordinary.ksd','z');
        return loaded.name+':'+loaded.number+':'+s.imageWidth+':'+s.imageHeight;
    "#;
    std::fs::write(project.path().join("export.tjs"), script).unwrap();
    assert_eq!(
        s.execute_storage("export.tjs").unwrap(),
        Value::string("保存した状態:42:2:2")
    );
    assert_eq!(
        s.evaluate("options.name").unwrap(),
        Value::string(&format!("file://.{}", target.display()))
    );
    let bytes = std::fs::read(&target).unwrap();
    assert_eq!(&bytes[..2], b"BM");
    assert_eq!(u32::from_le_bytes(bytes[2..6].try_into().unwrap()), 70);
    assert!(bytes.len() > 70);
    assert!(saves.path().join("ordinary.ksd").is_file());
    assert!(!saves.path().join("画像.bmp").exists());
    assert!(
        s.evaluate(&format!(
            "s.saveLayerImage({})",
            quoted(&external.path().join("other.bmp"))
        ))
        .is_err()
    );
}

#[test]
fn png_and_jpeg_exports_preserve_dimensions_color_and_png_alpha() {
    let (project, _saves, mut s) = session();
    let external = tempfile::tempdir().unwrap();
    let dir = external.path().to_owned();
    s.services.set_file_dialog_handler(move |r| {
        Ok(Some(FileSelection {
            path: dir.join(&r.name),
            filter_index: 1,
        }))
    });
    std::fs::write(project.path().join("setup.tjs"), "var w=new Window(),p=new Layer(w,null),s=new Layer(w,p);s.setImageSize(8,8);s.fillRect(0,0,8,8,0x80123456);").unwrap();
    s.execute_storage("setup.tjs").unwrap();
    for (mode, extension, alpha) in [
        ("png", "png", 128),
        ("png24", "png", 255),
        ("jpg", "jpg", 255),
        ("jpg100", "jpg", 255),
    ] {
        let name = format!("{mode}.{extension}");
        s.evaluate(&format!("global.options=%[save:true,name:'{name}']"))
            .unwrap();
        s.evaluate("Storages.selectFile(options)").unwrap();
        s.evaluate(&format!("s.saveLayerImage(options.name,'{mode}')"))
            .unwrap();
        let bytes = std::fs::read(external.path().join(name)).unwrap();
        let image = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(image.dimensions(), (8, 8));
        for pixel in image.pixels() {
            assert_eq!(pixel[3], alpha);
            for (actual, expected) in pixel.0[..3].iter().zip([0x12, 0x34, 0x56]) {
                assert!(actual.abs_diff(expected) <= if extension == "jpg" { 3 } else { 0 });
            }
        }
    }
}

#[test]
fn open_grants_read_access_only_to_the_chosen_file() {
    let (_project, _saves, mut s) = session();
    let external = tempfile::tempdir().unwrap();
    let target = external.path().join("Selected.TJS");
    let sibling = external.path().join("sibling.tjs");
    std::fs::write(&target, "42").unwrap();
    std::fs::write(&sibling, "43").unwrap();
    let selected = target.clone();
    s.services.set_file_dialog_handler(move |r| {
        assert!(!r.save);
        Ok(Some(FileSelection {
            path: selected.clone(),
            filter_index: 1,
        }))
    });
    s.evaluate("global.options=%[filter:'TJS|*.tjs']").unwrap();
    s.evaluate("Storages.selectFile(options)").unwrap();
    assert_eq!(
        s.evaluate("Scripts.evalStorage(options.name)").unwrap(),
        Value::Integer(42)
    );
    assert_eq!(
        s.evaluate("Storages.isExistentStorage(options.name)")
            .unwrap(),
        Value::Integer(1)
    );
    assert!(
        s.evaluate(&format!("Scripts.evalStorage({})", quoted(&sibling)))
            .is_err()
    );
    s.evaluate("Plugins.link('saveStruct.dll')").unwrap();
    assert!(
        s.evaluate("(Dictionary.saveStruct incontextof %[])(options.name,'z')")
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(target).unwrap(), "42");
}

#[cfg(unix)]
#[test]
fn a_symlink_replacing_the_selected_save_file_is_rejected() {
    let (_project, _saves, mut s) = session();
    let external = tempfile::tempdir().unwrap();
    let target = external.path().join("selected.ksd");
    let other = external.path().join("other.ksd");
    std::fs::write(&other, "original").unwrap();
    let selected = target.clone();
    s.services.set_file_dialog_handler(move |_| {
        Ok(Some(FileSelection {
            path: selected.clone(),
            filter_index: 1,
        }))
    });
    s.evaluate("global.options=%[save:true]").unwrap();
    s.evaluate("Storages.selectFile(options)").unwrap();
    s.evaluate("Plugins.link('saveStruct.dll')").unwrap();
    std::os::unix::fs::symlink(&other, &target).unwrap();
    assert!(
        s.evaluate("(Dictionary.saveStruct incontextof %[])(options.name,'z')")
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(other).unwrap(), "original");
}

use krkrz_runtime::Session;
use krkrz_tjs::Value;

#[test]
fn plugin_preserves_layers_and_exports_png_with_the_copied_alpha() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
    std::fs::write(project.path().join("case.tjs"), r#"
        var w=new Window(),existing=new Layer(w,null);
        existing.setImageSize(3,1);existing.fillRect(0,0,3,1,0xff123456);
        var oldLayer=Layer,oldWindow=Window;
        Plugins.link('layerExSave.dll');
        var method=Layer.saveLayerImagePng;
        Plugins.link('plugin/LAYEREXSAVE.DLL');
        if(oldLayer!==Layer || oldWindow!==Window || method!==Layer.saveLayerImagePng) throw 'replaced class or method';
        var dst=new Layer(w,existing),src=new Layer(w,existing),copy=new Layer(w,existing);
        dst.assignImages(existing);
        src.setImageSize(2,1);src.fillRect(0,0,2,1,0xffffffff);
        src.setMainPixel(0,0,0xabcdef);src.setMainPixel(1,0,0x123400);
        copy.assignImages(dst);
        dst.setClip(2,0,1,1);dst.face=dfProvince;
        dst.copyBlueToAlpha(src);
        dst.saveLayerImagePng(System.dataPath+'screenshot.png');
        src.saveLayerImagePng(System.dataPath+'opaque.png');
        src.copyBlueToAlpha(src);
        return [dst.getMainPixel(0,0),dst.getMaskPixel(0,0),dst.getMaskPixel(1,0),dst.getMaskPixel(2,0),copy.getMaskPixel(0,0),src.getMaskPixel(0,0)].join(',');
    "#).unwrap();
    assert_eq!(
        session.execute_storage("case.tjs").unwrap(),
        Value::string("1193046,239,0,255,255,239")
    );
    let image = image::open(saves.path().join("screenshot.png"))
        .unwrap()
        .to_rgba8();
    assert_eq!(image.dimensions(), (3, 1));
    assert_eq!(
        image.as_raw(),
        &[
            0x12, 0x34, 0x56, 239, 0x12, 0x34, 0x56, 0, 0x12, 0x34, 0x56, 255
        ]
    );
    let opaque = image::open(saves.path().join("opaque.png")).unwrap();
    assert!(!opaque.color().has_alpha());
    for expression in [
        "dst.copyBlueToAlpha(null)",
        "dst.saveLayerImagePng('../escape.png')",
        "dst.saveLayerImagePng(System.dataPath+'metadata.png',%[offs_x:1])",
        "dst.saveLayerImageTlg5(System.dataPath+'unsupported.tlg')",
        "(Window.startSaveLayerImage incontextof w)(dst,System.dataPath+'async.png')",
    ] {
        assert!(session.evaluate(expression).is_err(), "{expression}");
    }
    assert!(!saves.path().join("metadata.png").exists());
    assert!(!saves.path().join("unsupported.tlg").exists());
    assert!(!saves.path().join("async.png").exists());
}

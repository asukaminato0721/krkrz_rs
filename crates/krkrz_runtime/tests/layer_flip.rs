use krkrz_runtime::Session;
use krkrz_tjs::Value;

#[test]
fn flips_move_whole_main_and_province_images_without_changing_shared_sources() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
    std::fs::write(
        project.path().join("flip.tjs"),
        r#"
        var w=new Window(), a=new Layer(w,null), b=new Layer(w,a);
        a.setImageSize(3,2);
        for(var y=0;y<2;y++)for(var x=0;x<3;x++){
            var n=y*3+x+1;
            a.setMainPixel(x,y,n);a.setMaskPixel(x,y,n+10);a.setProvincePixel(x,y,n+20);
        }
        b.assignImages(a);
        b.setClip(1,0,1,1);b.face=dfProvince;b.holdAlpha=true;b.imageModified=false;
        var r=[];
        b.flipUD();
        for(var y=0;y<2;y++)for(var x=0;x<3;x++)r.add(b.getMainPixel(x,y));
        r.add(b.getMaskPixel(0,0));r.add(b.getProvincePixel(0,0));
        r.add(a.getMainPixel(0,0));r.add(a.getProvincePixel(0,0));r.add(b.imageModified);
        b.flipLR();
        for(var y=0;y<2;y++)for(var x=0;x<3;x++)r.add(b.getMainPixel(x,y));
        r.add(b.getMaskPixel(0,0));r.add(b.getProvincePixel(0,0));
        b.flipLR();b.flipUD();r.add(b.getMainPixel(0,0));
        return r.join(',');
    "#,
    )
    .unwrap();
    assert_eq!(
        session.execute_storage("flip.tjs").unwrap(),
        Value::string("4,5,6,1,2,3,14,24,1,21,1,6,5,4,3,2,1,16,26,1")
    );
}

#[test]
fn flip_requires_a_main_image() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 1000).unwrap();
    std::fs::write(project.path().join("flip.tjs"),
        "var w=new Window(),l=new Layer(w,null);l.hasImage=false;try{l.flipUD();}catch(e){return 7;}return 0;").unwrap();
    assert_eq!(
        session.execute_storage("flip.tjs").unwrap(),
        Value::Integer(7)
    );
}

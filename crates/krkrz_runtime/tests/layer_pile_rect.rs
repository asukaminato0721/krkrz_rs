use krkrz_runtime::Session;
use krkrz_tjs::Value;

fn execute(source: &str) -> Value {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("test.tjs"),
        format!(
            r#"
var w=new Window(),d=new Layer(w,null),s=new Layer(w,d);
d.setImageSize(4,2);s.setImageSize(4,2);
d.type=ltOpaque;d.holdAlpha=true;s.type=ltOpaque;
for(var y=0;y<2;y++)for(var x=0;x<4;x++){{
d.setMainPixel(x,y,0);d.setMaskPixel(x,y,255);
s.setMainPixel(x,y,0xff0000);s.setMaskPixel(x,y,128);
}}
{source}
"#
        ),
    )
    .unwrap();
    Session::open(project.path(), Some(saves.path()), None, 100_000)
        .unwrap()
        .execute_storage("test.tjs")
        .unwrap()
}

#[test]
fn source_type_is_ignored_and_optional_opacity_uses_the_eighth_argument() {
    assert_eq!(
        execute(
            r#"
var result=d.pileRect(0,0,s,0,0,2,1);
d.pileRect(0,1,s,0,0,2,1,128,false);
return [typeof result,d.getMainPixel(0,0),d.getMainPixel(0,1),d.getMaskPixel(0,1),d.getMainPixel(3,0)].join(',');
"#
        ),
        Value::string("void,8323072,4128768,255,0")
    );
}

#[test]
fn clipping_self_overlap_and_transparent_source_are_handled() {
    assert_eq!(
        execute(
            r#"
d.setClip(1,0,2,1);d.pileRect(0,0,s,-1,0,4,1,void);
d.setClip();d.setMainPixel(0,1,0x112233);d.setMainPixel(1,1,0x445566);
d.pileRect(1,1,d,0,1,2,1);
s.setMaskPixel(0,0,0);d.pileRect(1,1,s,0,0,1,1);
return [d.getMainPixel(0,0),d.getMainPixel(1,0),d.getMainPixel(3,0),d.getMainPixel(1,1),d.getMainPixel(2,1)].join(',');
"#
        ),
        Value::string("0,8323072,0,1122867,4478310")
    );
}

#[test]
fn alpha_destination_and_invalid_draw_faces() {
    assert_eq!(
        execute(
            r#"
d.face=dfAlpha;d.setMaskPixel(0,0,0);d.pileRect(0,0,s,0,0,1,1);
var alpha=d.getMaskPixel(0,0),caught=0;
try{d.pileRect();}catch(e){caught++;}
try{d.pileRect(0,0,null,0,0,1,1);}catch(e){caught++;}
for(var face=2;face<=4;face++){d.face=face;try{d.pileRect(0,0,s,0,0,1,1);}catch(e){caught++;}}
return [alpha,caught].join(',');
"#
        ),
        Value::string("128,5")
    );
}

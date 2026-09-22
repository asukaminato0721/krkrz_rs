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
d.setImageSize(8,4);s.setImageSize(2,1);
d.type=ltOpaque;d.holdAlpha=true;s.type=ltAlpha;
for(var y=0;y<4;y++)for(var x=0;x<8;x++){d.setMainPixel(x,y,0);d.setMaskPixel(x,y,255);}
s.setMainPixel(0,0,0xff0000);s.setMaskPixel(0,0,128);
s.setMainPixel(1,0,0x00ff00);s.setMaskPixel(1,0,255);
{source}
"#
        ),
    )
    .unwrap();
    Session::open(project.path(), Some(saves.path()), None, 1_000_000)
        .unwrap()
        .execute_storage("test.tjs")
        .unwrap()
}

#[test]
fn scales_with_auto_alpha_opacity_and_preserves_destination_alpha() {
    assert_eq!(
        execute(
            r#"
d.operateStretch(0,0,8,4,s,0,0,2,1);
return [d.getMainPixel(0,0),d.getMainPixel(3,3),d.getMainPixel(4,0),d.getMaskPixel(0,0)].join(',');
"#
        ),
        Value::string("8323072,8323072,65024,255")
    );
    assert_eq!(
        execute(
            r#"
d.operateStretch(0,0,8,4,s,0,0,2,1,omAlpha,128,stNearest);
return [d.getMainPixel(0,0),d.getMainPixel(7,3),d.getMaskPixel(0,0)].join(',');
"#
        ),
        Value::string("4128768,32256,255")
    );
}

#[test]
fn clips_and_flips_without_changing_pixels_outside_clip() {
    assert_eq!(
        execute(
            r#"
d.setClip(1,1,6,2);
d.operateStretch(8,0,-8,4,s,0,0,2,1,omOpaque);
return [d.getMainPixel(0,1),d.getMainPixel(1,1),d.getMainPixel(6,2),d.getMainPixel(7,2),d.getMainPixel(1,0)].join(',');
"#
        ),
        Value::string("0,65280,16711680,0,0")
    );
}

#[test]
fn unscaled_overlap_uses_a_source_snapshot() {
    assert_eq!(
        execute(
            r#"
d.setMainPixel(0,0,0x112233);d.setMainPixel(1,0,0x445566);d.setMainPixel(2,0,0x778899);
d.operateStretch(1,0,3,1,d,0,0,3,1,omOpaque);
return [d.getMainPixel(1,0),d.getMainPixel(2,0),d.getMainPixel(3,0)].join(',');
"#
        ),
        Value::string("1122867,4478310,7833753")
    );
}

#[test]
fn filtered_scaling_blends_and_zero_opacity_is_noop() {
    assert_eq!(
        execute(
            r#"
d.holdAlpha=false;s.fillRect(0,0,2,1,0xffff0000);
d.operateStretch(0,0,8,4,s,0,0,2,1,omOpaque,128,stLinear);
var color=d.getMainPixel(4,2);
d.operateStretch(0,0,8,4,s,0,0,2,1,omOpaque,0);
return [color,d.getMainPixel(4,2)].join(',');
"#
        ),
        Value::string("8323072,8323072")
    );
}

#[test]
fn invalid_calls_and_unsupported_blends_fail_explicitly() {
    assert_eq!(
        execute(
            r#"
var caught=0;
try{d.operateStretch();}catch(e){caught++;}
try{d.operateStretch(0,0,8,4,null,0,0,2,1);}catch(e){caught++;}
try{d.operateStretch(0,0,8,4,s,-1,0,2,1);}catch(e){caught++;}
return caught;
"#
        ),
        Value::Integer(3)
    );
}

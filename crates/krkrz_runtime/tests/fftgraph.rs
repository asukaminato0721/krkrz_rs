use krkrz_runtime::Session;
use krkrz_tjs::Value;

fn session(source: &str) -> (Session, tempfile::TempDir, tempfile::TempDir) {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("tone.wav"),
        include_bytes!("fixtures/tone.wav"),
    )
    .unwrap();
    std::fs::write(
        project.path().join("startup.tjs"),
        format!(
            r#"
Plugins.link('fftgraph.dll');
var w=new Window(),l=new Layer(w,null),s=new WaveSoundBuffer(null);
l.setImageSize(64,32);s.open('tone.wav');
{source}
"#
        ),
    )
    .unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 5_000_000).unwrap();
    session.startup().unwrap();
    (session, project, saves)
}

#[test]
fn plugin_link_preserves_function_and_silent_graph_pixels() {
    let (mut s, _project, _saves) = session(
        r#"
var original=drawFFTGraph;
Plugins.link('plugin/FFTGRAPH.DLL');
var updates=[];
l.update=function(x,y,w,h){updates.add([x,y,w,h].join(','));};
l.fillRect(0,0,64,32,0xff123456);
var result=drawFFTGraph(l,s,2,3,8,8);
"#,
    );
    assert_eq!(
        s.evaluate("original===drawFFTGraph && typeof result=='void'")
            .unwrap(),
        Value::Integer(1)
    );
    assert_eq!(
        s.evaluate("updates.join('|')").unwrap(),
        Value::string("2,3,8,8")
    );
    assert_eq!(s.evaluate("[l.getMainPixel(2,10),l.getMaskPixel(2,10),l.getMainPixel(2,9),l.getMaskPixel(2,9),l.getMainPixel(1,10)].join(',')").unwrap(), Value::string("8421504,255,0,0,1193046"));
    s.evaluate("drawFFTGraph(l,s,2,3,8,8,%[type:1,division:2,thick:2])")
        .unwrap();
    assert_eq!(
        s.evaluate("[l.getMainPixel(2,10),l.getMainPixel(3,9),l.getMainPixel(3,7)].join(',')")
            .unwrap(),
        Value::string("12632256,7368816,11579568")
    );
}

#[test]
fn audio_preview_draws_signal_without_advancing_playback_and_peak_decays() {
    let (mut s, _project, _saves) = session(
        r#"
s.useVisBuffer=true;s.play();
drawFFTGraph(l,s,0,0,64,32);
function highest(){var n=0;for(var y=0;y<31;y++)for(var x=0;x<64;x++)if(l.getMaskPixel(x,y))n++;return n;}
var signal=highest();
"#,
    );
    assert_eq!(
        s.evaluate("signal>0 && s.samplePosition==0").unwrap(),
        Value::Integer(1)
    );
    s.evaluate("s.stop()").unwrap();
    s.evaluate("drawFFTGraph(l,s,0,0,64,32)").unwrap();
    assert_eq!(s.evaluate("highest()>0").unwrap(), Value::Integer(1));
    s.evaluate("Scripts.exec('for(var i=0;i<70;i++)drawFFTGraph(l,s,0,0,64,32);')")
        .unwrap();
    assert_eq!(s.evaluate("highest()").unwrap(), Value::Integer(0));
}

#[test]
fn unsafe_rectangles_options_and_reentrant_invalidation_are_rejected() {
    let (mut s, _project, _saves) = session("");
    for call in [
        "drawFFTGraph()",
        "drawFFTGraph(l,s,-1,0,8,8)",
        "drawFFTGraph(l,s,63,0,8,8)",
        "drawFFTGraph(l,s,0,0,8,8,%[type:1,division:0])",
        "drawFFTGraph(l,s,0,0,8,8,%[type:1,division:2,thick:0])",
        "drawFFTGraph(l,%[getVisBuffer:function(){invalidate global.l;return 0;}],0,0,8,8)",
    ] {
        assert!(s.evaluate(call).is_err(), "{call}");
    }
}

#[test]
fn callback_budget_exhaustion_is_not_swallowed() {
    let (mut s, _project, _saves) = session("");
    s.budget = 30_000;
    let error = s
        .evaluate("drawFFTGraph(l,%[getVisBuffer:function(){while(true){}}],0,0,8,8)")
        .unwrap_err();
    assert!(
        error.downcast_ref::<krkrz_tjs::VmAbort>().is_some(),
        "{error:#}"
    );
}

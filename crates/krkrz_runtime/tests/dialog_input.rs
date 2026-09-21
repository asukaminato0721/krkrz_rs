use krkrz_runtime::Session;
use krkrz_tjs::{Value, VmAbort};

// Both installed-script probes acquire the same game's System.createAppLock.
static INSTALLED_GAME: std::sync::Mutex<()> = std::sync::Mutex::new(());

const SETUP: &str = r#"
Plugins.link('win32dialog.dll');
var events=[];
class TextDialog extends WIN32Dialog {
 function TextDialog(){
  super.WIN32Dialog(null);
  var h=new global.WIN32Dialog.Header();h.store(%[title:'Rename']);h.dlgItems=4;
  var label=new global.WIN32Dialog.Items(),edit=new global.WIN32Dialog.Items(),ok=new global.WIN32Dialog.Items(),cancel=new global.WIN32Dialog.Items();
  label.store(%[windowClass:STATIC,style:WS_VISIBLE,title:'New name',id:9]);
  edit.store(%[windowClass:EDIT,style:WS_VISIBLE|ES_AUTOHSCROLL,title:'template',id:10]);
  ok.store(%[windowClass:BUTTON,style:WS_VISIBLE|BS_DEFPUSHBUTTON,title:'&OK',id:IDOK]);
  cancel.store(%[windowClass:BUTTON,style:WS_VISIBLE,title:'Cancel',id:IDCANCEL]);
  makeTemplate(h,label,edit,ok,cancel);
  invalidate h;invalidate label;invalidate edit;invalidate ok;invalidate cancel;
 }
 var answer, button;
 function onInit(msg,wp,lp){events.add('init:'+msg);setItemText(10,'initial');return true;}
 function onCommand(msg,wp,lp){
  events.add('command:'+msg+':'+wp);
  answer=getItemText(10);button=sendItemMessage(IDOK,BM_GETSTATE,0,0);
  close(wp);return true;
 }
}
var d=new TextDialog();
"#;

fn session() -> (tempfile::TempDir, tempfile::TempDir, Session) {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("setup.tjs"), SETUP).unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 1_000_000).unwrap();
    session.execute_storage("setup.tjs").unwrap();
    (project, saves, session)
}

#[test]
fn modal_input_initializes_controls_and_distinguishes_confirm_empty_and_cancel() {
    let (_project, _saves, mut s) = session();
    let mut answers = [Some(" 名前 ".to_string()), Some(String::new()), None].into_iter();
    s.services.set_input_dialog_handler(move |r| {
        assert_eq!(
            (&*r.title, &*r.fields[0].label, &*r.fields[0].text),
            ("Rename", "New name", "initial")
        );
        assert_eq!((&*r.accept_label, &*r.cancel_label), ("&OK", "Cancel"));
        Ok(answers.next().unwrap().map(|text| vec![text]))
    });
    for (code, text) in [(1, " 名前 "), (1, ""), (2, "initial")] {
        assert_eq!(s.evaluate("d.open(null)").unwrap(), Value::Integer(code));
        assert_eq!(s.evaluate("d.answer").unwrap(), Value::string(text));
        assert_eq!(
            s.evaluate("d.isValid || d.button").unwrap(),
            Value::Integer(0)
        );
    }
    assert_eq!(
        s.evaluate("events.join('|')").unwrap(),
        Value::string("init:272|command:273:1|init:272|command:273:1|init:272|command:273:2")
    );
}

#[test]
fn modal_errors_and_invalidation_leave_no_open_dialog() {
    let (_project, _saves, mut s) = session();
    s.services
        .set_input_dialog_handler(|_| anyhow::bail!("backend failed"));
    assert!(format!("{:#}", s.evaluate("d.open(null)").unwrap_err()).contains("backend failed"));
    assert_eq!(s.evaluate("d.isValid").unwrap(), Value::Integer(0));
    s.services
        .set_input_dialog_handler(|_| panic!("closed in onInit"));
    s.evaluate("d.onInit=function(){close(7);}").unwrap();
    assert_eq!(s.evaluate("d.open(null)").unwrap(), Value::Integer(7));
    s.evaluate("d.onInit=function(){invalidate this;}").unwrap();
    assert!(
        format!("{:#}", s.evaluate("d.open(null)").unwrap_err())
            .contains("invalidated during callback")
    );
}

#[test]
fn modal_missing_host_and_modeless_remain_explicitly_unsupported() {
    let (_project, _saves, mut s) = session();
    let error = s.evaluate("d.open(null)").unwrap_err();
    assert!(error.downcast_ref::<VmAbort>().is_some());
    s.services
        .set_input_dialog_handler(|_| panic!("modeless cannot use modal backend"));
    s.evaluate("d.modeless=true").unwrap();
    assert!(
        format!("{:#}", s.evaluate("d.open(null)").unwrap_err()).contains("modeless presentation")
    );
}

#[test]
fn modal_combo_and_multiline_preserve_options_and_windows_newlines() {
    let (_project, _saves, mut s) = session();
    std::fs::write(_project.path().join("editor.tjs"), r#"
      var h=new WIN32Dialog.Header();h.store(%[title:'Generic editor',dlgItems:4]);
      var combo=new WIN32Dialog.Items(),edit=new WIN32Dialog.Items(),ok=new WIN32Dialog.Items(),cancel=new WIN32Dialog.Items();
      combo.store(%[windowClass:'COMBOBOX',style:WIN32Dialog.WS_VISIBLE|WIN32Dialog.CBS_DROPDOWN,id:11]);
      edit.store(%[windowClass:'EDIT',style:WIN32Dialog.WS_VISIBLE|WIN32Dialog.ES_MULTILINE|WIN32Dialog.ES_WANTRETURN,id:10]);
      ok.store(%[windowClass:'BUTTON',style:WIN32Dialog.WS_VISIBLE,title:'OK',id:1]);
      cancel.store(%[windowClass:'BUTTON',style:WIN32Dialog.WS_VISIBLE,title:'Cancel',id:2]);
      d.makeTemplate(h,combo,edit,ok,cancel);
      d.onInit=function(){
        sendItemMessage(11,CB_ADDSTRING,0,'Alice');sendItemMessage(11,CB_ADDSTRING,0,'Bob');
        sendItemMessage(11,CB_LIMITTEXT,18,0);setItemText(11,'Alice');
        setItemText(10,'first\r\nsecond');sendItemMessage(10,EM_SETSEL,0,-1);setItemFocus(10);
        setPos(50,70);
      };
    "#).unwrap();
    s.execute_storage("editor.tjs").unwrap();
    s.services.set_input_dialog_handler(|r| {
        assert_eq!(r.fields.len(), 2);
        assert_eq!(r.position, Some([50, 70]));
        assert_eq!(r.fields[0].choices.as_ref().unwrap(), &["Alice", "Bob"]);
        assert_eq!(r.fields[0].max_length, 18);
        assert_eq!(r.fields[0].text, "Alice");
        assert!(!r.fields[0].multiline && !r.fields[0].focused);
        assert!(r.fields[1].multiline && r.fields[1].focused);
        assert_eq!(r.fields[1].selection, Some([0, -1]));
        assert_eq!(r.fields[1].text, "first\r\nsecond");
        Ok(Some(vec!["Bob".into(), "changed\ntext\r\nlast".into()]))
    });
    assert_eq!(s.evaluate("d.open(null)").unwrap(), Value::Integer(1));
    assert_eq!(
        s.evaluate("d.answer").unwrap(),
        Value::string("changed\r\ntext\r\nlast")
    );
    s.services.set_input_dialog_handler(|_| Ok(Some(vec![])));
    assert!(
        format!("{:#}", s.evaluate("d.open(null)").unwrap_err()).contains("wrong number of fields")
    );
    assert_eq!(s.evaluate("d.isValid").unwrap(), Value::Integer(0));
}

#[test]
#[ignore = "requires KRKRZ_TEST_PROJECT with the installed inputString compatibility scripts"]
fn installed_input_string_confirms_and_cancels() {
    let _lock = INSTALLED_GAME.lock().unwrap();
    let project = std::env::var_os("KRKRZ_TEST_PROJECT").expect("KRKRZ_TEST_PROJECT");
    let mut storage = krkrz_assets::storage::Storage::open(
        std::path::Path::new(&project),
        None,
        Default::default(),
    )
    .unwrap();
    storage.detect_cipher().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut s = Session::from_storage(storage, Some(saves.path()), None, 100_000_000).unwrap();
    s.startup().unwrap();
    let mut answers = [Some("新しい名前".to_string()), None, Some(String::new())].into_iter();
    s.services.set_input_dialog_handler(move |r| {
        assert_eq!(
            (&*r.title, &*r.fields[0].label, &*r.fields[0].text),
            ("名前", "入力してください", "初期値")
        );
        Ok(answers.next().unwrap().map(|text| vec![text]))
    });
    for expected in [Value::string("新しい名前"), Value::Void, Value::string("")] {
        assert_eq!(
            s.evaluate("System.inputString('名前','入力してください','初期値')")
                .unwrap(),
            expected
        );
    }
}

#[test]
#[ignore = "requires KRKRZ_TEST_PROJECT with EditStandMessageDialog"]
fn installed_editable_combo_and_multiline_dialog() {
    let _lock = INSTALLED_GAME.lock().unwrap();
    let project = std::env::var_os("KRKRZ_TEST_PROJECT").expect("KRKRZ_TEST_PROJECT");
    let mut storage = krkrz_assets::storage::Storage::open(
        std::path::Path::new(&project),
        None,
        Default::default(),
    )
    .unwrap();
    storage.detect_cipher().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut s = Session::from_storage(storage, Some(saves.path()), None, 100_000_000).unwrap();
    s.startup().unwrap();
    s.evaluate("KAGLoadScript('win32dialog.tjs')").unwrap();
    s.evaluate("KAGLoadScript('editstandtext.tjs')").unwrap();
    s.evaluate("global.testDialog = new EditStandMessageDialog('Edit','Alice','line1\\nline2',['Alice','Bob'])").unwrap();
    s.services.set_input_dialog_handler(|r| {
        assert_eq!(r.fields.len(), 2);
        assert_eq!(r.fields[0].text, "Alice");
        assert_eq!(
            r.fields[0].choices.as_deref(),
            Some(["Alice".to_string(), "Bob".to_string()].as_slice())
        );
        assert_eq!(r.fields[0].max_length, 18);
        assert!(r.fields[1].multiline && r.fields[1].focused);
        assert_eq!(r.fields[1].selection, Some([0, -1]));
        assert_eq!(r.fields[1].text, "line1\r\nline2");
        Ok(Some(vec!["Bob".into(), "new line1\nnew line2".into()]))
    });
    s.evaluate("global.testResult = testDialog.open(null)")
        .unwrap();
    assert_eq!(s.evaluate("testResult.name").unwrap(), Value::string("Bob"));
    assert_eq!(
        s.evaluate("testResult.text").unwrap(),
        Value::string("new line1\nnew line2")
    );
    s.services.set_input_dialog_handler(|_| Ok(None));
    assert_eq!(s.evaluate("testDialog.open(null)").unwrap(), Value::Void);
}

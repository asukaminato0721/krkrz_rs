use krkrz_runtime::Session;
use krkrz_tjs::Value;

fn session() -> (tempfile::TempDir, tempfile::TempDir, Session) {
    let dir = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut session = Session::open(dir.path(), Some(saves.path()), None, 100_000).unwrap();
    session.evaluate("Plugins.link('ktsndopt.dll')").unwrap();
    (dir, saves, session)
}
const INIT: &str = "ktSndOptDlg_Init(1,80,1,1,20,1,70,1,0,30,1,90,0,0,40)";
#[test]
fn dialog_preserves_hidden_values_and_returns_changed_octets() {
    let (_dir, _saves, mut s) = session();
    s.services.set_input_dialog_handler(|request| {
        assert_eq!(request.title, "Sound settings");
        assert_eq!(
            request.fields.iter().map(|f| f.id).collect::<Vec<_>>(),
            [0, 1, 2, 3, 4, 5, 6]
        );
        assert_eq!(request.fields[0].range, Some([0, 100]));
        assert!(request.fields[1].checkbox);
        assert_eq!(
            request
                .fields
                .iter()
                .map(|f| f.text.as_str())
                .collect::<Vec<_>>(),
            ["80", "1", "20", "70", "0", "30", "90"]
        );
        Ok(Some(
            ["60", "0", "25", "50", "1", "35", "100"]
                .map(String::from)
                .to_vec(),
        ))
    });
    assert_eq!(s.evaluate(INIT).unwrap(), Value::Integer(1));
    assert_eq!(
        s.evaluate("ktSndOptDlg_GetState()").unwrap(),
        Value::Octet(vec![60, 0, 25, 50, 1, 35, 100, 0, 40])
    );
    assert_eq!(s.evaluate("ktSndOptDlg_Final()").unwrap(), Value::Void);
    assert!(s.evaluate("ktSndOptDlg_GetState()").is_err());
}
#[test]
fn cancel_and_invalid_answers_do_not_publish_state() {
    let (_dir, _saves, mut s) = session();
    s.services.set_input_dialog_handler(|_| Ok(None));
    assert_eq!(s.evaluate(INIT).unwrap(), Value::Integer(0));
    assert!(s.evaluate("ktSndOptDlg_GetState()").is_err());
    for answer in [vec![], vec!["999".into(); 7], vec!["text".into(); 7]] {
        s.services
            .set_input_dialog_handler(move |_| Ok(Some(answer.clone())));
        assert!(s.evaluate(INIT).is_err());
        assert!(s.evaluate("ktSndOptDlg_GetState()").is_err());
    }
}

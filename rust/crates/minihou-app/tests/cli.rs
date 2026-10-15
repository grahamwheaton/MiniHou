use std::process::Command;
#[test]
fn selftest_cli_prints_one_json_line_without_modifying_graph() {
    let path=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.json");
    let before=std::fs::read(&path).unwrap();
    let output=Command::new(env!("CARGO_BIN_EXE_minihou-app")).arg("--selftest").arg(&path).output().unwrap();
    assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
    let text=String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.lines().count(),1);
    let value:serde_json::Value=serde_json::from_str(&text).unwrap();
    assert_eq!(value["nodes"],3);
    assert_eq!(value["cooked_second"],serde_json::json!(["n","o"]));
    assert_eq!(std::fs::read(path).unwrap(),before);
}
#[test]
fn missing_graph_exits_nonzero_without_stdout() {
    let path=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/does-not-exist.json");
    let output=Command::new(env!("CARGO_BIN_EXE_minihou-app")).arg("--selftest").arg(path).output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}

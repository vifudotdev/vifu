#[cfg(feature = "runtime")]
#[test]
fn public_sdk_exposes_the_embeddable_runtime() {
    use vifu::runtime::prelude::*;

    let mut runtime = HeadlessRuntime::new();
    let advance = runtime.dispatch(RuntimeCommand::new(
        "command-1",
        "application.input",
        json!({ "text": "Hello" }),
    ));

    assert_eq!(advance.snapshot.revision, 1);
}

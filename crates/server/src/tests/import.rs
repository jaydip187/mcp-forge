use super::*;
use forge_importers::openapi3::import;
use forge_importers::parse_document;

#[test]
fn assign_names_avoids_existing_and_keeps_spec_names() {
    let doc = parse_document(
            r#"{"openapi":"3.0.0","info":{"title":"t","version":"1"},"servers":[{"url":"https://x.io"}],
                "paths":{"/a":{"get":{"operationId":"a"}},"/a2":{"get":{"operationId":"a_2"}},"/b":{"get":{"operationId":"b"}}}}"#,
        )
        .unwrap();
    let imported = import(&doc, &openapi3::Options::default()).unwrap();
    let names = assign_names(&imported.operations, &["a".into()]);
    // `a` clashes with an existing tool; `a_2` belongs to the spec, so it becomes `a_3`.
    assert_eq!(names["GET /a"], "a_3");
    assert_eq!(names["GET /a2"], "a_2");
    assert_eq!(names["GET /b"], "b");
}

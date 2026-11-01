use serde_json::json;

use super::*;

#[test]
fn resources_validate() {
    let ok = ResourceInput {
        name: "style_guide".into(),
        description: String::new(),
        mime_type: "text/markdown".into(),
        content: "# Style".into(),
    };
    assert!(ok.validate().is_ok());
    let bad = ResourceInput {
        name: "bad name".into(),
        mime_type: "image/png".into(),
        ..ok.clone()
    };
    let e = bad.validate().unwrap_err();
    assert!(e.has("name") && e.has("mime_type"), "{e:?}");
    let json_bad = ResourceInput {
        mime_type: "application/json".into(),
        content: "{ nope".into(),
        ..ok
    };
    assert!(json_bad.validate().unwrap_err().has("content"));
}

#[test]
fn prompts_validate_and_render() {
    let input = PromptInput {
        name: "triage".into(),
        description: "Triage a ticket".into(),
        arguments: vec![
            PromptArg {
                name: "ticket".into(),
                description: String::new(),
                required: true,
            },
            PromptArg {
                name: "tone".into(),
                description: String::new(),
                required: false,
            },
        ],
        template: "Triage ticket {{ticket}}. Tone: {{tone}}.".into(),
    };
    assert!(input.validate().is_ok());
    let mut bad = input.clone();
    bad.arguments.push(PromptArg {
        name: "unused".into(),
        description: String::new(),
        required: false,
    });
    bad.template.push_str(" {{missing}}");
    let e = bad.validate().unwrap_err();
    assert!(e.has("arguments[2].name") && e.has("arguments"), "{e:?}");

    let def = PromptDef {
        id: Uuid::nil(),
        project_id: Uuid::nil(),
        name: input.name,
        description: input.description,
        arguments: input.arguments,
        template: input.template,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let args = |v: Value| v.as_object().unwrap().clone();
    assert_eq!(
        def.render(&args(json!({ "ticket": "#42", "tone": "kind" })))
            .unwrap(),
        "Triage ticket #42. Tone: kind."
    );
    assert_eq!(
        def.render(&args(json!({ "ticket": 7 }))).unwrap(),
        "Triage ticket 7. Tone: ."
    );
    assert!(def.render(&args(json!({}))).unwrap_err().contains("ticket"));
}

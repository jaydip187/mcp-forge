use super::*;

#[test]
fn slugify_examples() {
    assert_eq!(slugify("My GitHub API"), "my-github-api");
    assert_eq!(slugify("  --Hello__World!!  "), "hello-world");
    assert_eq!(slugify("日本"), "");
    assert!(slugify(&"ab ".repeat(40)).len() <= MAX_SLUG_LEN);
    assert!(is_valid_slug(&slugify(&"ab ".repeat(40))));
}

#[test]
fn slug_rules() {
    assert!(is_valid_slug("a"));
    assert!(is_valid_slug("my-api-2"));
    for bad in ["", "-a", "a-", "a--b", "A", "a_b", "a b"] {
        assert!(!is_valid_slug(bad), "{bad:?}");
    }
}

#[test]
fn input_validation() {
    let ok = ProjectInput {
        name: " Demo API ".into(),
        slug: None,
    };
    assert_eq!(
        ok.validate().unwrap(),
        ("Demo API".into(), "demo-api".into())
    );

    let underivable = ProjectInput {
        name: "日本".into(),
        slug: None,
    };
    assert!(underivable.validate().unwrap_err().has("slug"));

    let bad = ProjectInput {
        name: "".into(),
        slug: Some("Bad Slug".into()),
    };
    let e = bad.validate().unwrap_err();
    assert!(e.has("name") && e.has("slug"));
}

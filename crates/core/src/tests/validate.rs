use super::*;

#[test]
fn name_rules() {
    assert!(is_valid_name("get_user"));
    assert!(is_valid_name("Get-User_2"));
    assert!(is_valid_name(&"a".repeat(64)));
    assert!(!is_valid_name(""));
    assert!(!is_valid_name(&"a".repeat(65)));
    assert!(!is_valid_name("get user"));
    assert!(!is_valid_name("get.user"));
    assert!(!is_valid_name("café"));
}

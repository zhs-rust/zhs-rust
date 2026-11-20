//! `Object` 查找语义:精确匹配优先,大小写不敏感回退不改变精确匹配结果。

use zhs_json::{Object, Value};

fn object(entries: &[(&str, i64)]) -> Object {
    let mut object = Object::new();
    for (key, value) in entries {
        object.insert(*key, Value::from(*value));
    }
    object
}

#[test]
fn exact_get_matches_verbatim_key() {
    let object = object(&[("Host", 1), ("host", 2)]);

    assert_eq!(object.get("Host"), Some(&Value::from(1)));
    assert_eq!(object.get("host"), Some(&Value::from(2)));
    assert_eq!(object.get("HOST"), None);
}

#[test]
fn ignore_case_lookup_finds_folded_key() {
    let object = object(&[("Host", 1), ("Port", 3306)]);

    assert_eq!(object.get_ignore_ascii_case("HOST"), Some(&Value::from(1)));
    assert_eq!(object.get_ignore_ascii_case("host"), Some(&Value::from(1)));
    assert_eq!(object.get_ignore_ascii_case("port"), Some(&Value::from(3306)));
}

#[test]
fn ignore_case_lookup_prefers_exact_match() {
    // 精确命中决定胜负,回退只在精确未命中时生效
    let object = object(&[("Host", 1), ("host", 2)]);

    assert_eq!(object.get("host"), Some(&Value::from(2)));
    assert_eq!(object.get("Host"), Some(&Value::from(1)));
}

#[test]
fn ignore_case_lookup_returns_first_folded_match() {
    // 同一折叠键出现多次时按插入顺序取第一个
    let object = object(&[("Host", 1), ("HOST", 3)]);

    assert_eq!(object.get_ignore_ascii_case("host"), Some(&Value::from(1)));
}

#[test]
fn ignore_case_lookup_missing_key_is_none() {
    let object = object(&[("Host", 1)]);

    assert_eq!(object.get_ignore_ascii_case("user"), None);
}

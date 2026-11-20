//! `FromJson` 派生宏的大小写不敏感回退:
//! 精确匹配优先,未命中时按 ASCII 大小写不敏感回退(对齐
//! ASP.NET Core web 默认的 `PropertyNameCaseInsensitive = true`)。

use zhs_framework::json;
use zhs_framework::prelude::FromJson;

#[derive(FromJson, Debug)]
struct Settings {
    host: String,
    port: u16,
    issuer: Option<String>,
}

#[test]
fn pascal_case_keys_bind_without_rename() {
    let value = json::parse(r#"{"Host":"127.0.0.1","Port":3306}"#).unwrap();
    let settings = Settings::from_json(&value).unwrap();

    assert_eq!(settings.host, "127.0.0.1");
    assert_eq!(settings.port, 3306);
    assert_eq!(settings.issuer, None);
}

#[test]
fn exact_key_still_wins_over_folded_duplicate() {
    // 同时存在 "Host"(值 1)与 "host"(值 2):字段 host 按精确匹配取 2
    let value = json::parse(r#"{"Host":"a","host":"b","Port":1}"#).unwrap();
    let settings = Settings::from_json(&value).unwrap();

    assert_eq!(settings.host, "b");
}

#[test]
fn optional_field_binds_from_folded_key() {
    let value = json::parse(r#"{"host":"h","port":1,"ISSUER":"zhs"}"#).unwrap();
    let settings = Settings::from_json(&value).unwrap();

    assert_eq!(settings.issuer.as_deref(), Some("zhs"));
}

#[test]
fn missing_required_field_still_fails() {
    let value = json::parse(r#"{"host":"h"}"#).unwrap();
    let error = Settings::from_json(&value).unwrap_err();

    assert!(error.to_string().contains("port"), "{error}");
}

#[derive(FromJson, Debug)]
struct Renamed {
    #[json(rename = "DisplayName")]
    display_name: String,
}

#[test]
fn rename_attribute_keeps_exact_priority() {
    let value = json::parse(r#"{"DisplayName":"ada"}"#).unwrap();
    let renamed = Renamed::from_json(&value).unwrap();
    assert_eq!(renamed.display_name, "ada");

    // rename 之外的字段同样享受回退
    let value = json::parse(r#"{"displayname":"ada"}"#).unwrap();
    let renamed = Renamed::from_json(&value).unwrap();
    assert_eq!(renamed.display_name, "ada");
}

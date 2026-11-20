use zhs_framework::prelude::*;

#[test]
fn prelude_exposes_core_types() {
    let id = Guid::new_v4();
    assert_eq!(id.version(), 4);

    let timestamp: DateTimeOffset = "2026-11-19T21:30:00+08:00".parse().unwrap();
    assert_eq!(timestamp.to_string(), "2026-11-19T21:30:00+08:00");

    let amount: Decimal = "123.45".parse().unwrap();
    assert_eq!(amount.to_string(), "123.45");
}

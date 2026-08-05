use super::*;

#[test]
fn into_value_and_option_null() {
    let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 13).unwrap();
    let timestamp = date.and_hms_opt(9, 30, 0).unwrap();
    let decimal = Decimal::try_from("1.5").unwrap();

    assert_eq!(7i32.into_value(), Value::Int32(7));
    assert_eq!(42i64.into_value(), Value::Int64(42));
    assert_eq!(1.5f64.into_value(), Value::Double(1.5));
    assert_eq!(true.into_value(), Value::Bool(true));
    assert_eq!(
        String::from("exa").into_value(),
        Value::String("exa".to_string())
    );
    assert_eq!("exa".into_value(), Value::String("exa".to_string()));
    assert_eq!(decimal.clone().into_value(), Value::Numeric(decimal));
    assert_eq!(date.into_value(), Value::Date(date));
    assert_eq!(timestamp.into_value(), Value::Timestamp(timestamp));
    assert_eq!(Value::Int64(9).into_value(), Value::Int64(9));

    // Option<T>: Some forwards to the inner conversion, None maps to NULL.
    assert_eq!(Some(42i64).into_value(), Value::Int64(42));
    assert_eq!(None::<i64>.into_value(), Value::Null);
}

#[test]
fn decimal_from_str_roundtrip() {
    let high_precision = Decimal::try_from("-1.000000000000000001").unwrap();
    assert_eq!(high_precision.unscaled, -1_000_000_000_000_000_001);
    assert_eq!(high_precision.scale, 18);
    assert_eq!(high_precision.to_string(), "-1.000000000000000001");

    let zero = Decimal::try_from("0").unwrap();
    assert_eq!(zero.scale, 0);
    assert_eq!(zero.to_string(), "0");

    let one_and_half = Decimal::try_from("1.5").unwrap();
    assert_eq!(one_and_half.to_string(), "1.5");

    assert!(matches!(Decimal::try_from("abc"), Err(UdfError::Type(_))));
}

#[test]
fn value_exatype_typed_variants() {
    let date = chrono::NaiveDate::from_ymd_opt(2026, 6, 5).unwrap();
    let timestamp = date.and_hms_opt(12, 0, 0).unwrap();

    let values = [
        Value::Null,
        Value::Double(1.5),
        Value::Int32(7),
        Value::Int64(42),
        Value::Numeric(Decimal::try_from("1.000000000000000001").unwrap()),
        Value::Bool(true),
        Value::String("exa".to_string()),
        Value::Date(date),
        Value::Timestamp(timestamp),
    ];

    assert_eq!(values.len(), 9);

    for v in &values {
        match v {
            Value::Null => {}
            Value::Double(d) => assert_eq!(*d, 1.5),
            Value::Int32(i) => assert_eq!(*i, 7),
            Value::Int64(i) => assert_eq!(*i, 42),
            Value::Numeric(d) => assert_eq!(d.to_string(), "1.000000000000000001"),
            Value::Bool(b) => assert!(*b),
            Value::String(s) => assert_eq!(s, "exa"),
            Value::Date(d) => assert_eq!(*d, date),
            Value::Timestamp(ts) => assert_eq!(*ts, timestamp),
        }
    }

    let types = [
        ExaType::Unsupported,
        ExaType::Double,
        ExaType::Int32,
        ExaType::Int64,
        ExaType::Numeric {
            precision: Some(18),
            scale: Some(2),
        },
        ExaType::Boolean,
        ExaType::String { size: Some(256) },
        ExaType::Char { size: Some(10) },
        ExaType::Date,
        ExaType::Timestamp,
        ExaType::TimestampTz,
        ExaType::Geometry,
        ExaType::HashType,
        ExaType::IntervalYearToMonth,
        ExaType::IntervalDayToSecond,
    ];
    assert_eq!(types.len(), 15);
    assert_eq!(
        ExaType::Numeric {
            precision: None,
            scale: None
        },
        ExaType::Numeric {
            precision: None,
            scale: None
        }
    );
    assert_ne!(
        ExaType::String { size: Some(1) },
        ExaType::Char { size: Some(1) }
    );
    assert_ne!(ExaType::Double, ExaType::Int32);
}

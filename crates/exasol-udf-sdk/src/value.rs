/// A column value in a UDF call
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Double(f64),
    Int32(i32),
    Int64(i64),
    Numeric(String),
    Timestamp(String),
    Date(String),
    String(String),
    Boolean(bool),
}

/// Column type tag (without a value)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExaType {
    Unsupported,
    Double,
    Int32,
    Int64,
    Numeric,
    Timestamp,
    Date,
    String,
    Boolean,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_exatype_variants() {
        let values = [
            Value::Null,
            Value::Double(1.5),
            Value::Int32(7),
            Value::Int64(42),
            Value::Numeric("1.000000000000000001".to_string()),
            Value::Timestamp("2026-06-05T12:00:00".to_string()),
            Value::Date("2026-06-05".to_string()),
            Value::String("exa".to_string()),
            Value::Boolean(true),
        ];

        assert_eq!(values.len(), 9);

        for v in &values {
            match v {
                Value::Null => {}
                Value::Double(d) => assert_eq!(*d, 1.5),
                Value::Int32(i) => assert_eq!(*i, 7),
                Value::Int64(i) => assert_eq!(*i, 42),
                Value::Numeric(s) => assert_eq!(s, "1.000000000000000001"),
                Value::Timestamp(s) => assert_eq!(s, "2026-06-05T12:00:00"),
                Value::Date(s) => assert_eq!(s, "2026-06-05"),
                Value::String(s) => assert_eq!(s, "exa"),
                Value::Boolean(b) => assert!(*b),
            }
        }

        let types = [
            ExaType::Unsupported,
            ExaType::Double,
            ExaType::Int32,
            ExaType::Int64,
            ExaType::Numeric,
            ExaType::Timestamp,
            ExaType::Date,
            ExaType::String,
            ExaType::Boolean,
        ];
        assert_eq!(types.len(), 9);
        assert_eq!(ExaType::Double, ExaType::Double);
        assert_ne!(ExaType::Double, ExaType::Int32);
    }
}

use crate::error::UdfError;

/// A fixed-point decimal carrying its digits as an integer plus a scale.
///
/// The numeric value is `unscaled * 10^(-scale)`. This representation round-trips
/// the Exasol wire form losslessly for up to 38 significant digits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decimal {
    pub unscaled: i128,
    pub scale: u8,
}

impl TryFrom<&str> for Decimal {
    type Error = UdfError;

    fn try_from(literal: &str) -> Result<Self, Self::Error> {
        let (digits, scale) = match literal.find('.') {
            Some(point) => {
                let mut digits = String::with_capacity(literal.len() - 1);
                digits.push_str(&literal[..point]);
                digits.push_str(&literal[point + 1..]);
                let scale = literal.len() - point - 1;
                let scale = u8::try_from(scale).map_err(|_| {
                    UdfError::Type(format!("decimal scale too large in '{literal}'"))
                })?;
                (digits, scale)
            }
            None => (literal.to_string(), 0u8),
        };

        let unscaled = digits
            .parse::<i128>()
            .map_err(|e| UdfError::Type(format!("invalid decimal literal '{literal}': {e}")))?;

        Ok(Decimal { unscaled, scale })
    }
}

impl std::fmt::Display for Decimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.scale == 0 {
            return write!(f, "{}", self.unscaled);
        }

        let negative = self.unscaled < 0;
        let digits = self.unscaled.unsigned_abs().to_string();
        let scale = self.scale as usize;

        let padded = if digits.len() <= scale {
            format!("{:0>width$}", digits, width = scale + 1)
        } else {
            digits
        };

        let point = padded.len() - scale;
        let sign = if negative { "-" } else { "" };
        write!(f, "{}{}.{}", sign, &padded[..point], &padded[point..])
    }
}

/// A column value in a UDF call
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Double(f64),
    Int32(i32),
    Int64(i64),
    Numeric(Decimal),
    Bool(bool),
    String(String),
    Date(chrono::NaiveDate),
    Timestamp(chrono::NaiveDateTime),
}

/// Conversion into the SDK [`Value`] type for RETURNS-shape UDF output.
///
/// The `#[exasol_udf]` macro's RETURNS shim converts a function's returned
/// `Option<T>` through this trait before handing it to
/// [`UdfContext::set_return`](crate::context::UdfContext::set_return). The
/// `Option<T>` blanket impl maps `None` to `Value::Null`, so a UDF returning
/// `Ok(None)` yields SQL NULL.
pub trait IntoValue {
    /// Consume `self` and produce the corresponding [`Value`].
    fn into_value(self) -> Value;
}

impl IntoValue for Value {
    fn into_value(self) -> Value {
        self
    }
}

impl IntoValue for i32 {
    fn into_value(self) -> Value {
        Value::Int32(self)
    }
}

impl IntoValue for i64 {
    fn into_value(self) -> Value {
        Value::Int64(self)
    }
}

impl IntoValue for f64 {
    fn into_value(self) -> Value {
        Value::Double(self)
    }
}

impl IntoValue for bool {
    fn into_value(self) -> Value {
        Value::Bool(self)
    }
}

impl IntoValue for String {
    fn into_value(self) -> Value {
        Value::String(self)
    }
}

impl IntoValue for &str {
    fn into_value(self) -> Value {
        Value::String(self.to_string())
    }
}

impl IntoValue for Decimal {
    fn into_value(self) -> Value {
        Value::Numeric(self)
    }
}

impl IntoValue for chrono::NaiveDate {
    fn into_value(self) -> Value {
        Value::Date(self)
    }
}

impl IntoValue for chrono::NaiveDateTime {
    fn into_value(self) -> Value {
        Value::Timestamp(self)
    }
}

impl<T: IntoValue> IntoValue for Option<T> {
    fn into_value(self) -> Value {
        match self {
            Some(inner) => inner.into_value(),
            None => Value::Null,
        }
    }
}

/// Column type tag (without a value)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExaType {
    Unsupported,
    Double,
    Int32,
    Int64,
    Numeric {
        precision: Option<u32>,
        scale: Option<u32>,
    },
    Boolean,
    String {
        size: Option<u32>,
    },
    Char {
        size: Option<u32>,
    },
    Date,
    Timestamp,
    TimestampTz,
    Geometry,
    HashType,
    IntervalYearToMonth,
    IntervalDayToSecond,
}

#[cfg(test)]
mod tests {
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
}

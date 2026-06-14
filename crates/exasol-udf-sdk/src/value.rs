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

impl TryFrom<f64> for Decimal {
    type Error = UdfError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        if !value.is_finite() {
            return Err(UdfError::Type(format!(
                "cannot convert non-finite f64 to decimal: {value}"
            )));
        }
        Decimal::try_from(format!("{value}").as_str())
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
    fn decimal_from_str_and_f64_roundtrip() {
        let high_precision = Decimal::try_from("-1.000000000000000001").unwrap();
        assert_eq!(high_precision.unscaled, -1_000_000_000_000_000_001);
        assert_eq!(high_precision.scale, 18);
        assert_eq!(high_precision.to_string(), "-1.000000000000000001");

        let zero = Decimal::try_from("0").unwrap();
        assert_eq!(zero.scale, 0);
        assert_eq!(zero.to_string(), "0");

        let one_and_half = Decimal::try_from("1.5").unwrap();
        assert_eq!(one_and_half.to_string(), "1.5");

        let from_f64 = Decimal::try_from(1.5_f64).unwrap();
        assert_eq!(from_f64.to_string(), "1.5");

        assert!(matches!(
            Decimal::try_from(f64::NAN),
            Err(UdfError::Type(_))
        ));

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

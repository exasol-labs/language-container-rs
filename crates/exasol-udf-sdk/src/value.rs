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
#[path = "value_tests.rs"]
mod tests;

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

const MAX_SCALE: u32 = 28;
const DEFAULT_DIVISION_SCALE: u32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Decimal {
    unscaled: i128,
    scale: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecimalError {
    message: String,
}

impl DecimalError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for DecimalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DecimalError {}

impl Decimal {
    pub const ZERO: Decimal = Decimal {
        unscaled: 0,
        scale: 0,
    };

    pub fn new(unscaled: i128, scale: u32) -> Result<Self, DecimalError> {
        if scale > MAX_SCALE {
            return Err(DecimalError::new(format!(
                "scale must be between 0 and {MAX_SCALE}"
            )));
        }

        Ok(Self { unscaled, scale }.normalize())
    }

    pub const fn from_i64(value: i64) -> Self {
        Self {
            unscaled: value as i128,
            scale: 0,
        }
    }

    pub const fn from_i32(value: i32) -> Self {
        Self {
            unscaled: value as i128,
            scale: 0,
        }
    }

    pub const fn unscaled(&self) -> i128 {
        self.unscaled
    }

    pub const fn scale(&self) -> u32 {
        self.scale
    }

    pub const fn is_zero(&self) -> bool {
        self.unscaled == 0
    }

    pub fn abs(&self) -> Self {
        // `i128::MIN` has no positive `i128` counterpart, so its magnitude
        // saturates to `i128::MAX` instead of overflowing (which would panic).
        let unscaled = if self.unscaled == i128::MIN {
            i128::MAX
        } else {
            self.unscaled.unsigned_abs() as i128
        };

        Self {
            unscaled,
            scale: self.scale,
        }
    }

    pub fn checked_abs(&self) -> Option<Self> {
        if self.unscaled >= 0 {
            Some(*self)
        } else {
            self.checked_neg()
        }
    }

    pub fn checked_neg(&self) -> Option<Self> {
        Some(Self {
            unscaled: self.unscaled.checked_neg()?,
            scale: self.scale,
        })
    }

    pub fn checked_add(&self, other: &Self) -> Option<Self> {
        let (left, right, scale) = align(self, other)?;
        let unscaled = left.checked_add(right)?;

        Self::new(unscaled, scale).ok()
    }

    pub fn checked_sub(&self, other: &Self) -> Option<Self> {
        let (left, right, scale) = align(self, other)?;
        let unscaled = left.checked_sub(right)?;

        Self::new(unscaled, scale).ok()
    }

    pub fn checked_mul(&self, other: &Self) -> Option<Self> {
        let unscaled = self.unscaled.checked_mul(other.unscaled)?;
        let scale = self.scale.checked_add(other.scale)?;

        if scale > MAX_SCALE {
            round_to_scale(unscaled, scale, MAX_SCALE)
        } else {
            Self::new(unscaled, scale).ok()
        }
    }

    pub fn checked_div(&self, other: &Self) -> Option<Self> {
        self.checked_div_with_scale(other, DEFAULT_DIVISION_SCALE)
    }

    pub fn checked_div_with_scale(&self, other: &Self, scale: u32) -> Option<Self> {
        if other.unscaled == 0 || scale > MAX_SCALE {
            return None;
        }

        let shift = scale as i32 + other.scale as i32 - self.scale as i32;

        let (numerator, denominator) = if shift >= 0 {
            (
                self.unscaled.checked_mul(pow10(shift as u32)?)?,
                other.unscaled,
            )
        } else {
            (
                self.unscaled,
                other.unscaled.checked_mul(pow10((-shift) as u32)?)?,
            )
        };

        let quotient = divide_round_half_up(numerator, denominator)?;

        Self::new(quotient, scale).ok()
    }

    pub fn round(&self, scale: u32) -> Option<Self> {
        if scale >= self.scale {
            return Some(*self);
        }

        round_to_scale(self.unscaled, self.scale, scale)
    }

    fn normalize(mut self) -> Self {
        while self.scale > 0 && self.unscaled % 10 == 0 {
            self.unscaled /= 10;
            self.scale -= 1;
        }

        self
    }
}

impl Default for Decimal {
    fn default() -> Self {
        Self::ZERO
    }
}

impl From<i32> for Decimal {
    fn from(value: i32) -> Self {
        Self::from_i32(value)
    }
}

impl From<i64> for Decimal {
    fn from(value: i64) -> Self {
        Self::from_i64(value)
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.unscaled == 0 {
            return formatter.write_str("0");
        }

        let negative = self.unscaled < 0;
        let digits = self.unscaled.unsigned_abs().to_string();

        if negative {
            formatter.write_str("-")?;
        }

        if self.scale == 0 {
            return formatter.write_str(&digits);
        }

        let scale = self.scale as usize;

        if digits.len() <= scale {
            formatter.write_str("0.")?;
            for _ in 0..(scale - digits.len()) {
                formatter.write_str("0")?;
            }
            formatter.write_str(&digits)
        } else {
            let split = digits.len() - scale;
            formatter.write_str(&digits[..split])?;
            formatter.write_str(".")?;
            formatter.write_str(&digits[split..])
        }
    }
}

impl FromStr for Decimal {
    type Err = DecimalError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.trim();
        if input.is_empty() {
            return Err(DecimalError::new("decimal string is empty"));
        }

        let (mantissa, exponent) = split_exponent(input)?;

        let (negative, mantissa) = match mantissa.as_bytes().first() {
            Some(b'-') => (true, &mantissa[1..]),
            Some(b'+') => (false, &mantissa[1..]),
            _ => (false, mantissa),
        };

        let mut parts = mantissa.split('.');
        let integer = parts.next().unwrap_or("");
        let fraction = parts.next().unwrap_or("");

        if parts.next().is_some() {
            return Err(DecimalError::new("decimal contains more than one '.'"));
        }
        if integer.is_empty() && fraction.is_empty() {
            return Err(DecimalError::new("decimal has no digits"));
        }
        if !integer.bytes().all(|byte| byte.is_ascii_digit())
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(DecimalError::new("decimal contains a non-digit character"));
        }

        let mut unscaled: i128 = 0;

        for byte in integer.bytes().chain(fraction.bytes()) {
            let digit = (byte - b'0') as i128;
            unscaled = unscaled
                .checked_mul(10)
                .and_then(|value| value.checked_add(digit))
                .ok_or_else(|| DecimalError::new("decimal is too large"))?;
        }

        if negative {
            unscaled = unscaled
                .checked_neg()
                .ok_or_else(|| DecimalError::new("decimal is too large"))?;
        }

        let scale = fraction.len() as i64 - i64::from(exponent);

        if scale < 0 {
            let shift = u32::try_from(-scale)
                .map_err(|_| DecimalError::new("decimal exponent is out of range"))?;
            unscaled = unscaled
                .checked_mul(
                    pow10(shift)
                        .ok_or_else(|| DecimalError::new("decimal exponent is out of range"))?,
                )
                .ok_or_else(|| DecimalError::new("decimal is too large"))?;

            Self::new(unscaled, 0)
        } else if scale > i64::from(MAX_SCALE) {
            let scale = u32::try_from(scale)
                .map_err(|_| DecimalError::new("decimal exponent is out of range"))?;
            round_to_scale(unscaled, scale, MAX_SCALE)
                .ok_or_else(|| DecimalError::new("decimal is too large"))
        } else {
            Self::new(unscaled, scale as u32)
        }
    }
}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let (left, right, _) = align(self, other)?;
        Some(left.cmp(&right))
    }
}

fn split_exponent(input: &str) -> Result<(&str, i32), DecimalError> {
    let mut exponent_index = None;

    for (index, byte) in input.bytes().enumerate() {
        if byte == b'e' || byte == b'E' {
            exponent_index = Some(index);
            break;
        }
    }

    let Some(index) = exponent_index else {
        return Ok((input, 0));
    };

    let mantissa = &input[..index];
    let exponent = &input[index + 1..];

    if exponent.is_empty() {
        return Err(DecimalError::new("decimal exponent is empty"));
    }

    let exponent = exponent
        .parse::<i32>()
        .map_err(|_| DecimalError::new("decimal exponent is not a valid integer"))?;

    Ok((mantissa, exponent))
}

fn align(left: &Decimal, right: &Decimal) -> Option<(i128, i128, u32)> {
    let scale = left.scale.max(right.scale);

    let left = left.unscaled.checked_mul(pow10(scale - left.scale)?)?;
    let right = right.unscaled.checked_mul(pow10(scale - right.scale)?)?;

    Some((left, right, scale))
}

fn pow10(exponent: u32) -> Option<i128> {
    let mut value: i128 = 1;

    for _ in 0..exponent {
        value = value.checked_mul(10)?;
    }

    Some(value)
}

fn divide_round_half_up(numerator: i128, denominator: i128) -> Option<i128> {
    let quotient = numerator.checked_div(denominator)?;
    let remainder = numerator.checked_rem(denominator)?;

    if remainder == 0 {
        return Some(quotient);
    }

    let should_round = remainder.unsigned_abs().checked_mul(2)? >= denominator.unsigned_abs();

    if !should_round {
        return Some(quotient);
    }

    if (numerator < 0) ^ (denominator < 0) {
        quotient.checked_sub(1)
    } else {
        quotient.checked_add(1)
    }
}

fn round_to_scale(unscaled: i128, from_scale: u32, to_scale: u32) -> Option<Decimal> {
    if to_scale >= from_scale {
        return Decimal::new(unscaled, from_scale).ok();
    }

    let divisor = pow10(from_scale - to_scale)?;
    let rounded = divide_round_half_up(unscaled, divisor)?;

    Decimal::new(rounded, to_scale).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats() {
        assert_eq!("123.45".parse::<Decimal>().unwrap().to_string(), "123.45");
        assert_eq!("-0.001".parse::<Decimal>().unwrap().to_string(), "-0.001");
        assert_eq!("1.500".parse::<Decimal>().unwrap().to_string(), "1.5");
        assert_eq!("1e3".parse::<Decimal>().unwrap().to_string(), "1000");
        assert_eq!("1.2e-3".parse::<Decimal>().unwrap().to_string(), "0.0012");
        assert_eq!("0".parse::<Decimal>().unwrap().to_string(), "0");
    }

    #[test]
    fn normalizes_equality() {
        let a: Decimal = "1.50".parse().unwrap();
        let b: Decimal = "1.5".parse().unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn adds_and_subtracts() {
        let a: Decimal = "1.20".parse().unwrap();
        let b: Decimal = "2.3".parse().unwrap();

        assert_eq!(a.checked_add(&b).unwrap().to_string(), "3.5");
        assert_eq!(b.checked_sub(&a).unwrap().to_string(), "1.1");
    }

    #[test]
    fn multiplies_and_divides() {
        let a: Decimal = "1.5".parse().unwrap();
        let b: Decimal = "2".parse().unwrap();

        assert_eq!(a.checked_mul(&b).unwrap().to_string(), "3");
        assert_eq!(
            Decimal::from_i32(1)
                .checked_div(&Decimal::from_i32(3))
                .unwrap()
                .to_string(),
            "0.3333333333"
        );
    }

    #[test]
    fn rounds_half_up() {
        let value: Decimal = "1.25".parse().unwrap();
        assert_eq!(value.round(1).unwrap().to_string(), "1.3");

        let value: Decimal = "1.24".parse().unwrap();
        assert_eq!(value.round(1).unwrap().to_string(), "1.2");
    }

    #[test]
    fn rejects_invalid_input() {
        assert!("".parse::<Decimal>().is_err());
        assert!("abc".parse::<Decimal>().is_err());
        assert!("1.2.3".parse::<Decimal>().is_err());
        assert!("1e".parse::<Decimal>().is_err());
    }

    #[test]
    fn compares_numerically() {
        let a: Decimal = "1.5".parse().unwrap();
        let b: Decimal = "1.50".parse().unwrap();
        let c: Decimal = "2".parse().unwrap();

        assert_eq!(a.partial_cmp(&b), Some(Ordering::Equal));
        assert_eq!(a.partial_cmp(&c), Some(Ordering::Less));
    }
}

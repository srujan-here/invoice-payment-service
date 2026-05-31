//! Money as integer minor units (USD cents).
//!
//! The whole point of this newtype is that **there is no API to turn money into
//! a float**. `Money` wraps an `i64` count of cents, arithmetic is checked
//! (overflow is an error, not a silent wrap), and it serializes as a plain JSON
//! integer. Floats are therefore unrepresentable in the money path by
//! construction — not by convention.

use std::fmt;
use std::iter::Sum;

use serde::{Deserialize, Serialize};

/// An amount of money in USD cents. Always non-negative in our domain, but the
/// type itself permits negatives so intermediate arithmetic (e.g. credits) is
/// expressible without panicking; callers validate the sign where it matters.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, sqlx::Type,
)]
#[serde(transparent)]
#[sqlx(transparent)]
pub struct Money(i64);

#[derive(Debug, thiserror::Error)]
#[error("monePy arithmetic overflowed")]
pub struct MoneyOverflow;

impl Money {
    pub const ZERO: Money = Money(0);

    /// Construct from a raw cent count.
    pub const fn from_cents(cents: i64) -> Self {
        Money(cents)
    }

    /// The underlying cent count. This is the only way out — note it returns an
    /// integer, never a float.
    pub const fn cents(self) -> i64 {
        self.0
    }

    pub fn is_negative(self) -> bool {
        self.0 < 0
    }

    /// Multiply an amount by a quantity, erroring on overflow. Used to turn a
    /// line item's `unit_amount_cents * quantity` into a total.
    pub fn checked_mul_qty(self, qty: i64) -> Result<Money, MoneyOverflow> {
        self.0.checked_mul(qty).map(Money).ok_or(MoneyOverflow)
    }

    pub fn checked_add(self, other: Money) -> Result<Money, MoneyOverflow> {
        self.0.checked_add(other.0).map(Money).ok_or(MoneyOverflow)
    }

    pub fn checked_sub(self, other: Money) -> Result<Money, MoneyOverflow> {
        self.0.checked_sub(other.0).map(Money).ok_or(MoneyOverflow)
    }
}

impl Sum for Money {
    /// Saturating-free sum used only in tests / non-overflowing contexts.
    /// Production totals go through [`Money::checked_add`] in the service layer.
    fn sum<I: Iterator<Item = Money>>(iter: I) -> Self {
        iter.fold(Money::ZERO, |acc, m| Money(acc.0 + m.0))
    }
}

impl fmt::Display for Money {
    /// Human formatting only (logs, docs). Uses integer div/mod — never a float.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let abs = self.0.unsigned_abs();
        write!(f, "{sign}${}.{:02}", abs / 100, abs % 100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiplies_and_sums_without_floats() {
        let unit = Money::from_cents(1999);
        let line = unit.checked_mul_qty(3).unwrap();
        assert_eq!(line.cents(), 5997);
        let total = [line, Money::from_cents(1)]
            .into_iter()
            .try_fold(Money::ZERO, Money::checked_add)
            .unwrap();
        assert_eq!(total.cents(), 5998);
    }

    #[test]
    fn overflow_is_an_error_not_a_wrap() {
        assert!(Money::from_cents(i64::MAX).checked_mul_qty(2).is_err());
        assert!(Money::from_cents(i64::MAX)
            .checked_add(Money::from_cents(1))
            .is_err());
    }

    #[test]
    fn display_uses_integer_math() {
        assert_eq!(Money::from_cents(4999).to_string(), "$49.99");
        assert_eq!(Money::from_cents(5).to_string(), "$0.05");
        assert_eq!(Money::from_cents(-150).to_string(), "-$1.50");
    }
}

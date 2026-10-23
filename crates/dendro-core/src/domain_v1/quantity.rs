use super::{ClockId, Refusal, Validate};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UtcMicros(pub i64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MonotonicDuration {
    pub microseconds: Count,
    pub clock: ClockId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "IntervalWire", into = "IntervalWire")]
pub struct Interval {
    start: UtcMicros,
    end: UtcMicros,
    clock: ClockId,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IntervalWire {
    start: UtcMicros,
    end: UtcMicros,
    clock: ClockId,
}
impl Interval {
    pub fn new(start: UtcMicros, end: UtcMicros, clock: ClockId) -> Result<Self, Refusal> {
        if end < start {
            return Err(Refusal::InvalidInterval);
        }
        Ok(Self { start, end, clock })
    }
    pub fn start(&self) -> UtcMicros {
        self.start
    }
    pub fn end(&self) -> UtcMicros {
        self.end
    }
    pub fn clock(&self) -> &ClockId {
        &self.clock
    }
    pub fn contains(&self, time: UtcMicros) -> bool {
        self.start <= time && time < self.end
    }
    pub fn contains_interval(&self, other: &Self) -> bool {
        self.clock == other.clock && self.start <= other.start && other.end <= self.end
    }
    pub fn duration_us(&self) -> u64 {
        (i128::from(self.end.0) - i128::from(self.start.0)) as u64
    }
}
impl TryFrom<IntervalWire> for Interval {
    type Error = Refusal;
    fn try_from(v: IntervalWire) -> Result<Self, Self::Error> {
        Self::new(v.start, v.end, v.clock)
    }
}
impl From<Interval> for IntervalWire {
    fn from(v: Interval) -> Self {
        Self {
            start: v.start,
            end: v.end,
            clock: v.clock,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Count(pub u64);
impl Count {
    pub fn checked_add(self, other: Self) -> Result<Self, Refusal> {
        self.0
            .checked_add(other.0)
            .map(Self)
            .ok_or(Refusal::Overflow)
    }
}

/// Exact decimal: canonical numerator / positive power-of-ten denominator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "DecimalWire", into = "DecimalWire")]
pub struct Decimal {
    coefficient: i128,
    denominator: u128,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DecimalWire {
    coefficient: i128,
    denominator: u128,
}
impl Decimal {
    pub fn new(mut coefficient: i128, mut denominator: u128) -> Result<Self, Refusal> {
        if denominator == 0 {
            return Err(Refusal::InvalidQuantity);
        }
        let mut d = denominator;
        while d > 1 && d.is_multiple_of(10) {
            d /= 10;
        }
        if d != 1 {
            return Err(Refusal::InvalidQuantity);
        }
        if coefficient == 0 {
            denominator = 1;
        }
        while denominator > 1 && coefficient % 10 == 0 {
            coefficient /= 10;
            denominator /= 10;
        }
        Ok(Self {
            coefficient,
            denominator,
        })
    }
    pub fn coefficient(self) -> i128 {
        self.coefficient
    }
    pub fn denominator(self) -> u128 {
        self.denominator
    }
    pub fn checked_add(self, rhs: Self) -> Result<Self, Refusal> {
        let denominator = self.denominator.max(rhs.denominator);
        let scale = |v: Self| -> Result<i128, Refusal> {
            let multiplier =
                i128::try_from(denominator / v.denominator).map_err(|_| Refusal::Overflow)?;
            v.coefficient
                .checked_mul(multiplier)
                .ok_or(Refusal::Overflow)
        };
        Self::new(
            scale(self)?
                .checked_add(scale(rhs)?)
                .ok_or(Refusal::Overflow)?,
            denominator,
        )
    }
}
impl TryFrom<DecimalWire> for Decimal {
    type Error = Refusal;
    fn try_from(v: DecimalWire) -> Result<Self, Self::Error> {
        Self::new(v.coefficient, v.denominator)
    }
}
impl From<Decimal> for DecimalWire {
    fn from(v: Decimal) -> Self {
        Self {
            coefficient: v.coefficient,
            denominator: v.denominator,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "RatioWire", into = "RatioWire")]
pub struct Ratio {
    numerator: u128,
    denominator: u128,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RatioWire {
    numerator: u128,
    denominator: u128,
}
fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a
}
impl Ratio {
    pub const ZERO: Self = Self {
        numerator: 0,
        denominator: 1,
    };
    pub const ONE: Self = Self {
        numerator: 1,
        denominator: 1,
    };
    pub fn new(numerator: u128, denominator: u128) -> Result<Self, Refusal> {
        if denominator == 0 {
            return Err(Refusal::InvalidQuantity);
        }
        let d = gcd(numerator, denominator);
        Ok(Self {
            numerator: numerator / d,
            denominator: denominator / d,
        })
    }
    pub fn numerator(self) -> u128 {
        self.numerator
    }
    pub fn denominator(self) -> u128 {
        self.denominator
    }
    pub fn is_weight(self) -> bool {
        self.numerator <= self.denominator
    }
    pub fn checked_add(self, rhs: Self) -> Result<Self, Refusal> {
        let g = gcd(self.denominator, rhs.denominator);
        let left = rhs.denominator / g;
        let right = self.denominator / g;
        Self::new(
            self.numerator
                .checked_mul(left)
                .and_then(|a| {
                    rhs.numerator
                        .checked_mul(right)
                        .and_then(|b| a.checked_add(b))
                })
                .ok_or(Refusal::Overflow)?,
            self.denominator
                .checked_mul(left)
                .ok_or(Refusal::Overflow)?,
        )
    }
    pub fn remainder(self) -> Result<Self, Refusal> {
        if !self.is_weight() {
            return Err(Refusal::InvalidQuantity);
        }
        Self::new(self.denominator - self.numerator, self.denominator)
    }
}
impl TryFrom<RatioWire> for Ratio {
    type Error = Refusal;
    fn try_from(v: RatioWire) -> Result<Self, Self::Error> {
        Self::new(v.numerator, v.denominator)
    }
}
impl From<Ratio> for RatioWire {
    fn from(v: Ratio) -> Self {
        Self {
            numerator: v.numerator,
            denominator: v.denominator,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CurrencyCode(String);
impl TryFrom<String> for CurrencyCode {
    type Error = Refusal;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        if v.len() != 3 || !v.bytes().all(|b| b.is_ascii_uppercase()) {
            return Err(Refusal::InvalidQuantity);
        }
        Ok(Self(v))
    }
}
impl From<CurrencyCode> for String {
    fn from(v: CurrencyCode) -> Self {
        v.0
    }
}
/// Number of source decimal places, independently retained from canonical value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct SourceScale(u8);
impl SourceScale {
    pub fn new(scale: u8) -> Result<Self, Refusal> {
        if scale > 38 {
            return Err(Refusal::InvalidQuantity);
        }
        Ok(Self(scale))
    }
    pub fn decimal_places(self) -> u8 {
        self.0
    }
}
impl TryFrom<u8> for SourceScale {
    type Error = Refusal;
    fn try_from(v: u8) -> Result<Self, Self::Error> {
        Self::new(v)
    }
}
impl From<SourceScale> for u8 {
    fn from(v: SourceScale) -> Self {
        v.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Money {
    pub amount: Decimal,
    pub currency: CurrencyCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_scale: Option<SourceScale>,
}
// Source precision is retained metadata, not part of numeric equality.
impl PartialEq for Money {
    fn eq(&self, rhs: &Self) -> bool {
        self.amount == rhs.amount && self.currency == rhs.currency
    }
}
impl Eq for Money {}
impl Validate for Money {
    fn validate(&self) -> Result<(), Refusal> {
        if self
            .source_scale
            .is_some_and(|s| u32::from(s.0) < self.amount.denominator.ilog10())
        {
            return Err(Refusal::InvalidQuantity);
        }
        Ok(())
    }
}
impl Money {
    pub fn from_source(
        coefficient: i128,
        denominator: u128,
        currency: CurrencyCode,
    ) -> Result<Self, Refusal> {
        let amount = Decimal::new(coefficient, denominator)?;
        let source_scale = Some(SourceScale::new(denominator.ilog10() as u8)?);
        Ok(Self {
            amount,
            currency,
            source_scale,
        })
    }
    pub fn checked_add(&self, rhs: &Self) -> Result<Self, Refusal> {
        self.validate()?;
        rhs.validate()?;
        if self.currency != rhs.currency {
            return Err(Refusal::InvalidQuantity);
        }
        Ok(Self {
            amount: self.amount.checked_add(rhs.amount)?,
            currency: self.currency.clone(),
            // A computed sum has no single source precision attestation.
            source_scale: None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Tokens,
    Bytes,
    Characters,
    Messages,
    Fragments,
    Events,
    Microseconds,
    CpuUserMicroseconds,
    CpuSystemMicroseconds,
    LinuxRssBytes,
    WindowsWorkingSetBytes,
    Count,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quantity {
    pub value: Count,
    pub unit: Unit,
}
impl Validate for Quantity {
    fn validate(&self) -> Result<(), Refusal> {
        if self.unit == Unit::Unknown {
            Err(Refusal::InvalidQuantity)
        } else {
            Ok(())
        }
    }
}

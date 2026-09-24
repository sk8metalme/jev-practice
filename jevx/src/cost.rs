use serde::{Deserialize, Serialize};
use std::fmt;

/// A cost is measurable only when provider usage/cost metadata or a matching local price exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostStatus {
    Available,
    Unknown,
    Unavailable,
}

/// 金額の出所。推定値と外部の実績値を混同しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CostBasis {
    #[default]
    Estimated,
    Actual,
}

impl fmt::Display for CostBasis {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Estimated => "estimated",
            Self::Actual => "actual",
        })
    }
}

impl fmt::Display for CostStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Available => "available",
            Self::Unknown => "unknown",
            Self::Unavailable => "unavailable",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostEstimate {
    pub amount: Option<f64>,
    pub currency: Option<String>,
    pub price_version: Option<String>,
    pub status: CostStatus,
    /// 既存receiptとの後方互換のため、欠落時はestimatedとして読む。
    #[serde(default)]
    pub basis: CostBasis,
}

impl CostEstimate {
    pub fn available(amount: f64, currency: Option<String>, price_version: Option<String>) -> Self {
        Self {
            amount: Some(amount),
            currency,
            price_version,
            status: CostStatus::Available,
            basis: CostBasis::Estimated,
        }
    }

    pub fn actual(amount: f64, currency: Option<String>, price_version: Option<String>) -> Self {
        Self {
            amount: Some(amount),
            currency,
            price_version,
            status: CostStatus::Available,
            basis: CostBasis::Actual,
        }
    }

    pub fn unknown(currency: Option<String>, price_version: Option<String>) -> Self {
        Self {
            amount: None,
            currency,
            price_version,
            status: CostStatus::Unknown,
            basis: CostBasis::Estimated,
        }
    }

    pub fn unavailable() -> Self {
        Self {
            amount: None,
            currency: None,
            price_version: None,
            status: CostStatus::Unavailable,
            basis: CostBasis::Estimated,
        }
    }

    /// Returns whether an externally supplied estimate is safe to persist and aggregate.
    pub fn is_valid(&self) -> bool {
        let metadata_valid = self
            .currency
            .as_deref()
            .is_none_or(|value| safe_cost_metadata(value, 32))
            && self
                .price_version
                .as_deref()
                .is_none_or(|value| safe_cost_metadata(value, 128));
        if !metadata_valid {
            return false;
        }
        match self.status {
            CostStatus::Available => self.amount.is_some_and(|amount| {
                amount.is_finite()
                    && amount >= 0.0
                    && (amount == 0.0 || (self.currency.is_some() && self.price_version.is_some()))
            }),
            CostStatus::Unknown | CostStatus::Unavailable => self.amount.is_none(),
        }
    }
}

impl Default for CostEstimate {
    fn default() -> Self {
        Self::unavailable()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CostSummary {
    pub jev: CostEstimate,
    pub codex: CostEstimate,
    pub total: CostEstimate,
}

/// 複数receiptの同一componentを、安全に合算するための内部集計器。
///
/// unknown/unavailable、価格版または通貨の不一致、非0なのに価格メタデータがない値は
/// 合算結果を返さない。既知の外部呼び出しなしの0円（metadataなし）は例外として扱う。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CostAccumulator {
    amount: f64,
    has_amount: bool,
    saw_unknown: bool,
    saw_unavailable: bool,
    invalid_metadata: bool,
    basis: Option<CostBasis>,
    currency: Option<String>,
    price_version: Option<String>,
}

impl CostAccumulator {
    pub fn add(&mut self, estimate: &CostEstimate) {
        if !estimate.is_valid() {
            self.invalid_metadata = true;
            return;
        }
        if let Some(basis) = self.basis {
            if basis != estimate.basis {
                self.invalid_metadata = true;
            }
        } else {
            self.basis = Some(estimate.basis);
        }
        match estimate.status {
            CostStatus::Unknown => self.saw_unknown = true,
            CostStatus::Unavailable => self.saw_unavailable = true,
            CostStatus::Available => {
                let Some(amount) = estimate.amount.filter(|value| value.is_finite()) else {
                    self.saw_unknown = true;
                    return;
                };
                if amount < 0.0 {
                    self.saw_unknown = true;
                    return;
                }
                let has_metadata = estimate.currency.is_some() && estimate.price_version.is_some();
                if amount != 0.0 && !has_metadata {
                    self.invalid_metadata = true;
                }
                if let Some(currency) = &self.currency
                    && estimate.currency.as_deref() != Some(currency)
                    && amount != 0.0
                {
                    self.invalid_metadata = true;
                }
                if let Some(price_version) = &self.price_version
                    && estimate.price_version.as_deref() != Some(price_version)
                    && amount != 0.0
                {
                    self.invalid_metadata = true;
                }
                if amount != 0.0 {
                    if self.currency.is_none() {
                        self.currency = estimate.currency.clone();
                    }
                    if self.price_version.is_none() {
                        self.price_version = estimate.price_version.clone();
                    }
                }
                self.amount += amount;
                self.has_amount = true;
            }
        }
    }

    pub fn amount(&self) -> Option<f64> {
        (self.has_amount && !self.saw_unknown && !self.saw_unavailable && !self.invalid_metadata)
            .then_some(self.amount)
    }
}

impl CostSummary {
    /// 外部モデルを呼んでいないことがコード側で確定した観測の費用。
    /// 不明なusageを0円へ変換する用途には使わない。
    pub fn no_external_call() -> Self {
        let jev = CostEstimate::actual(0.0, None, None);
        let codex = CostEstimate::actual(0.0, None, None);
        Self {
            total: total_cost(&jev, &codex),
            jev,
            codex,
        }
    }
}

/// Codex usage carried by an optional Hook payload extension.
/// Missing fields remain missing; they are never converted into zero usage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodexUsage {
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub main_turns: Option<u32>,
    pub subagent_count: Option<u32>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    /// Fallback/escalationで追加されたToken。通常のusageと混ぜずに保持する。
    pub additional_input_tokens: Option<u64>,
    pub additional_output_tokens: Option<u64>,
    pub additional_reasoning_tokens: Option<u64>,
    pub elapsed_ms: Option<u64>,
    pub fallback_stage: Option<String>,
    #[serde(default)]
    pub cost: CostEstimate,
    /// `cost`とは別に、fallback/escalationで増えた実費または推定費用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_cost: Option<CostEstimate>,
}

impl CodexUsage {
    /// Checks metadata before it can enter a receipt or a report.
    pub fn is_valid(&self) -> bool {
        self.model
            .as_deref()
            .is_none_or(|value| safe_cost_metadata(value, 128))
            && self
                .reasoning_effort
                .as_deref()
                .is_none_or(|value| safe_cost_metadata(value, 32))
            && self
                .fallback_stage
                .as_deref()
                .is_none_or(|value| safe_cost_metadata(value, 64))
            && self.cost.is_valid()
            && self
                .additional_cost
                .as_ref()
                .is_none_or(CostEstimate::is_valid)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TokenPricing {
    pub input_per_million: Option<f64>,
    pub output_per_million: Option<f64>,
    pub reasoning_per_million: Option<f64>,
    pub currency: Option<String>,
    pub price_version: Option<String>,
}

impl TokenPricing {
    pub fn estimate_two_part(
        &self,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cache_hit: bool,
    ) -> CostEstimate {
        if cache_hit {
            return CostEstimate::available(0.0, self.currency.clone(), self.price_version.clone());
        }
        let (Some(input_tokens), Some(output_tokens)) = (input_tokens, output_tokens) else {
            return CostEstimate::unavailable();
        };
        let (Some(input_price), Some(output_price)) =
            (self.input_per_million, self.output_per_million)
        else {
            return CostEstimate::unknown(self.currency.clone(), self.price_version.clone());
        };
        if !valid_price(input_price) || !valid_price(output_price) {
            return CostEstimate::unknown(self.currency.clone(), self.price_version.clone());
        }
        let amount =
            (input_tokens as f64 * input_price + output_tokens as f64 * output_price) / 1_000_000.0;
        CostEstimate::available(amount, self.currency.clone(), self.price_version.clone())
    }

    pub fn estimate_three_part(
        &self,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        reasoning_tokens: Option<u64>,
    ) -> CostEstimate {
        let (Some(input_tokens), Some(output_tokens), Some(reasoning_tokens)) =
            (input_tokens, output_tokens, reasoning_tokens)
        else {
            return CostEstimate::unavailable();
        };
        let (Some(input_price), Some(output_price), Some(reasoning_price)) = (
            self.input_per_million,
            self.output_per_million,
            self.reasoning_per_million,
        ) else {
            return CostEstimate::unknown(self.currency.clone(), self.price_version.clone());
        };
        if !valid_price(input_price) || !valid_price(output_price) || !valid_price(reasoning_price)
        {
            return CostEstimate::unknown(self.currency.clone(), self.price_version.clone());
        }
        let amount = (input_tokens as f64 * input_price
            + output_tokens as f64 * output_price
            + reasoning_tokens as f64 * reasoning_price)
            / 1_000_000.0;
        CostEstimate::available(amount, self.currency.clone(), self.price_version.clone())
    }
}

pub fn total_cost(jev: &CostEstimate, codex: &CostEstimate) -> CostEstimate {
    match (jev.amount, codex.amount) {
        (Some(jev_amount), Some(codex_amount))
            if valid_available_component(jev)
                && valid_available_component(codex)
                && jev.currency_matches(codex)
                && jev.price_version_matches(codex) =>
        {
            CostEstimate::available(
                jev_amount + codex_amount,
                codex_currency(jev, codex),
                codex_price_version(jev, codex),
            )
            .with_basis(combined_basis(jev, codex))
        }
        _ if matches!(jev.status, CostStatus::Unknown)
            || matches!(codex.status, CostStatus::Unknown) =>
        {
            CostEstimate::unknown(codex_currency(jev, codex), codex_price_version(jev, codex))
        }
        _ => CostEstimate::unavailable(),
    }
}

impl CostEstimate {
    fn with_basis(mut self, basis: CostBasis) -> Self {
        self.basis = basis;
        self
    }
}

fn combined_basis(left: &CostEstimate, right: &CostEstimate) -> CostBasis {
    if left.basis == CostBasis::Actual && right.basis == CostBasis::Actual {
        CostBasis::Actual
    } else {
        CostBasis::Estimated
    }
}

fn valid_price(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

fn safe_cost_metadata(value: &str, max_chars: usize) -> bool {
    !value.is_empty()
        && value.chars().count() <= max_chars
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_-/.:".contains(character))
}

fn valid_available_component(estimate: &CostEstimate) -> bool {
    if !estimate.is_valid() || !matches!(estimate.status, CostStatus::Available) {
        return false;
    }
    let Some(amount) = estimate.amount else {
        return false;
    };
    if !amount.is_finite() || amount < 0.0 {
        return false;
    }
    amount == 0.0 || (estimate.currency.is_some() && estimate.price_version.is_some())
}

fn codex_currency(left: &CostEstimate, right: &CostEstimate) -> Option<String> {
    if left.amount == Some(0.0) && right.amount != Some(0.0) {
        right.currency.clone().or_else(|| left.currency.clone())
    } else {
        left.currency.clone().or_else(|| right.currency.clone())
    }
}

fn codex_price_version(left: &CostEstimate, right: &CostEstimate) -> Option<String> {
    if left.amount == Some(0.0) && right.amount != Some(0.0) {
        right
            .price_version
            .clone()
            .or_else(|| left.price_version.clone())
    } else {
        left.price_version
            .clone()
            .or_else(|| right.price_version.clone())
    }
}

impl CostEstimate {
    fn currency_matches(&self, other: &CostEstimate) -> bool {
        self.amount == Some(0.0) || other.amount == Some(0.0) || self.currency == other.currency
    }

    fn price_version_matches(&self, other: &CostEstimate) -> bool {
        self.amount == Some(0.0)
            || other.amount == Some(0.0)
            || self.price_version == other.price_version
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pricing() -> TokenPricing {
        TokenPricing {
            input_per_million: Some(1.0),
            output_per_million: Some(2.0),
            reasoning_per_million: Some(3.0),
            currency: Some("USD".to_owned()),
            price_version: Some("fixture-1".to_owned()),
        }
    }

    #[test]
    fn estimates_two_and_three_part_costs() {
        let pricing = pricing();
        let two = pricing.estimate_two_part(Some(1_000), Some(2_000), false);
        assert_eq!(two.status, CostStatus::Available);
        assert_eq!(two.basis, CostBasis::Estimated);
        assert_eq!(two.amount, Some(0.005));

        let three = pricing.estimate_three_part(Some(1_000), Some(2_000), Some(3_000));
        assert_eq!(three.status, CostStatus::Available);
        assert_eq!(three.amount, Some(0.014));
    }

    #[test]
    fn missing_usage_and_prices_are_not_reported_as_zero() {
        let pricing = TokenPricing::default();
        assert_eq!(
            pricing.estimate_two_part(Some(1), Some(1), false).status,
            CostStatus::Unknown
        );
        assert_eq!(
            pricing.estimate_two_part(None, None, false).status,
            CostStatus::Unavailable
        );
        assert_eq!(
            pricing.estimate_two_part(Some(1), Some(1), true).amount,
            Some(0.0)
        );
        assert_eq!(CostSummary::no_external_call().total.amount, Some(0.0));
        assert_eq!(
            CostSummary::no_external_call().total.basis,
            CostBasis::Actual
        );
    }

    #[test]
    fn total_requires_known_matching_cost_components() {
        let pricing = pricing();
        let jev = pricing.estimate_two_part(Some(1), Some(1), false);
        let codex = pricing.estimate_three_part(Some(1), Some(1), Some(1));
        let total = total_cost(&jev, &codex);
        assert_eq!(total.status, CostStatus::Available);
        assert_eq!(total.amount, Some(0.000009));
        assert_eq!(total.basis, CostBasis::Estimated);

        let actual =
            CostEstimate::actual(0.01, Some("USD".to_owned()), Some("actual-1".to_owned()));
        assert_eq!(actual.basis, CostBasis::Actual);
        assert_eq!(total_cost(&actual, &actual).basis, CostBasis::Actual);

        let no_external = CostSummary::no_external_call();
        let combined = total_cost(&no_external.jev, &actual);
        assert_eq!(combined.amount, Some(0.01));
        assert_eq!(combined.currency.as_deref(), Some("USD"));
        assert_eq!(combined.price_version.as_deref(), Some("actual-1"));
        assert_eq!(combined.basis, CostBasis::Actual);

        let missing_metadata = CostEstimate::actual(0.01, None, None);
        assert_eq!(
            total_cost(&no_external.jev, &missing_metadata).status,
            CostStatus::Unavailable
        );

        let unavailable = total_cost(&jev, &CostEstimate::unavailable());
        assert_eq!(unavailable.status, CostStatus::Unavailable);

        let mut amount_with_unknown_status = jev.clone();
        amount_with_unknown_status.status = CostStatus::Unknown;
        assert_eq!(
            total_cost(&amount_with_unknown_status, &codex).status,
            CostStatus::Unknown
        );

        let mismatched_currency =
            CostEstimate::available(0.1, Some("JPY".to_owned()), Some("fixture-1".to_owned()));
        assert_eq!(
            total_cost(&jev, &mismatched_currency).status,
            CostStatus::Unavailable
        );
        let mismatched_version =
            CostEstimate::available(0.1, Some("USD".to_owned()), Some("fixture-2".to_owned()));
        assert_eq!(
            total_cost(&jev, &mismatched_version).status,
            CostStatus::Unavailable
        );
        let unsafe_metadata = CostEstimate::actual(
            0.1,
            Some("USD\nsecret".to_owned()),
            Some("fixture-1".to_owned()),
        );
        assert_eq!(
            total_cost(&jev, &unsafe_metadata).status,
            CostStatus::Unavailable
        );
        assert_eq!(
            total_cost(
                &CostEstimate::unknown(Some("USD".to_owned()), None),
                &unavailable
            )
            .status,
            CostStatus::Unknown
        );
    }

    #[test]
    fn invalid_and_partial_pricing_is_explicit() {
        let pricing = TokenPricing {
            input_per_million: Some(-1.0),
            output_per_million: Some(f64::NAN),
            reasoning_per_million: Some(f64::INFINITY),
            currency: Some("USD".to_owned()),
            price_version: Some("invalid".to_owned()),
        };
        assert_eq!(
            pricing.estimate_two_part(Some(1), Some(1), false).status,
            CostStatus::Unknown
        );
        assert_eq!(
            pricing
                .estimate_three_part(Some(1), Some(1), Some(1))
                .status,
            CostStatus::Unknown
        );
        assert_eq!(
            pricing.estimate_three_part(Some(1), None, Some(1)).status,
            CostStatus::Unavailable
        );
        assert_eq!(CostStatus::Available.to_string(), "available");
        assert_eq!(CostStatus::Unknown.to_string(), "unknown");
        assert_eq!(CostStatus::Unavailable.to_string(), "unavailable");
        assert_eq!(CostBasis::Estimated.to_string(), "estimated");
        assert_eq!(CostBasis::Actual.to_string(), "actual");
    }

    #[test]
    fn legacy_cost_json_defaults_to_estimated_basis() {
        let value = serde_json::json!({
            "amount": 0.1,
            "currency": "USD",
            "priceVersion": "legacy",
            "status": "available"
        });
        let estimate: CostEstimate = serde_json::from_value(value).expect("legacy cost");
        assert_eq!(estimate.basis, CostBasis::Estimated);
    }

    #[test]
    fn accumulator_rejects_unknown_and_mixed_price_metadata() {
        let mut known = CostAccumulator::default();
        known.add(&CostEstimate::available(
            0.1,
            Some("USD".to_owned()),
            Some("fixture-1".to_owned()),
        ));
        known.add(&CostEstimate::available(
            0.2,
            Some("USD".to_owned()),
            Some("fixture-1".to_owned()),
        ));
        assert!((known.amount().expect("known amount") - 0.3).abs() < 1e-9);

        let mut unknown = known.clone();
        unknown.add(&CostEstimate::unknown(
            Some("USD".to_owned()),
            Some("fixture-1".to_owned()),
        ));
        assert_eq!(unknown.amount(), None);

        let mut mixed = CostAccumulator::default();
        mixed.add(&CostEstimate::available(
            0.1,
            Some("USD".to_owned()),
            Some("fixture-1".to_owned()),
        ));
        mixed.add(&CostEstimate::available(
            0.2,
            Some("JPY".to_owned()),
            Some("fixture-1".to_owned()),
        ));
        assert_eq!(mixed.amount(), None);

        let mut mixed_version = CostAccumulator::default();
        mixed_version.add(&CostEstimate::available(
            0.1,
            Some("USD".to_owned()),
            Some("fixture-1".to_owned()),
        ));
        mixed_version.add(&CostEstimate::available(
            0.2,
            Some("USD".to_owned()),
            Some("fixture-2".to_owned()),
        ));
        assert_eq!(mixed_version.amount(), None);

        let mut missing_amount = CostAccumulator::default();
        missing_amount.add(&CostEstimate {
            amount: None,
            currency: Some("USD".to_owned()),
            price_version: Some("fixture-1".to_owned()),
            status: CostStatus::Available,
            basis: CostBasis::Actual,
        });
        assert_eq!(missing_amount.amount(), None);

        let mut negative = CostAccumulator::default();
        negative.add(&CostEstimate {
            amount: Some(-0.1),
            currency: Some("USD".to_owned()),
            price_version: Some("fixture-1".to_owned()),
            status: CostStatus::Available,
            basis: CostBasis::Actual,
        });
        assert_eq!(negative.amount(), None);

        let mut metadata_missing = CostAccumulator::default();
        metadata_missing.add(&CostEstimate::actual(0.1, None, None));
        assert_eq!(metadata_missing.amount(), None);

        let mut zero = CostAccumulator::default();
        zero.add(&CostSummary::no_external_call().total);
        assert_eq!(zero.amount(), Some(0.0));

        let mut mixed_basis = CostAccumulator::default();
        mixed_basis.add(&CostEstimate::available(
            0.1,
            Some("USD".to_owned()),
            Some("fixture-1".to_owned()),
        ));
        mixed_basis.add(&CostEstimate::actual(
            0.2,
            Some("USD".to_owned()),
            Some("fixture-1".to_owned()),
        ));
        assert_eq!(mixed_basis.amount(), None);

        let partial = TokenPricing {
            input_per_million: Some(1.0),
            output_per_million: Some(2.0),
            reasoning_per_million: None,
            currency: Some("USD".to_owned()),
            price_version: Some("fixture-1".to_owned()),
        };
        assert_eq!(
            partial
                .estimate_three_part(Some(1), Some(1), Some(1))
                .status,
            CostStatus::Unknown
        );
    }
}

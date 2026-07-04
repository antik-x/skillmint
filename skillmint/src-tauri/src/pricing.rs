//! PRD-08: reference pricing engine — a 1:1 Rust port of AI-Digest's
//! `src/digest/pricing.py`.
//!
//! Prices are *reference list prices* (CNY per 1M tokens) for cross-platform
//! comparison only. Real billing differs (some models are subscription, some
//! pay-as-you-go). Every cost derived here must be labelled "参考估算".
//!
//! The pricing table is data, not code: the [`MODEL_PRICING`] constant is the
//! always-available fallback and the migration seed (db.rs copies it into
//! `digest_model` on first run). At runtime [`Pricing`] is built from the DB so
//! editing `digest_model` updates costs without a rebuild. When the DB is empty
//! or a model is unknown, resolution falls back to the constant, then to
//! [`DEFAULT_MODEL_PRICING`]. The engine **never fails to resolve** — a missing
//! price degrades to the default, never panics.

use crate::models::UsageSample;
use crate::settings::BillingMode;

/// (billing_mode, input_price, cache_read_price, output_price).
pub type PricingTuple = (BillingMode, f64, f64, f64);

/// Fallback for any model not found anywhere. Matches AI-Digest.
pub const DEFAULT_MODEL_PRICING: PricingTuple = (BillingMode::PayAsYouGo, 4.0, 1.0, 16.0);

/// The builtin seed table. Tuple: `(billing_mode, input, cache_read, output)`.
/// Kept byte-for-byte aligned with AI-Digest `MODEL_PRICING` so costs are
/// caliber-comparable across the two tools. This is also the constant the DB
/// migration seeds from (db.rs::migrate_digest_dimensions).
///
/// NOTE: iterate with `pricing::MODEL_PRICING.iter()` — it is a slice of
/// 5-tuples to satisfy the const evaluator (tuples of references).
pub const MODEL_PRICING: &[(&str, BillingMode, f64, f64, f64)] = &[
    // DeepSeek (pay-as-you-go)
    ("deepseek-chat", BillingMode::PayAsYouGo, 2.0, 0.5, 8.0),
    ("deepseek-reasoner", BillingMode::PayAsYouGo, 4.0, 1.0, 16.0),
    ("DeepSeek-V3", BillingMode::PayAsYouGo, 2.0, 0.5, 8.0),
    ("DeepSeek-R1", BillingMode::PayAsYouGo, 4.0, 1.0, 16.0),
    // GLM (commonly subscription; reference per-token prices shown)
    ("GLM-4.5", BillingMode::Subscription, 2.0, 0.5, 8.0),
    ("GLM-5.1", BillingMode::Subscription, 2.0, 0.5, 8.0),
    ("GLM-5.2", BillingMode::Subscription, 2.0, 0.5, 8.0),
    // Anthropic Claude (pay-as-you-go list prices, USD→CNY ~7.2)
    ("claude-3-7-sonnet-latest", BillingMode::PayAsYouGo, 21.6, 2.16, 108.0),
    ("claude-sonnet-4-5", BillingMode::PayAsYouGo, 21.6, 2.16, 108.0),
    // OpenAI (pay-as-you-go list prices)
    ("gpt-4o-mini", BillingMode::PayAsYouGo, 1.08, 0.27, 4.32),
    ("gpt-4o", BillingMode::PayAsYouGo, 17.28, 4.32, 69.12),
    ("gpt-5", BillingMode::PayAsYouGo, 8.64, 2.16, 34.56),
    ("o1", BillingMode::PayAsYouGo, 108.0, 0.0, 432.0),
    // Kimi / Moonshot (commonly subscription)
    ("moonshot-v1-128k", BillingMode::Subscription, 60.0, 60.0, 60.0),
];

/// One row of `digest_model`, as loaded from the DB. `model_id` is matched
/// case-insensitively (and fuzzily) so variant suffixes like
/// `builtin:bigmodel-coding-plan/GLM-5.2` still resolve.
#[derive(Debug, Clone)]
pub struct ModelPricingRow {
    pub model_id: String,
    pub input_price: f64,
    pub cache_price: f64,
    pub output_price: f64,
    pub billing_mode: BillingMode,
}

/// Resolves (billing_mode, input, cache_read, output) for a model id, with
/// fuzzy substring matching. This is the runtime port of
/// `pricing.lookup_pricing`. Built from DB rows when available; the constant
/// table is always consulted as a second-tier fallback.
#[derive(Debug, Clone, Default)]
pub struct Pricing {
    /// Lowercased model_id -> resolved tuple. Source of truth at runtime.
    db_rows: Vec<(String, BillingMode, f64, f64, f64)>,
}

impl Pricing {
    /// Build from DB rows. An empty slice yields a resolver that still works —
    /// it falls through to the constant table. Never errors.
    pub fn from_db(rows: Vec<ModelPricingRow>) -> Self {
        let db_rows = rows
            .into_iter()
            .map(|r| {
                (
                    r.model_id.to_lowercase(),
                    r.billing_mode,
                    r.input_price,
                    r.cache_price,
                    r.output_price,
                )
            })
            .collect();
        Self { db_rows }
    }

    /// Resolve pricing for one model id. Matching order (mirrors AI-Digest):
    /// 1. exact case-insensitive DB hit
    /// 2. fuzzy DB substring match
    /// 3. exact/fuzzy constant-table match
    /// 4. DEFAULT_MODEL_PRICING
    pub fn lookup(&self, model_id: &str) -> (BillingMode, f64, f64, f64) {
        if model_id.is_empty() {
            return DEFAULT_MODEL_PRICING;
        }
        let lower = model_id.to_lowercase();

        // 1. exact DB hit
        for (key, mode, inp, cache, out) in &self.db_rows {
            if key == &lower {
                return (*mode, *inp, *cache, *out);
            }
        }
        // 2. fuzzy DB substring match
        for (key, mode, inp, cache, out) in &self.db_rows {
            if !key.is_empty() && lower.contains(key.as_str()) {
                return (*mode, *inp, *cache, *out);
            }
        }
        // 3. constant table (exact then fuzzy)
        for (id, mode, inp, cache, out) in MODEL_PRICING {
            if id.to_lowercase() == lower {
                return (*mode, *inp, *cache, *out);
            }
        }
        for (id, mode, inp, cache, out) in MODEL_PRICING {
            if lower.contains(&id.to_lowercase()) {
                return (*mode, *inp, *cache, *out);
            }
        }
        // 4. default
        DEFAULT_MODEL_PRICING
    }

    /// Just the billing mode for a model id.
    pub fn billing_mode_for(&self, model_id: &str) -> BillingMode {
        self.lookup(model_id).0
    }

    /// Estimate reference cost in CNY for one usage sample. Mirrors
    /// `pricing.estimate_cost`: cached reads use the low cache rate; cache
    /// creation is charged at the input rate; reasoning is charged at output.
    pub fn estimate_cost(&self, u: &UsageSample) -> f64 {
        let model = u.model_id.clone().unwrap_or_default();
        let (_, in_price, cache_price, out_price) = self.lookup(&model);
        let fresh_input = (u.input_tokens
            - u.cache_creation_input_tokens
            - u.cache_read_input_tokens)
        .max(0) as f64;
        let cost = ((fresh_input + u.cache_creation_input_tokens as f64) * in_price
            + u.cache_read_input_tokens as f64 * cache_price
            + u.output_tokens as f64 * out_price
            + u.reasoning_tokens as f64 * out_price)
            / 1_000_000.0;
        (cost * 1e4).round() / 1e4 // round to 4 decimals, matching Python round(cost, 4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::UsageSample;

    fn sample(model: &str, input: i64, cache_read: i64, cache_create: i64, output: i64, reasoning: i64) -> UsageSample {
        UsageSample {
            session_id: "s".into(),
            source: "zcode".into(),
            project_path: None,
            model_id: Some(model.into()),
            input_tokens: input,
            output_tokens: output,
            reasoning_tokens: reasoning,
            cache_creation_input_tokens: cache_create,
            cache_read_input_tokens: cache_read,
            total_tokens: input + output + reasoning + cache_read + cache_create,
            model_calls: 1,
            tool_calls: 0,
            duration_ms: None,
        }
    }

    #[test]
    fn resolves_exact_and_fuzzy() {
        use crate::settings::BillingMode;
        let p = Pricing::from_db(vec![]);
        // exact
        let m = p.lookup("GLM-5.2");
        assert_eq!(m.0, BillingMode::Subscription);
        // fuzzy: variant suffix still resolves to the same entry
        let m2 = p.lookup("builtin:bigmodel-coding-plan/GLM-5.2");
        assert_eq!(m2.0, BillingMode::Subscription);
        // unknown -> default
        let m3 = p.lookup("some-unknown-model");
        assert_eq!(m3.0, BillingMode::PayAsYouGo);
    }

    #[test]
    fn estimate_cost_matches_python_formula() {
        let p = Pricing::from_db(vec![]);
        // deepseek-reasoner: in=4, cache=1, out=16 (pay_as_you_go)
        // input=1000000, cache_read=0, cache_create=0, output=0, reasoning=0
        // -> fresh_input=1e6, cost = 1e6*4/1e6 = 4.0
        let u = sample("deepseek-reasoner", 1_000_000, 0, 0, 0, 0);
        assert!((p.estimate_cost(&u) - 4.0).abs() < 1e-6);
        // cache read priced at the cache rate (1.0 for deepseek-reasoner): 1e6 * 1.0/1e6 = 1.0
        let u2 = sample("deepseek-reasoner", 0, 1_000_000, 0, 0, 0);
        assert!((p.estimate_cost(&u2) - 1.0).abs() < 1e-6);
        // reasoning priced at output rate (16.0)
        let u3 = sample("deepseek-reasoner", 0, 0, 0, 0, 1_000_000);
        assert!((p.estimate_cost(&u3) - 16.0).abs() < 1e-6);
    }
}

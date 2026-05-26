//! Token usage and optional cost aggregation.
//!
//! The agent records provider-reported usage after each model call. This module
//! merges those entries into [`UsageSummary`], optionally loading LiteLLM-style
//! pricing data so callers can inspect approximate per-model cost.

use crate::{AgentSettings, ModelUsageStats, UsageSummary};
use browser_use_llm::{ChatCompletion, ChatUsage};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

const DEFAULT_MODEL_PRICING_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
const DEFAULT_OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";

#[derive(Debug, Clone)]
struct TokenUsageEntry {
    model: String,
    pricing_model: String,
    usage: ChatUsage,
}

#[derive(Debug, Clone, Deserialize)]
struct ModelPricing {
    input_cost_per_token: Option<f64>,
    output_cost_per_token: Option<f64>,
    cache_read_input_token_cost: Option<f64>,
    cache_creation_input_token_cost: Option<f64>,
}

pub(crate) struct TokenUsageTracker {
    include_cost: bool,
    pricing_url: String,
    openrouter_models_url: String,
    base_summary: Option<UsageSummary>,
    entries: Vec<TokenUsageEntry>,
    pricing_data: Option<BTreeMap<String, ModelPricing>>,
    pricing_loaded: bool,
    openrouter_pricing_data: Option<BTreeMap<String, ModelPricing>>,
    openrouter_pricing_loaded: bool,
    client: reqwest::Client,
}

#[derive(Debug, Clone, Copy)]
struct UsageCost {
    prompt_cost: f64,
    prompt_cached_cost: f64,
    completion_cost: f64,
    total_cost: f64,
}

impl TokenUsageTracker {
    pub(crate) fn for_settings(settings: &AgentSettings) -> Self {
        let include_cost = settings.calculate_cost
            || std::env::var("BROWSER_USE_CALCULATE_COST")
                .is_ok_and(|value| value.eq_ignore_ascii_case("true"));
        let pricing_url = std::env::var("BROWSER_USE_MODEL_PRICING_URL")
            .unwrap_or_else(|_| DEFAULT_MODEL_PRICING_URL.to_owned());
        let openrouter_models_url = std::env::var("BROWSER_USE_OPENROUTER_MODELS_URL")
            .unwrap_or_else(|_| DEFAULT_OPENROUTER_MODELS_URL.to_owned());
        Self {
            include_cost,
            pricing_url,
            openrouter_models_url,
            base_summary: None,
            entries: Vec::new(),
            pricing_data: None,
            pricing_loaded: false,
            openrouter_pricing_data: None,
            openrouter_pricing_loaded: false,
            client: reqwest::Client::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_test_pricing_urls(
        mut self,
        pricing_url: String,
        openrouter_models_url: String,
    ) -> Self {
        self.pricing_url = pricing_url;
        self.openrouter_models_url = openrouter_models_url;
        self
    }

    pub(crate) fn with_base_summary(mut self, base_summary: Option<UsageSummary>) -> Self {
        self.base_summary = base_summary;
        self
    }

    pub(crate) fn add_completion_with_provider(
        &mut self,
        provider: &str,
        completion: &ChatCompletion<Value>,
    ) {
        let Some(usage) = completion.usage.clone() else {
            return;
        };
        self.entries.push(TokenUsageEntry {
            model: completion.model.clone(),
            pricing_model: pricing_model_name(provider, &completion.model),
            usage,
        });
    }

    pub(crate) async fn summary(&mut self) -> UsageSummary {
        self.ensure_pricing_loaded().await;
        self.ensure_openrouter_pricing_loaded().await;
        let mut summary = self.base_summary.clone().unwrap_or_default();

        for entry in &self.entries {
            add_usage_to_summary(
                &mut summary,
                entry,
                self.pricing_for_model(&entry.pricing_model)
                    .map(|pricing| calculate_usage_cost(&entry.usage, pricing)),
            );
        }

        refresh_usage_averages(&mut summary);
        summary
    }

    async fn ensure_pricing_loaded(&mut self) {
        if !self.include_cost || self.pricing_loaded {
            return;
        }
        self.pricing_loaded = true;
        let mut pricing = custom_model_pricing();
        if let Ok(response) = self.client.get(&self.pricing_url).send().await {
            if let Ok(value) = response.json::<Value>().await {
                merge_litellm_pricing(&mut pricing, value);
            }
        }
        self.pricing_data = Some(pricing);
    }

    async fn ensure_openrouter_pricing_loaded(&mut self) {
        if !self.include_cost
            || self.openrouter_pricing_loaded
            || !self
                .entries
                .iter()
                .any(|entry| is_openrouter_pricing_model(&entry.pricing_model))
        {
            return;
        }
        self.openrouter_pricing_loaded = true;
        if let Ok(response) = self.client.get(&self.openrouter_models_url).send().await {
            if let Ok(value) = response.json::<Value>().await {
                self.openrouter_pricing_data = Some(openrouter_pricing_from_models_response(value));
            }
        }
    }

    fn pricing_for_model(&self, model: &str) -> Option<&ModelPricing> {
        if !self.include_cost {
            return None;
        }
        if let Some(pricing) = self.openrouter_pricing_for_model(model) {
            return Some(pricing);
        }
        let pricing = self.pricing_data.as_ref()?;
        pricing
            .get(model)
            .or_else(|| litellm_model_alias(model).and_then(|alias| pricing.get(alias)))
    }

    fn openrouter_pricing_for_model(&self, model: &str) -> Option<&ModelPricing> {
        if !is_openrouter_pricing_model(model) {
            return None;
        }
        let model_id = normalize_openrouter_model_id(model)?;
        self.openrouter_pricing_data.as_ref()?.get(model_id)
    }
}

fn pricing_model_name(provider: &str, model: &str) -> String {
    if provider.eq_ignore_ascii_case("openrouter") && !is_openrouter_pricing_model(model) {
        format!("openrouter/{model}")
    } else {
        model.to_owned()
    }
}

fn custom_model_pricing() -> BTreeMap<String, ModelPricing> {
    let bu_1_0 = ModelPricing {
        input_cost_per_token: Some(0.2 / 1_000_000.0),
        output_cost_per_token: Some(2.0 / 1_000_000.0),
        cache_read_input_token_cost: Some(0.02 / 1_000_000.0),
        cache_creation_input_token_cost: None,
    };
    let bu_2_0 = ModelPricing {
        input_cost_per_token: Some(0.60 / 1_000_000.0),
        output_cost_per_token: Some(3.50 / 1_000_000.0),
        cache_read_input_token_cost: Some(0.06 / 1_000_000.0),
        cache_creation_input_token_cost: None,
    };

    [
        ("bu-1-0".to_owned(), bu_1_0.clone()),
        ("bu-latest".to_owned(), bu_2_0.clone()),
        ("smart".to_owned(), bu_2_0.clone()),
        ("bu-2-0".to_owned(), bu_2_0),
    ]
    .into_iter()
    .collect()
}

fn merge_litellm_pricing(pricing: &mut BTreeMap<String, ModelPricing>, value: Value) {
    let Value::Object(map) = value else {
        return;
    };
    for (model, value) in map {
        let Ok(model_pricing) = serde_json::from_value::<ModelPricing>(value) else {
            continue;
        };
        pricing.insert(model, model_pricing);
    }
}

fn litellm_model_alias(model: &str) -> Option<&'static str> {
    match model {
        "gemini-flash-latest" => Some("gemini/gemini-flash-latest"),
        "gemini-3-flash-preview" => Some("gemini/gemini-3-flash-preview"),
        "gemini-3.1-flash-lite" => Some("gemini/gemini-3.1-flash-lite-preview"),
        _ => None,
    }
}

fn is_openrouter_pricing_model(model: &str) -> bool {
    model.starts_with("openrouter/") || model.starts_with("openrouter-")
}

fn normalize_openrouter_model_id(model: &str) -> Option<&str> {
    let model = model
        .strip_prefix("openrouter/")
        .or_else(|| model.strip_prefix("openrouter-"))
        .unwrap_or(model);
    model.contains('/').then_some(model)
}

fn openrouter_pricing_from_models_response(value: Value) -> BTreeMap<String, ModelPricing> {
    let Some(models) = value.get("data").and_then(Value::as_array) else {
        return BTreeMap::new();
    };

    let mut pricing = BTreeMap::new();
    for model in models {
        let Some(model_id) = model.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(openrouter_pricing) = model.get("pricing").and_then(Value::as_object) else {
            continue;
        };
        let input_cost_per_token = pricing_number(openrouter_pricing.get("prompt"));
        let output_cost_per_token = pricing_number(openrouter_pricing.get("completion"));
        if input_cost_per_token.is_none() && output_cost_per_token.is_none() {
            continue;
        }
        pricing.insert(
            model_id.to_owned(),
            ModelPricing {
                input_cost_per_token,
                output_cost_per_token,
                cache_read_input_token_cost: pricing_number(
                    openrouter_pricing.get("input_cache_read"),
                ),
                cache_creation_input_token_cost: pricing_number(
                    openrouter_pricing.get("input_cache_write"),
                ),
            },
        );
    }
    pricing
}

fn pricing_number(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) if !text.trim().is_empty() => text.parse::<f64>().ok(),
        _ => None,
    }
}

fn add_usage_to_summary(
    summary: &mut UsageSummary,
    entry: &TokenUsageEntry,
    cost: Option<UsageCost>,
) {
    let usage = &entry.usage;
    summary.total_prompt_tokens += usage.prompt_tokens;
    summary.total_prompt_cached_tokens += usage.prompt_cached_tokens.unwrap_or(0);
    summary.total_completion_tokens += usage.completion_tokens;
    summary.total_tokens += usage.prompt_tokens + usage.completion_tokens;
    summary.entry_count += 1;

    let stats = summary
        .by_model
        .entry(entry.model.clone())
        .or_insert_with(|| ModelUsageStats {
            model: entry.model.clone(),
            ..ModelUsageStats::default()
        });
    stats.prompt_tokens += usage.prompt_tokens;
    stats.completion_tokens += usage.completion_tokens;
    stats.total_tokens += usage.prompt_tokens + usage.completion_tokens;
    stats.invocations += 1;

    if let Some(cost) = cost {
        summary.total_prompt_cost += cost.prompt_cost;
        summary.total_prompt_cached_cost += cost.prompt_cached_cost;
        summary.total_completion_cost += cost.completion_cost;
        summary.total_cost += cost.total_cost + cost.prompt_cached_cost;
        stats.cost += cost.total_cost;
    }
}

fn refresh_usage_averages(summary: &mut UsageSummary) {
    for stats in summary.by_model.values_mut() {
        if stats.invocations > 0 {
            stats.average_tokens_per_invocation =
                stats.total_tokens as f64 / stats.invocations as f64;
        }
    }
}

fn calculate_usage_cost(usage: &ChatUsage, pricing: &ModelPricing) -> UsageCost {
    let cached_tokens = usage.prompt_cached_tokens.unwrap_or(0);
    let uncached_prompt_tokens = usage.prompt_tokens.saturating_sub(cached_tokens);
    let prompt_new_cost =
        uncached_prompt_tokens as f64 * pricing.input_cost_per_token.unwrap_or(0.0);
    let prompt_cached_cost =
        cached_tokens as f64 * pricing.cache_read_input_token_cost.unwrap_or(0.0);
    let prompt_cache_creation_cost = usage.prompt_cache_creation_tokens.unwrap_or(0) as f64
        * pricing.cache_creation_input_token_cost.unwrap_or(0.0);
    let prompt_cost = prompt_new_cost + prompt_cached_cost + prompt_cache_creation_cost;
    let completion_cost =
        usage.completion_tokens as f64 * pricing.output_cost_per_token.unwrap_or(0.0);

    UsageCost {
        prompt_cost,
        prompt_cached_cost,
        completion_cost,
        total_cost: prompt_cost + completion_cost,
    }
}

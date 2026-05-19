use crate::{AgentSettings, ModelUsageStats, UsageSummary};
use browser_use_llm::{ChatCompletion, ChatUsage};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

const DEFAULT_MODEL_PRICING_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";

#[derive(Debug, Clone)]
struct TokenUsageEntry {
    model: String,
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
    base_summary: Option<UsageSummary>,
    entries: Vec<TokenUsageEntry>,
    pricing_data: Option<BTreeMap<String, ModelPricing>>,
    pricing_loaded: bool,
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
        Self {
            include_cost,
            pricing_url,
            base_summary: None,
            entries: Vec::new(),
            pricing_data: None,
            pricing_loaded: false,
            client: reqwest::Client::new(),
        }
    }

    pub(crate) fn with_base_summary(mut self, base_summary: Option<UsageSummary>) -> Self {
        self.base_summary = base_summary;
        self
    }

    pub(crate) fn add_completion(&mut self, completion: &ChatCompletion<Value>) {
        let Some(usage) = completion.usage.clone() else {
            return;
        };
        self.entries.push(TokenUsageEntry {
            model: completion.model.clone(),
            usage,
        });
    }

    pub(crate) async fn summary(&mut self) -> UsageSummary {
        self.ensure_pricing_loaded().await;
        let mut summary = self.base_summary.clone().unwrap_or_default();

        for entry in &self.entries {
            add_usage_to_summary(
                &mut summary,
                entry,
                self.pricing_for_model(&entry.model)
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

    fn pricing_for_model(&self, model: &str) -> Option<&ModelPricing> {
        if !self.include_cost {
            return None;
        }
        let pricing = self.pricing_data.as_ref()?;
        pricing
            .get(model)
            .or_else(|| litellm_model_alias(model).and_then(|alias| pricing.get(alias)))
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
        ("bu-latest".to_owned(), bu_1_0.clone()),
        ("smart".to_owned(), bu_1_0),
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

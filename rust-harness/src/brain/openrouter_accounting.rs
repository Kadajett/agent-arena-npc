use std::sync::Arc;

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::observability::{AnalyticsEvent, AnalyticsSink, EventLevel};

const OPENROUTER_API_BASE: &str = "https://openrouter.ai/api/v1";

/// Reads `OpenRouter`'s accounting APIs without placing billing work on a brain's hot path.
#[derive(Clone)]
pub struct OpenRouterAccountingClient {
    http: Client,
    api_key: Arc<str>,
}

impl OpenRouterAccountingClient {
    #[must_use]
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            http: Client::new(),
            api_key: Arc::from(api_key.into()),
        }
    }

    /// Fetch the exact provider endpoint prices currently advertised for a model.
    ///
    /// # Errors
    /// Returns an error when the request, status, or response schema is invalid.
    pub async fn model_endpoints(&self, model_id: &str) -> anyhow::Result<ModelEndpoints> {
        let response = self
            .http
            .get(format!("{OPENROUTER_API_BASE}/models/{model_id}/endpoints"))
            .bearer_auth(self.api_key.as_ref())
            .send()
            .await?
            .error_for_status()?
            .json::<ModelEndpointsEnvelope>()
            .await?;
        Ok(response.data)
    }

    /// Fetch `OpenRouter`'s finalized accounting record for one generation.
    ///
    /// # Errors
    /// Returns an error when the request, status, or response schema is invalid.
    pub async fn generation(&self, generation_id: &str) -> anyhow::Result<GenerationRecord> {
        let response = self
            .http
            .get(format!("{OPENROUTER_API_BASE}/generation"))
            .bearer_auth(self.api_key.as_ref())
            .query(&[("id", generation_id)])
            .send()
            .await?
            .error_for_status()?
            .json::<GenerationEnvelope>()
            .await?;
        Ok(response.data)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ModelEndpointsEnvelope {
    data: ModelEndpoints,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelEndpoints {
    pub id: String,
    #[serde(default)]
    pub endpoints: Vec<ModelEndpoint>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelEndpoint {
    pub name: String,
    pub provider_name: String,
    pub model_id: Option<String>,
    pub quantization: Option<String>,
    pub status: Option<i64>,
    pub context_length: Option<u64>,
    pub max_completion_tokens: Option<u64>,
    pub max_prompt_tokens: Option<u64>,
    pub supports_implicit_caching: Option<bool>,
    pub latency_last_30m: Option<ServicePercentiles>,
    pub throughput_last_30m: Option<ServicePercentiles>,
    pub uptime_last_30m: Option<f64>,
    pub uptime_last_5m: Option<f64>,
    pub uptime_last_1d: Option<f64>,
    #[serde(default)]
    pub supported_parameters: Vec<String>,
    #[serde(default)]
    pub pricing: EndpointPricing,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServicePercentiles {
    pub p50: Option<f64>,
    pub p75: Option<f64>,
    pub p90: Option<f64>,
    pub p99: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct EndpointPricing {
    pub prompt: Option<Value>,
    pub completion: Option<Value>,
    pub request: Option<Value>,
    pub image: Option<Value>,
    pub web_search: Option<Value>,
    pub internal_reasoning: Option<Value>,
    pub input_cache_read: Option<Value>,
    pub input_cache_write: Option<Value>,
    pub discount: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct GenerationEnvelope {
    data: GenerationRecord,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GenerationRecord {
    pub id: Option<String>,
    pub model: Option<String>,
    pub provider_name: Option<String>,
    pub service_tier: Option<String>,
    pub is_byok: Option<bool>,
    pub streamed: Option<bool>,
    pub latency: Option<u64>,
    pub generation_time: Option<u64>,
    pub tokens_prompt: Option<u64>,
    pub tokens_completion: Option<u64>,
    pub native_tokens_prompt: Option<u64>,
    pub native_tokens_completion: Option<u64>,
    pub native_tokens_reasoning: Option<u64>,
    pub native_tokens_cached: Option<u64>,
    pub num_media_prompt: Option<u64>,
    pub num_media_completion: Option<u64>,
    pub num_search_results: Option<u64>,
    pub total_cost: Option<f64>,
    pub upstream_inference_cost: Option<f64>,
    pub cache_discount: Option<f64>,
    pub created_at: Option<DateTime<Utc>>,
}

pub fn record_price_snapshot(
    analytics: &Arc<dyn AnalyticsSink>,
    character_id: Option<&str>,
    correlation_id: Uuid,
    requested_model: &str,
    result: &anyhow::Result<ModelEndpoints>,
) {
    let Ok(model) = result else {
        let event = AnalyticsEvent::new("model.price_snapshot_failed", EventLevel::Warn)
            .correlation(correlation_id)
            .attribute("requested_model", requested_model.to_owned())
            .attribute("error_class", "openrouter_accounting_api");
        record_for_character(analytics, event, character_id);
        return;
    };
    let snapshot_id = Uuid::new_v4();
    for endpoint in &model.endpoints {
        let mut event = AnalyticsEvent::new("model.price_snapshot", EventLevel::Info)
            .correlation(correlation_id)
            .attribute("snapshot_id", snapshot_id.to_string())
            .attribute("observed_at", Utc::now().to_rfc3339())
            .attribute("requested_model", requested_model.to_owned())
            .attribute("priced_model", model.id.clone())
            .attribute("provider", endpoint.provider_name.clone())
            .attribute("endpoint", endpoint.name.clone())
            .attribute("model_id", endpoint.model_id.clone().unwrap_or_default())
            .attribute(
                "quantization",
                endpoint.quantization.clone().unwrap_or_default(),
            )
            .attribute("status_known", endpoint.status.is_some())
            .attribute("status", endpoint.status.unwrap_or_default())
            .attribute(
                "supports_implicit_caching",
                endpoint.supports_implicit_caching.unwrap_or(false),
            )
            .attribute(
                "supported_parameter_count",
                u64::try_from(endpoint.supported_parameters.len()).unwrap_or(u64::MAX),
            );
        event = add_optional_u64(event, "context_length", endpoint.context_length);
        event = add_optional_u64(
            event,
            "max_completion_tokens",
            endpoint.max_completion_tokens,
        );
        event = add_optional_u64(event, "max_prompt_tokens", endpoint.max_prompt_tokens);
        event = add_percentiles(
            event,
            "latency_last_30m",
            endpoint.latency_last_30m.as_ref(),
        );
        event = add_percentiles(
            event,
            "throughput_last_30m",
            endpoint.throughput_last_30m.as_ref(),
        );
        event = add_optional_f64(event, "uptime_last_30m", endpoint.uptime_last_30m);
        event = add_optional_f64(event, "uptime_last_5m", endpoint.uptime_last_5m);
        event = add_optional_f64(event, "uptime_last_1d", endpoint.uptime_last_1d);
        event = add_price(
            event,
            "prompt_usd_per_token",
            endpoint.pricing.prompt.as_ref(),
        );
        event = add_price(
            event,
            "completion_usd_per_token",
            endpoint.pricing.completion.as_ref(),
        );
        event = add_price(event, "request_usd", endpoint.pricing.request.as_ref());
        event = add_price(event, "image_usd", endpoint.pricing.image.as_ref());
        event = add_price(
            event,
            "web_search_usd",
            endpoint.pricing.web_search.as_ref(),
        );
        event = add_price(
            event,
            "internal_reasoning_usd_per_token",
            endpoint.pricing.internal_reasoning.as_ref(),
        );
        event = add_price(
            event,
            "input_cache_read_usd_per_token",
            endpoint.pricing.input_cache_read.as_ref(),
        );
        event = add_price(
            event,
            "input_cache_write_usd_per_token",
            endpoint.pricing.input_cache_write.as_ref(),
        );
        event = add_price(event, "discount", endpoint.pricing.discount.as_ref());
        record_for_character(analytics, event, character_id);
    }
    let event = AnalyticsEvent::new("model.price_snapshot_completed", EventLevel::Info)
        .correlation(correlation_id)
        .attribute("snapshot_id", snapshot_id.to_string())
        .attribute("requested_model", requested_model.to_owned())
        .attribute(
            "endpoint_count",
            u64::try_from(model.endpoints.len()).unwrap_or(u64::MAX),
        );
    record_for_character(analytics, event, character_id);
}

pub fn record_generation(
    analytics: &Arc<dyn AnalyticsSink>,
    character_id: Option<&str>,
    correlation_id: Uuid,
    generation_id: &str,
    attempts: u8,
    accounting_latency_ms: u64,
    result: &anyhow::Result<GenerationRecord>,
) {
    let mut event = match result {
        Ok(record) => {
            let mut event = AnalyticsEvent::new("model.generation_accounted", EventLevel::Info)
                .correlation(correlation_id)
                .attribute("generation_id", generation_id.to_owned())
                .attribute("accounting_attempts", attempts)
                .attribute("accounting_latency_ms", accounting_latency_ms)
                .attribute("record_id", record.id.clone().unwrap_or_default())
                .attribute("model", record.model.clone().unwrap_or_default())
                .attribute(
                    "actual_provider",
                    record.provider_name.clone().unwrap_or_default(),
                )
                .attribute(
                    "service_tier",
                    record.service_tier.clone().unwrap_or_default(),
                )
                .attribute("is_byok", record.is_byok.unwrap_or(false))
                .attribute("streamed", record.streamed.unwrap_or(false))
                .attribute("upstream_cost_applicable", record.is_byok.unwrap_or(false));
            event = add_optional_u64(event, "latency_ms", record.latency);
            event = add_optional_u64(event, "generation_time_ms", record.generation_time);
            event = add_optional_u64(event, "tokens_prompt", record.tokens_prompt);
            event = add_optional_u64(event, "tokens_completion", record.tokens_completion);
            event = add_optional_u64(event, "native_tokens_prompt", record.native_tokens_prompt);
            event = add_optional_u64(
                event,
                "native_tokens_completion",
                record.native_tokens_completion,
            );
            event = add_optional_u64(
                event,
                "native_tokens_reasoning",
                record.native_tokens_reasoning,
            );
            event = add_optional_u64(event, "native_tokens_cached", record.native_tokens_cached);
            event = add_optional_u64(event, "media_prompt_count", record.num_media_prompt);
            event = add_optional_u64(event, "media_completion_count", record.num_media_completion);
            event = add_optional_u64(event, "search_result_count", record.num_search_results);
            event = add_optional_f64(event, "openrouter_cost_usd", record.total_cost);
            event = add_optional_f64(
                event,
                "upstream_inference_cost_usd",
                record.upstream_inference_cost,
            );
            event = add_optional_f64(event, "cache_discount_usd", record.cache_discount);
            if let Some(created_at) = record.created_at {
                event = event.attribute("provider_created_at", created_at.to_rfc3339());
            }
            event
        }
        Err(_) => AnalyticsEvent::new("model.generation_accounting_failed", EventLevel::Warn)
            .correlation(correlation_id)
            .attribute("generation_id", generation_id.to_owned())
            .attribute("accounting_attempts", attempts)
            .attribute("accounting_latency_ms", accounting_latency_ms)
            .attribute("error_class", "openrouter_accounting_api"),
    };
    if let Some(character_id) = character_id {
        event = event.character(character_id);
    }
    analytics.record(event);
}

fn add_price(mut event: AnalyticsEvent, name: &str, value: Option<&Value>) -> AnalyticsEvent {
    let Some(value) = value else {
        return event.attribute(format!("{name}_known"), false);
    };
    let exact = match value {
        Value::String(value) => value.clone(),
        other => other.to_string(),
    };
    event = event
        .attribute(format!("{name}_known"), true)
        .attribute(format!("{name}_exact"), exact.clone());
    exact
        .parse::<f64>()
        .ok()
        .map_or(event.clone(), |numeric| event.attribute(name, numeric))
}

fn record_for_character(
    analytics: &Arc<dyn AnalyticsSink>,
    mut event: AnalyticsEvent,
    character_id: Option<&str>,
) {
    if let Some(character_id) = character_id {
        event = event.character(character_id);
    }
    analytics.record(event);
}

fn add_optional_u64(event: AnalyticsEvent, name: &str, value: Option<u64>) -> AnalyticsEvent {
    event
        .attribute(format!("{name}_known"), value.is_some())
        .attribute(name, value.unwrap_or_default())
}

fn add_optional_f64(event: AnalyticsEvent, name: &str, value: Option<f64>) -> AnalyticsEvent {
    event
        .attribute(format!("{name}_known"), value.is_some())
        .attribute(name, value.unwrap_or_default())
}

fn add_percentiles(
    mut event: AnalyticsEvent,
    name: &str,
    value: Option<&ServicePercentiles>,
) -> AnalyticsEvent {
    event = event.attribute(format!("{name}_known"), value.is_some());
    let Some(value) = value else {
        return event;
    };
    event = add_optional_f64(event, &format!("{name}_p50"), value.p50);
    event = add_optional_f64(event, &format!("{name}_p75"), value.p75);
    event = add_optional_f64(event, &format!("{name}_p90"), value.p90);
    add_optional_f64(event, &format!("{name}_p99"), value.p99)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::RecordingAnalyticsSink;

    #[test]
    fn price_events_keep_exact_decimal_text_and_numeric_value() {
        let sink = Arc::new(RecordingAnalyticsSink::default());
        let analytics: Arc<dyn AnalyticsSink> = sink.clone();
        let result = Ok(ModelEndpoints {
            id: "author/model".to_owned(),
            endpoints: vec![ModelEndpoint {
                name: "provider/model".to_owned(),
                provider_name: "Provider".to_owned(),
                model_id: Some("author/model".to_owned()),
                quantization: Some("bf16".to_owned()),
                status: Some(0),
                context_length: Some(8_192),
                max_completion_tokens: None,
                max_prompt_tokens: None,
                supports_implicit_caching: Some(true),
                latency_last_30m: None,
                throughput_last_30m: None,
                uptime_last_30m: None,
                uptime_last_5m: None,
                uptime_last_1d: None,
                supported_parameters: vec!["temperature".to_owned()],
                pricing: EndpointPricing {
                    prompt: Some(Value::String("0.00000005".to_owned())),
                    ..EndpointPricing::default()
                },
            }],
        });

        record_price_snapshot(
            &analytics,
            Some("cassian"),
            Uuid::nil(),
            "author/model",
            &result,
        );

        let event = sink
            .events()
            .into_iter()
            .find(|event| event.name == "model.price_snapshot")
            .expect("price event");
        assert_eq!(event.character_id.as_deref(), Some("cassian"));
        assert_eq!(
            event.attributes["prompt_usd_per_token_exact"],
            Value::String("0.00000005".to_owned())
        );
        assert_eq!(
            event.attributes["prompt_usd_per_token"],
            Value::from(0.000_000_05)
        );
    }
}

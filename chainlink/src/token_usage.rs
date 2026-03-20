use serde::Deserialize;

/// Raw token usage as returned by the Claude API response metadata.
#[derive(Debug, Clone, Deserialize)]
pub struct RawTokenUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    #[serde(default)]
    pub cache_read_input_tokens: Option<i64>,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<i64>,
}

/// Parsed usage ready for database insertion.
#[derive(Debug, Clone)]
pub struct ParsedUsage {
    pub agent_id: String,
    pub session_id: Option<i64>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: Option<i64>,
    pub cache_creation_tokens: Option<i64>,
    pub model: String,
    pub cost_estimate: Option<f64>,
}

/// Per-million-token pricing for a model.
struct ModelPricing {
    input_per_mtok: f64,
    output_per_mtok: f64,
    cache_read_per_mtok: f64,
    cache_creation_per_mtok: f64,
}

/// Estimate cost in USD for a given model and token counts.
pub fn estimate_cost(
    model: &str,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: Option<i64>,
    cache_creation_tokens: Option<i64>,
) -> Option<f64> {
    let pricing = model_pricing(model)?;

    let input_cost = input_tokens as f64 * pricing.input_per_mtok / 1_000_000.0;
    let output_cost = output_tokens as f64 * pricing.output_per_mtok / 1_000_000.0;
    let cache_read_cost =
        cache_read_tokens.unwrap_or(0) as f64 * pricing.cache_read_per_mtok / 1_000_000.0;
    let cache_create_cost =
        cache_creation_tokens.unwrap_or(0) as f64 * pricing.cache_creation_per_mtok / 1_000_000.0;

    Some(input_cost + output_cost + cache_read_cost + cache_create_cost)
}

fn model_pricing(model: &str) -> Option<ModelPricing> {
    let m = model.to_lowercase();
    if m.contains("opus") {
        Some(ModelPricing {
            input_per_mtok: 15.0,
            output_per_mtok: 75.0,
            cache_read_per_mtok: 1.5,
            cache_creation_per_mtok: 18.75,
        })
    } else if m.contains("sonnet") {
        Some(ModelPricing {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
            cache_read_per_mtok: 0.3,
            cache_creation_per_mtok: 3.75,
        })
    } else if m.contains("haiku") {
        Some(ModelPricing {
            input_per_mtok: 0.80,
            output_per_mtok: 4.0,
            cache_read_per_mtok: 0.08,
            cache_creation_per_mtok: 1.0,
        })
    } else {
        None
    }
}

/// Parse raw API usage into a fully resolved ParsedUsage with cost estimate.
pub fn parse_api_usage(
    raw: &RawTokenUsage,
    model: &str,
    agent_id: &str,
    session_id: Option<i64>,
) -> ParsedUsage {
    let cost = estimate_cost(
        model,
        raw.input_tokens,
        raw.output_tokens,
        raw.cache_read_input_tokens,
        raw.cache_creation_input_tokens,
    );

    ParsedUsage {
        agent_id: agent_id.to_string(),
        session_id,
        input_tokens: raw.input_tokens,
        output_tokens: raw.output_tokens,
        cache_read_tokens: raw.cache_read_input_tokens,
        cache_creation_tokens: raw.cache_creation_input_tokens,
        model: model.to_string(),
        cost_estimate: cost,
    }
}

/// Aggregated usage summary grouped by agent and model.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UsageSummaryRow {
    pub agent_id: String,
    pub model: String,
    pub request_count: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cache_read_tokens: i64,
    pub total_cache_creation_tokens: i64,
    pub total_cost: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_cost_opus() {
        let cost = estimate_cost("claude-opus-4-6", 1_000_000, 1_000_000, None, None).unwrap();
        // $15 input + $75 output = $90
        assert!((cost - 90.0).abs() < 0.01);
    }

    #[test]
    fn test_estimate_cost_sonnet() {
        let cost = estimate_cost("claude-sonnet-4-6", 1_000_000, 1_000_000, None, None).unwrap();
        // $3 input + $15 output = $18
        assert!((cost - 18.0).abs() < 0.01);
    }

    #[test]
    fn test_estimate_cost_haiku() {
        let cost = estimate_cost("claude-haiku-4-5", 1_000_000, 1_000_000, None, None).unwrap();
        // $0.80 input + $4 output = $4.80
        assert!((cost - 4.80).abs() < 0.01);
    }

    #[test]
    fn test_estimate_cost_with_cache() {
        let cost = estimate_cost(
            "claude-opus-4-6",
            500_000,
            200_000,
            Some(300_000),
            Some(100_000),
        )
        .unwrap();
        let expected = 500_000.0 * 15.0 / 1_000_000.0
            + 200_000.0 * 75.0 / 1_000_000.0
            + 300_000.0 * 1.5 / 1_000_000.0
            + 100_000.0 * 18.75 / 1_000_000.0;
        assert!((cost - expected).abs() < 0.01);
    }

    #[test]
    fn test_estimate_cost_unknown_model() {
        assert!(estimate_cost("gpt-4", 1000, 1000, None, None).is_none());
    }

    #[test]
    fn test_parse_api_usage() {
        let raw = RawTokenUsage {
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_input_tokens: Some(200),
            cache_creation_input_tokens: None,
        };
        let parsed = parse_api_usage(&raw, "claude-sonnet-4-6", "worker-1", Some(42));
        assert_eq!(parsed.agent_id, "worker-1");
        assert_eq!(parsed.session_id, Some(42));
        assert_eq!(parsed.input_tokens, 1000);
        assert_eq!(parsed.output_tokens, 500);
        assert_eq!(parsed.cache_read_tokens, Some(200));
        assert!(parsed.cost_estimate.is_some());
    }

    #[test]
    fn test_raw_token_usage_deserialize() {
        let json = r#"{"input_tokens": 100, "output_tokens": 50}"#;
        let raw: RawTokenUsage = serde_json::from_str(json).unwrap();
        assert_eq!(raw.input_tokens, 100);
        assert_eq!(raw.output_tokens, 50);
        assert!(raw.cache_read_input_tokens.is_none());
    }

    #[test]
    fn test_raw_token_usage_with_cache_fields() {
        let json = r#"{
            "input_tokens": 100,
            "output_tokens": 50,
            "cache_read_input_tokens": 30,
            "cache_creation_input_tokens": 10
        }"#;
        let raw: RawTokenUsage = serde_json::from_str(json).unwrap();
        assert_eq!(raw.cache_read_input_tokens, Some(30));
        assert_eq!(raw.cache_creation_input_tokens, Some(10));
    }
}

//! HTTP adapters for checking API credentials without returning or logging them.
use serde_json::Value;

const MAX_BODY: usize = 1024 * 1024;

pub async fn models(provider: &str, key: &str) -> Result<Vec<String>, ()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ())?;
    let (url, header_name, header_value) = match provider {
        "openai" => (
            "https://api.openai.com/v1/models",
            "authorization",
            format!("Bearer {key}"),
        ),
        "openrouter" => (
            "https://openrouter.ai/api/v1/models",
            "authorization",
            format!("Bearer {key}"),
        ),
        "anthropic" => (
            "https://api.anthropic.com/v1/models",
            "x-api-key",
            key.to_string(),
        ),
        "gemini" => (
            "https://generativelanguage.googleapis.com/v1beta/models",
            "x-goog-api-key",
            key.to_string(),
        ),
        _ => return Err(()),
    };
    let mut request = client.get(url).header("accept", "application/json");
    request = request.header(header_name, header_value);
    if provider == "anthropic" {
        request = request.header("anthropic-version", "2023-06-01");
    }
    let response = request.send().await.map_err(|_| ())?;
    if !response.status().is_success() {
        return Err(());
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BODY as u64)
    {
        return Err(());
    }
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if bytes.len() + chunk.len() > MAX_BODY {
            return Err(());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| ())?;
    parse_model_ids(provider, &value)
}

fn parse_model_ids(provider: &str, value: &Value) -> Result<Vec<String>, ()> {
    let items = value
        .get("data")
        .or_else(|| value.get("models"))
        .and_then(Value::as_array)
        .ok_or(())?;
    let mut models = Vec::new();
    for item in items {
        if provider == "gemini" && !item.get("supportedGenerationMethods").and_then(Value::as_array).is_some_and(|methods| methods.iter().any(|method| method.as_str() == Some("generateContent"))) { continue; }
        let raw = item
            .get("id")
            .or_else(|| item.get("name"))
            .and_then(Value::as_str)
            .ok_or(())?;
        let id = if provider == "gemini" {
            raw.strip_prefix("models/").unwrap_or(raw)
        } else {
            raw
        };
        if !id.is_empty() && id.len() <= 160 {
            models.push(id.to_string());
        }
    }
    models.sort();
    models.dedup();
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_deduplicated_provider_models_and_only_generative_gemini_models() {
        let openai = json!({"data": [{"id": "gpt-z"}, {"id": "gpt-a"}, {"id": "gpt-a"}]});
        assert_eq!(parse_model_ids("openai", &openai).unwrap(), ["gpt-a", "gpt-z"]);
        let gemini = json!({"models": [
            {"name":"models/gemini-pro", "supportedGenerationMethods":["generateContent"]},
            {"name":"models/embed", "supportedGenerationMethods":["embedContent"]}
        ]});
        assert_eq!(parse_model_ids("gemini", &gemini).unwrap(), ["gemini-pro"]);
        assert!(parse_model_ids("openai", &json!({"error":"bad"})).is_err());
    }
}

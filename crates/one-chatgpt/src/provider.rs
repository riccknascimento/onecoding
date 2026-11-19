use crate::auth::{AccountSummary, AvailableModel, ChatGptAuth, ChatGptAuthError};
use one_orchestrator::{
    ChatItem, OrchestratorProvider, ProviderError, ToolCall, TurnEvent, TurnOutput, TurnRequest,
};
use one_protocol::{ProviderStatus, UsageInfo};
use serde_json::{json, Value};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// Intervalo com que a espera por dados confere o pedido de cancelamento.
const CANCEL_POLL: Duration = Duration::from_millis(50);
const RESPONSES_URL: &str = "https://api.openai.com/v1/responses";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

pub struct ChatGptProvider {
    auth: ChatGptAuth,
    session_gate: Mutex<()>,
}

impl ChatGptProvider {
    pub fn new() -> Result<Self, ChatGptAuthError> {
        let auth = ChatGptAuth::new()?;
        Ok(Self {
            auth,
            session_gate: Mutex::new(()),
        })
    }

    pub fn connect(&self) -> Result<AccountSummary, ChatGptAuthError> {
        let _guard = self.session_gate.lock().unwrap_or_else(|p| p.into_inner());
        self.auth.connect()
    }

    pub fn refresh_models(&self) -> Result<AccountSummary, ChatGptAuthError> {
        let _guard = self.session_gate.lock().unwrap_or_else(|p| p.into_inner());
        self.auth.reload_models()
    }

    pub fn disconnect(&self) -> Result<(), ChatGptAuthError> {
        let _guard = self.session_gate.lock().unwrap_or_else(|p| p.into_inner());
        self.auth.disconnect()
    }

    pub fn complete_chat(
        &self,
        model_slug: &str,
        req: &TurnRequest<'_>,
        sink: &mut dyn FnMut(TurnEvent),
    ) -> Result<TurnOutput, ProviderError> {
        self.complete_chat_cancellable(model_slug, req, None, sink)
    }

    pub fn complete_chat_cancellable(
        &self,
        model_slug: &str,
        req: &TurnRequest<'_>,
        cancelled: Option<&AtomicBool>,
        sink: &mut dyn FnMut(TurnEvent),
    ) -> Result<TurnOutput, ProviderError> {
        let _guard = self.session_gate.lock().unwrap_or_else(|p| p.into_inner());
        let (credentials, access_token) = self.auth.token_for_request().map_err(auth_error)?;
        let model = credentials
            .models
            .iter()
            .find(|m| m.slug == model_slug)
            .ok_or_else(|| {
                ProviderError::Unsupported(
                    "O modelo selecionado não está autorizado para esta conta.".into(),
                )
            })?;
        if !req.tools.is_empty() {
            return Err(ProviderError::Unsupported(
                "Ferramentas estão desativadas no chat nesta fase.".into(),
            ));
        }
        self.send_response(req, model, &access_token, cancelled, sink)
    }

    fn send_response(
        &self,
        req: &TurnRequest<'_>,
        model: &AvailableModel,
        access_token: &str,
        cancelled: Option<&AtomicBool>,
        sink: &mut dyn FnMut(TurnEvent),
    ) -> Result<TurnOutput, ProviderError> {
        let request = responses_request(req, model);
        let result = stream_response(RESPONSES_URL, access_token, &request, cancelled, sink);
        if matches!(&result, Err(ProviderError::NotConnected(message)) if message.contains("subscription_sharing_invalid_user"))
        {
            self.auth
                .clear_tokens()
                .map_err(|e| ProviderError::Failed(e.to_string()))?;
        }
        result
    }
}

impl OrchestratorProvider for ChatGptProvider {
    fn id(&self) -> &'static str {
        "chatgpt_plan"
    }

    fn status(&self) -> ProviderStatus {
        match self.auth.account() {
            Ok(Some(account)) => ProviderStatus::Connected {
                provider: self.id().into(),
                label: account.email.unwrap_or_else(|| "ChatGPT".into()),
                models: account.models.iter().map(model_label).collect(),
                plan_usage_authorized: account.plan_usage_authorized,
            },
            Ok(None) => ProviderStatus::Disconnected {
                reason: "Conecte sua conta ChatGPT para usar o OneOrchestrator.".into(),
            },
            Err(error) => ProviderStatus::Error {
                message: error.to_string(),
            },
        }
    }

    fn complete(
        &self,
        req: &TurnRequest<'_>,
        sink: &mut dyn FnMut(TurnEvent),
    ) -> Result<TurnOutput, ProviderError> {
        self.complete_cancellable(req, None, sink)
    }

    /// Cancelar fecha a conexão HTTP em andamento (ver `until_cancelled`).
    fn complete_cancellable(
        &self,
        req: &TurnRequest<'_>,
        cancelled: Option<&AtomicBool>,
        sink: &mut dyn FnMut(TurnEvent),
    ) -> Result<TurnOutput, ProviderError> {
        let _guard = self.session_gate.lock().unwrap_or_else(|p| p.into_inner());
        let (credentials, access_token) = self.auth.token_for_request().map_err(auth_error)?;
        let model = select_model(&credentials.models, req.model)?;
        self.send_response(req, model, &access_token, cancelled, sink)
    }
}

/// Modelo pedido pelo usuário (tem de existir na conta) ou o primeiro da conta. Nunca troca de modelo
/// em silêncio: um modelo pedido e ausente é erro.
fn select_model<'m>(
    models: &'m [AvailableModel],
    requested: Option<&str>,
) -> Result<&'m AvailableModel, ProviderError> {
    match requested {
        Some(slug) => models.iter().find(|m| m.slug == slug).ok_or_else(|| {
            ProviderError::Unsupported(format!("O modelo {slug} não está disponível nesta conta."))
        }),
        None => models.first().ok_or_else(|| {
            ProviderError::Unsupported(
                "A conta não retornou modelos disponíveis para ChatGPT Plan Usage.".into(),
            )
        }),
    }
}

fn model_label(model: &AvailableModel) -> String {
    format!("{} ({})", model.display_name, model.slug)
}

fn responses_request(req: &TurnRequest<'_>, model: &AvailableModel) -> Value {
    let input: Vec<Value> = req.items.iter().map(|item| match item {
        ChatItem::User(text) => json!({"role":"user", "content":[{"type":"input_text", "text":text}]}),
        ChatItem::Assistant(text) => json!({"role":"assistant", "content":text}),
        ChatItem::ToolCall(call) => json!({"type":"function_call", "call_id":call.id, "name":call.name, "arguments":call.args.to_string()}),
        ChatItem::ToolOutput { call_id, content } => json!({"type":"function_call_output", "call_id":call_id, "output":content}),
    }).collect();
    let tools: Vec<Value> = req
        .tools
        .iter()
        .map(|tool| {
            json!({
                "type":"function", "name":tool.name, "description":tool.description,
                "parameters":tool.parameters, "strict":true,
            })
        })
        .collect();
    json!({
        "model":model.slug,
        "instructions":req.instructions,
        "input":input,
        "tools":tools,
        "max_output_tokens":2048,
        "store":false,
        "stream":true,
    })
}

fn interrupted() -> ProviderError {
    ProviderError::Failed("Geração interrompida pelo usuário.".into())
}

fn is_cancelled(cancelled: Option<&AtomicBool>) -> bool {
    cancelled.is_some_and(|flag| flag.load(Ordering::Acquire))
}

/// Espera `fut`, mas devolve `Err` assim que o cancelamento for pedido. O future é descartado ao
/// retornar: isso fecha a conexão HTTP em andamento (nada fica rodando em segundo plano).
async fn until_cancelled<T>(
    cancelled: Option<&AtomicBool>,
    fut: impl Future<Output = T>,
) -> Result<T, ProviderError> {
    tokio::pin!(fut);
    loop {
        if is_cancelled(cancelled) {
            return Err(interrupted());
        }
        tokio::select! {
            out = &mut fut => return Ok(out),
            _ = tokio::time::sleep(CANCEL_POLL) => {}
        }
    }
}

/// Envia a requisição e consome o stream SSE. Roda num runtime próprio de uma só thread, descartado ao
/// final: cancelar (ou terminar) encerra a leitura bloqueada e libera a conexão e a thread.
fn stream_response(
    url: &str,
    access_token: &str,
    request: &Value,
    cancelled: Option<&AtomicBool>,
    sink: &mut dyn FnMut(TurnEvent),
) -> Result<TurnOutput, ProviderError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| ProviderError::Failed(format!("Falha ao preparar o transporte: {e}")))?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| ProviderError::Failed(format!("Falha ao preparar o transporte: {e}")))?;
        let mut response = until_cancelled(
            cancelled,
            client
                .post(url)
                .bearer_auth(access_token)
                .json(request)
                .send(),
        )
        .await?
        .map_err(|e| {
            ProviderError::Failed(format!("Falha de transporte com a Responses API: {e}"))
        })?;
        if !response.status().is_success() {
            let status = response.status();
            let request_id = response
                .headers()
                .get("x-request-id")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned();
            let body = until_cancelled(cancelled, response.json::<Value>())
                .await?
                .unwrap_or(Value::Null);
            return Err(http_error(status, &request_id, &body));
        }
        let mut parser = SseParser::default();
        let mut buf: Vec<u8> = Vec::new();
        loop {
            let chunk = until_cancelled(cancelled, response.chunk())
                .await?
                .map_err(|e| ProviderError::Failed(format!("Falha lendo o stream: {e}")))?;
            let Some(bytes) = chunk else { break };
            buf.extend_from_slice(&bytes);
            while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                if parser.feed_line(&String::from_utf8_lossy(&line), sink)? {
                    return parser.finish();
                }
            }
        }
        if !buf.is_empty() {
            parser.feed_line(&String::from_utf8_lossy(&buf), sink)?;
        }
        parser.finish()
    })
}

fn http_error(status: reqwest::StatusCode, request_id: &str, body: &Value) -> ProviderError {
    let code = error_code(body).unwrap_or("unknown_error");
    let message = format!(
        "Responses API HTTP {status} ({code}){}",
        if request_id.is_empty() {
            String::new()
        } else {
            format!("; request ID {request_id}")
        }
    );
    if status.as_u16() == 429 || code == "subscription_sharing_usage_limit_exceeded" {
        ProviderError::RateLimited(message)
    } else if status.as_u16() == 401
        || code == "subscription_sharing_user_not_eligible"
        || code == "subscription_sharing_invalid_user"
    {
        ProviderError::NotConnected(message)
    } else if code == "subscription_sharing_unsupported_capability" {
        ProviderError::Unsupported(message)
    } else {
        ProviderError::Failed(message)
    }
}

/// Lê um stream SSE já em memória (testes) linha a linha, com a mesma lógica do stream de rede.
#[cfg(test)]
fn consume_sse(
    mut reader: impl std::io::BufRead,
    cancelled: Option<&AtomicBool>,
    sink: &mut dyn FnMut(TurnEvent),
) -> Result<TurnOutput, ProviderError> {
    let mut parser = SseParser::default();
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .map_err(|e| ProviderError::Failed(format!("Falha lendo o stream: {e}")))?;
        if is_cancelled(cancelled) {
            return Err(interrupted());
        }
        if n == 0 || parser.feed_line(&line, sink)? {
            return parser.finish();
        }
    }
}

#[derive(Default)]
struct SseParser {
    output: TurnOutput,
    completed: bool,
    data: String,
}

impl SseParser {
    /// Processa uma linha; `true` quando `response.completed` chegou.
    fn feed_line(
        &mut self,
        line: &str,
        sink: &mut dyn FnMut(TurnEvent),
    ) -> Result<bool, ProviderError> {
        if let Some(value) = line.strip_prefix("data:") {
            if !self.data.is_empty() {
                self.data.push('\n');
            }
            self.data.push_str(value.trim_start());
        } else if line.trim().is_empty() && !self.data.is_empty() {
            let event: Value = serde_json::from_str(&self.data).map_err(|_| {
                ProviderError::Failed("Evento SSE inválido recebido da Responses API.".into())
            })?;
            self.data.clear();
            match event.get("type").and_then(Value::as_str).unwrap_or("") {
                "response.output_text.delta" => {
                    if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                        self.output.text.push_str(delta);
                        sink(TurnEvent::TextDelta(delta.to_owned()));
                    }
                }
                "response.output_item.added" | "response.output_item.done" => {
                    if let Some(item) = event.get("item") {
                        if item.get("type").and_then(Value::as_str) == Some("function_call")
                            && event.get("type").and_then(Value::as_str)
                                == Some("response.output_item.done")
                        {
                            let call = tool_call(item).ok_or_else(|| {
                                ProviderError::Failed(
                                    "Chamada de ferramenta malformada recebida da Responses API."
                                        .into(),
                                )
                            })?;
                            sink(TurnEvent::ToolCall(call.clone()));
                            self.output.tool_calls.push(call);
                        }
                    }
                }
                "response.completed" => {
                    self.output.model = event
                        .pointer("/response/model")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    if let Some(usage) = event.pointer("/response/usage") {
                        self.output.usage = parse_usage(usage);
                    }
                    sink(TurnEvent::Usage(self.output.usage.clone()));
                    self.completed = true;
                }
                // erro transmitido pela própria Responses API (ex.: parâmetros de ferramenta rejeitados):
                // antes era ignorado e o stream terminava "sem response.completed", sem a causa real.
                "error" => return Err(classify_error(&event)),
                "response.failed" => {
                    let err = event
                        .pointer("/response/error")
                        .cloned()
                        .unwrap_or(Value::Null);
                    return Err(classify_error(&err));
                }
                "response.incomplete" => {
                    return Err(ProviderError::Failed(
                        "A Responses API encerrou a resposta como incompleta.".into(),
                    ))
                }
                _ => {}
            }
        }
        Ok(self.completed)
    }

    fn finish(self) -> Result<TurnOutput, ProviderError> {
        if self.completed {
            Ok(self.output)
        } else {
            Err(ProviderError::Failed(
                "O stream da Responses API terminou sem response.completed.".into(),
            ))
        }
    }
}

fn tool_call(item: &Value) -> Option<ToolCall> {
    Some(ToolCall {
        id: item.get("call_id")?.as_str()?.to_owned(),
        name: item.get("name")?.as_str()?.to_owned(),
        args: serde_json::from_str(item.get("arguments")?.as_str()?).ok()?,
    })
}

fn parse_usage(value: &Value) -> UsageInfo {
    UsageInfo {
        input_tokens: value
            .get("input_tokens")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok()),
        output_tokens: value
            .get("output_tokens")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok()),
        cached_tokens: value
            .pointer("/input_tokens_details/cached_tokens")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok()),
        reasoning_tokens: value
            .pointer("/output_tokens_details/reasoning_tokens")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok()),
    }
}

fn error_code(body: &Value) -> Option<&str> {
    body.pointer("/error/code")
        .and_then(Value::as_str)
        .or_else(|| body.get("code").and_then(Value::as_str))
        .or_else(|| body.pointer("/error/type").and_then(Value::as_str))
}

fn classify_error(error: &Value) -> ProviderError {
    let code = error_code(error).unwrap_or("unknown_error");
    let message = format!("Responses API: {code}");
    match code {
        "subscription_sharing_usage_limit_exceeded" => ProviderError::RateLimited(message),
        "subscription_sharing_user_not_eligible" | "subscription_sharing_invalid_user" => {
            ProviderError::NotConnected(message)
        }
        "subscription_sharing_unsupported_capability"
        | "subscription_sharing_route_not_supported" => ProviderError::Unsupported(message),
        _ => ProviderError::Failed(message),
    }
}

fn auth_error(error: ChatGptAuthError) -> ProviderError {
    match error {
        ChatGptAuthError::PlanUsageNotAuthorized
        | ChatGptAuthError::ReauthenticationRequired(_) => {
            ProviderError::NotConnected(error.to_string())
        }
        other => ProviderError::Failed(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_orchestrator::{ToolSpec, TurnRequest};
    use std::io::Cursor;

    #[test]
    fn normal_chat_request_has_no_tools_and_is_not_stored_remotely() {
        let items = [ChatItem::User("mensagem".into())];
        let tools = [];
        let req = TurnRequest {
            instructions: "",
            items: &items,
            tools: &tools,
            model: None,
        };
        let json = responses_request(
            &req,
            &AvailableModel {
                slug: "model-test".into(),
                display_name: "Test model".into(),
            },
        );
        assert_eq!(json["model"], "model-test");
        assert_eq!(json["instructions"], "");
        assert_eq!(json["tools"], json!([]));
        assert_eq!(json["input"].as_array().unwrap().len(), 1);
        assert_eq!(json["input"][0]["content"][0]["text"], "mensagem");
        assert_eq!(json["stream"], true);
        assert_eq!(json["store"], false);
        assert_eq!(json["max_output_tokens"], 2048);
    }

    #[test]
    fn responses_payload_is_streamed_unstored_and_uses_typed_tools() {
        let items = vec![ChatItem::User("hello".into())];
        let tools = vec![ToolSpec {
            name: "task_run",
            description: "run",
            parameters: json!({"type":"object","additionalProperties":false}),
        }];
        let req = TurnRequest {
            instructions: "Do safe work",
            items: &items,
            tools: &tools,
            model: None,
        };
        let model = AvailableModel {
            slug: "gpt-account-model".into(),
            display_name: "Model".into(),
        };
        let payload = responses_request(&req, &model);
        assert_eq!(payload["store"], false);
        assert_eq!(payload["stream"], true);
        assert_eq!(payload["model"], "gpt-account-model");
        assert_eq!(payload["tools"][0]["strict"], true);
    }

    #[test]
    fn usage_and_plan_errors_are_mapped_without_fallback() {
        assert_eq!(
            classify_error(&json!({"code":"subscription_sharing_usage_limit_exceeded"})),
            ProviderError::RateLimited(
                "Responses API: subscription_sharing_usage_limit_exceeded".into()
            )
        );
        assert_eq!(
            classify_error(&json!({"code":"subscription_sharing_user_not_eligible"})),
            ProviderError::NotConnected(
                "Responses API: subscription_sharing_user_not_eligible".into()
            )
        );
        let usage = parse_usage(
            &json!({"input_tokens":10,"output_tokens":4,"input_tokens_details":{"cached_tokens":3}}),
        );
        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.cached_tokens, Some(3));
    }

    #[test]
    fn controlled_response_stream_emits_text_tools_and_usage() {
        let events = [
            json!({"type":"response.output_text.delta","delta":"hello"}),
            json!({"type":"response.output_item.done","item":{"id":"fc1","type":"function_call","call_id":"call1","name":"task_status","arguments":"{\"run_id\":\"run1\"}"}}),
            json!({"type":"response.completed","response":{"model":"gpt-verified","usage":{"input_tokens":12,"output_tokens":7,"input_tokens_details":{"cached_tokens":4}}}}),
        ];
        let body = events
            .iter()
            .map(|e| format!("data: {}\n\n", e))
            .collect::<String>();
        let mut observed = vec![];
        let result =
            consume_sse(Cursor::new(body), None, &mut |event| observed.push(event)).unwrap();
        assert_eq!(result.text, "hello");
        assert_eq!(result.tool_calls[0].name, "task_status");
        assert_eq!(result.tool_calls[0].args["run_id"], "run1");
        assert_eq!(result.usage.input_tokens, Some(12));
        assert_eq!(result.model.as_deref(), Some("gpt-verified"));
        assert_eq!(result.usage.cached_tokens, Some(4));
        assert!(observed
            .iter()
            .any(|e| matches!(e, TurnEvent::TextDelta(s) if s == "hello")));
    }

    /// Regressão do erro observado: um evento `error` em-stream era ignorado e o parser só relatava
    /// "terminou sem response.completed". Agora o código de erro chega ao chamador.
    #[test]
    fn in_stream_error_event_is_reported_with_its_code_not_as_a_missing_completion() {
        let event = json!({"type":"error","code":"invalid_function_parameters","message":"schema rejeitado","sequence_number":3});
        let result = consume_sse(Cursor::new(format!("data: {event}\n\n")), None, &mut |_| {});
        match result {
            Err(ProviderError::Failed(m)) => {
                assert!(m.contains("invalid_function_parameters"), "{m}");
                assert!(!m.contains("sem response.completed"), "{m}");
            }
            other => panic!("esperado Failed com o código, veio {other:?}"),
        }
    }

    #[test]
    fn in_stream_error_event_with_plan_limit_is_classified_as_rate_limit() {
        let event = json!({"type":"error","code":"subscription_sharing_usage_limit_exceeded"});
        let result = consume_sse(Cursor::new(format!("data: {event}\n\n")), None, &mut |_| {});
        assert!(matches!(result, Err(ProviderError::RateLimited(_))));
    }

    #[test]
    fn requested_model_must_exist_in_the_account_and_default_is_the_first() {
        let models = vec![
            AvailableModel {
                slug: "a".into(),
                display_name: "A".into(),
            },
            AvailableModel {
                slug: "b".into(),
                display_name: "B".into(),
            },
        ];
        assert_eq!(select_model(&models, None).unwrap().slug, "a");
        assert_eq!(select_model(&models, Some("b")).unwrap().slug, "b");
        // modelo pedido e ausente é erro, nunca troca silenciosa
        assert!(matches!(
            select_model(&models, Some("zzz")),
            Err(ProviderError::Unsupported(_))
        ));
        assert!(matches!(
            select_model(&[], None),
            Err(ProviderError::Unsupported(_))
        ));
    }

    #[test]
    fn sse_data_lines_of_one_event_are_joined_with_newlines() {
        let body = "data: {\"type\":\"response.output_text.delta\",\ndata: \"delta\":\"ok\"}\n\n\
                    data: {\"type\":\"response.completed\",\"response\":{\"model\":\"m\"}}\n\n";
        let out = consume_sse(Cursor::new(body), None, &mut |_| {}).unwrap();
        assert_eq!(out.text, "ok");
        assert_eq!(out.model.as_deref(), Some("m"));
    }

    #[test]
    fn malformed_tool_call_is_an_error_not_a_silent_drop() {
        let item = json!({"type":"response.output_item.done","item":{"type":"function_call","call_id":"c1","name":"task_status","arguments":"{nao-json"}});
        let done = json!({"type":"response.completed","response":{"model":"m"}});
        let body = format!("data: {item}\n\ndata: {done}\n\n");
        let result = consume_sse(Cursor::new(body), None, &mut |_| {});
        assert!(matches!(result, Err(ProviderError::Failed(m)) if m.contains("malformada")));
    }

    #[test]
    fn assistant_history_uses_plain_content_and_user_keeps_input_text() {
        let model = AvailableModel {
            slug: "m".into(),
            display_name: "M".into(),
        };
        let items = [
            ChatItem::User("oi".into()),
            ChatItem::Assistant("olá".into()),
        ];
        let req = TurnRequest {
            instructions: "",
            items: &items,
            tools: &[],
            model: None,
        };
        let payload = responses_request(&req, &model);
        assert_eq!(payload["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(payload["input"][1]["content"], "olá");
    }

    #[test]
    fn controlled_response_failure_stops_on_plan_limit() {
        let event = json!({"type":"response.failed","response":{"error":{"code":"subscription_sharing_usage_limit_exceeded"}}});
        let result = consume_sse(Cursor::new(format!("data: {event}\n\n")), None, &mut |_| {});
        assert!(matches!(result, Err(ProviderError::RateLimited(_))));
    }

    #[test]
    fn cancellation_stops_sse_consumption_without_marking_a_response_complete() {
        let cancelled = AtomicBool::new(true);
        let result = consume_sse(Cursor::new("data: {}\n\n"), Some(&cancelled), &mut |_| {});
        assert!(
            matches!(result, Err(ProviderError::Failed(message)) if message.contains("interrompida"))
        );
    }

    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::time::Instant;

    fn chunk(data: &str) -> String {
        format!("{:x}\r\n{}\r\n", data.len(), data)
    }

    const SSE_HEAD: &str =
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n";

    /// Servidor local de um único cliente. `script` escreve o que quiser e depois devolve o controle;
    /// em seguida o servidor fica parado e registra se o cliente fechou a conexão.
    fn serve(
        script: impl FnOnce(&mut std::net::TcpStream) + Send + 'static,
    ) -> (String, Arc<AtomicBool>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/responses", listener.local_addr().unwrap());
        let closed = Arc::new(AtomicBool::new(false));
        let seen = closed.clone();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_millis(100)))
                .unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut buf) {
                    Ok(0) => return,
                    Ok(n) => request.extend_from_slice(&buf[..n]),
                    Err(_) => {}
                }
            }
            script(&mut stream);
            // Parado: o cliente tem de ser quem encerra a conexão.
            for _ in 0..100 {
                match stream.read(&mut buf) {
                    Ok(0) => {
                        seen.store(true, Ordering::Release);
                        return;
                    }
                    Ok(_) => {}
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::TimedOut => {}
                    Err(_) => {
                        seen.store(true, Ordering::Release);
                        return;
                    }
                }
            }
        });
        (url, closed)
    }

    fn wait_closed(closed: &AtomicBool) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if closed.load(Ordering::Acquire) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn cancel_interrupts_a_stream_that_stops_sending_events() {
        let delta = json!({"type":"response.output_text.delta","delta":"parcial"});
        let (url, closed) = serve(move |s| {
            s.write_all(SSE_HEAD.as_bytes()).unwrap();
            s.write_all(chunk(&format!("data: {delta}\n\n")).as_bytes())
                .unwrap();
            s.flush().unwrap();
        });
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let started = Instant::now();
        let mut text = String::new();
        let result = stream_response(&url, "tok", &json!({}), Some(&cancelled), &mut |e| {
            if let TurnEvent::TextDelta(d) = e {
                text.push_str(&d);
                // o usuário clica em "Interromper" depois do primeiro trecho; o servidor não manda mais nada
                flag.store(true, Ordering::Release);
            }
        });
        assert_eq!(text, "parcial");
        assert!(
            matches!(&result, Err(ProviderError::Failed(m)) if m.contains("interrompida")),
            "{result:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "cancelamento demorou {:?} (o timeout da requisição é de 300 s)",
            started.elapsed()
        );
        assert!(
            wait_closed(&closed),
            "a conexão HTTP continuou aberta depois do cancelamento"
        );
    }

    #[test]
    fn cancel_interrupts_a_request_that_never_gets_a_response() {
        let (url, closed) = serve(|_| {}); // aceita e nunca responde
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            flag.store(true, Ordering::Release);
        });
        let started = Instant::now();
        let result = stream_response(&url, "tok", &json!({}), Some(&cancelled), &mut |_| {});
        assert!(
            matches!(&result, Err(ProviderError::Failed(m)) if m.contains("interrompida")),
            "{result:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(wait_closed(&closed));
    }

    #[test]
    fn network_stream_split_across_chunks_still_completes_with_usage() {
        let done = json!({"type":"response.completed","response":{"model":"m","usage":{"input_tokens":5,"output_tokens":2}}});
        let delta = json!({"type":"response.output_text.delta","delta":"oi"});
        let body = format!("data: {delta}\n\ndata: {done}\n\n");
        let (a, b) = body.split_at(25); // corta no meio do primeiro evento
        let (a, b) = (a.to_owned(), b.to_owned());
        let (url, _closed) = serve(move |s| {
            s.write_all(SSE_HEAD.as_bytes()).unwrap();
            s.write_all(chunk(&a).as_bytes()).unwrap();
            s.flush().unwrap();
            std::thread::sleep(Duration::from_millis(50));
            s.write_all(chunk(&b).as_bytes()).unwrap();
            s.flush().unwrap();
        });
        let out = stream_response(&url, "tok", &json!({}), None, &mut |_| {}).unwrap();
        assert_eq!(out.text, "oi");
        assert_eq!(out.usage.input_tokens, Some(5));
        assert_eq!(out.model.as_deref(), Some("m"));
    }

    #[test]
    fn http_error_status_is_classified_without_reading_a_stream() {
        let (url, _closed) = serve(|s| {
            let body = r#"{"error":{"code":"subscription_sharing_usage_limit_exceeded"}}"#;
            write!(
                s,
                "HTTP/1.1 429 Too Many Requests\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let result = stream_response(&url, "tok", &json!({}), None, &mut |_| {});
        assert!(matches!(result, Err(ProviderError::RateLimited(_))));
    }
}

//! Contratos entre UI, OneHub, Agent Gateway (OneNode) e adaptadores.
//!
//! Regras: sem lógica, sem I/O; só tipos serializáveis. `cargo test -p one-protocol`
//! regenera os tipos TypeScript em `apps/desktop/src/contracts/generated`.
//! Timestamps são strings RFC 3339 (UTC).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Versão do protocolo Hub ↔ Gateway. Incrementar em mudança incompatível.
pub const PROTOCOL_VERSION: u32 = 1;

/// Motor (CLI executora) suportado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum EngineKind {
    Codex,
    Claude,
    Cursor,
}

/// Estado de login da CLI. `Unknown` quando não foi possível determinar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AuthState {
    LoggedIn,
    LoggedOut,
    Unknown,
}

/// De onde veio a informação do modelo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ModelSource {
    /// Catálogo/listagem fornecido pela própria CLI.
    Catalog,
    /// Apelido estático (ex.: `opus`); a CLI não lista modelos.
    Alias,
    /// Modelo reportado em execução real.
    Reported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModelInfo {
    pub id: String,
    pub label: Option<String>,
    pub source: ModelSource,
    /// `true` só quando a CLI confirmou o modelo (catálogo ou execução).
    pub confirmed: bool,
}

/// Confiança no executável da CLI. A CLI só é executada quando `Trusted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TrustState {
    /// Identidade igual à aprovada pelo usuário.
    Trusted,
    /// Nunca aprovado: precisa de aprovação explícita antes de qualquer execução.
    Unknown,
    /// Já aprovado, mas o executável mudou: exige nova validação.
    Changed,
    /// Reprovado nas verificações de dono/permissões; não pode ser aprovado.
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EngineStatus {
    pub engine: EngineKind,
    pub installed: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub auth: AuthState,
    pub trust: TrustState,
    /// Identidade (SHA-256 hex) do executável encontrado; é o que o usuário aprova.
    pub fingerprint: Option<String>,
    pub models: Vec<ModelInfo>,
    /// Aviso curto para a UI (ex.: "timeout ao listar modelos"). Nunca contém saída bruta da CLI.
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum GatewayKind {
    Local,
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum GatewayStatus {
    Online,
    Offline,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GatewayInfo {
    pub id: String,
    pub name: String,
    pub kind: GatewayKind,
    pub status: GatewayStatus,
    pub os: String,
    pub protocol_version: u32,
    pub engines: Vec<EngineStatus>,
    /// `true` para dados de demonstração (sem integração real).
    pub demo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub gateway_id: String,
    /// Caminho no Gateway (nunca no Mac, para Gateways remotos).
    pub path: String,
    pub demo: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ExecMode {
    ReadOnly,
    Write,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Task {
    pub id: String,
    pub gateway_id: String,
    pub project_id: String,
    pub engine: EngineKind,
    pub model: Option<String>,
    pub mode: ExecMode,
    pub prompt: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RunStatus {
    Queued,
    Running,
    AwaitingApproval,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Run {
    pub id: String,
    pub task_id: String,
    pub status: RunStatus,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub exit_code: Option<i32>,
    /// Referências a evidências guardadas no Gateway (o Mac não copia logs completos).
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CapabilityKind {
    Skill,
    Agent,
    Command,
    Plugin,
    Mcp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CapabilityScope {
    Global,
    Gateway,
    Project,
    Cli,
}

/// Entrada do OneSkills Registry (ver docs/CAPABILITIES.md).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Capability {
    pub id: String,
    pub kind: CapabilityKind,
    pub name: String,
    pub scope: CapabilityScope,
    /// `None` = não específico de um motor.
    pub engine: Option<EngineKind>,
    pub version: Option<String>,
    pub source: String,
    pub demo: bool,
}

/// Informações do app e do armazenamento local para a tela de Configurações.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StoreStatus {
    pub schema_version: u32,
    pub fts5: bool,
    pub integrity_ok: bool,
}

// ───────────────────────── Execução (Fase 2A) ─────────────────────────

/// Classe do erro de uma execução. Mensagens ao usuário saem daqui, não de saída bruta das CLIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ErrorKind {
    InvalidRequest,
    NotApproved,
    WorkspaceNotAuthorized,
    WorkspaceBusy,
    SpawnFailed,
    ExecutorFailed,
    Timeout,
    Cancelled,
    OutputLimit,
    /// O app fechou (ou caiu) durante a execução.
    Interrupted,
}

/// Workspace autorizado pelo usuário (diálogo nativo). A webview só vê isto; nunca define caminhos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Workspace {
    pub id: String,
    pub gateway_id: String,
    pub label: String,
    /// Caminho canônico, só para exibição.
    pub path: String,
}

/// Pedido de execução vindo da UI. Só identificadores e texto: nada de caminhos, comandos ou flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StartRunRequest {
    pub workspace_id: String,
    pub engine: EngineKind,
    /// Modelo solicitado; `None` = padrão da CLI.
    pub model: Option<String>,
    pub prompt: String,
}

/// Uso informado pelo executor. Campos ausentes continuam `None` (nunca estimados).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UsageInfo {
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub cached_tokens: Option<u32>,
    pub reasoning_tokens: Option<u32>,
}

/// Evento normalizado, independente do motor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum EventPayload {
    Status {
        status: RunStatus,
    },
    /// Mensagem do agente (texto já truncado e sem segredos conhecidos).
    Message {
        text: String,
    },
    ToolStarted {
        name: String,
        summary: String,
    },
    ToolFinished {
        name: String,
        ok: bool,
        exit_code: Option<i32>,
    },
    Usage {
        usage: UsageInfo,
    },
    Warning {
        message: String,
    },
    Error {
        kind: ErrorKind,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunEvent {
    pub run_id: String,
    /// Ordem dentro da execução, começando em 1.
    pub seq: u32,
    pub ts: String,
    pub payload: EventPayload,
}

/// Visão de uma execução para a UI e o histórico (junta tarefa + execução + métricas OneUsage).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunView {
    pub run_id: String,
    pub task_id: String,
    pub gateway_id: String,
    pub workspace_id: String,
    pub workspace_label: String,
    pub engine: EngineKind,
    pub model_requested: Option<String>,
    /// Modelo informado pelo executor; `None` quando o motor não informa.
    pub model_reported: Option<String>,
    pub mode: ExecMode,
    pub prompt: String,
    pub status: RunStatus,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub duration_ms: Option<u32>,
    pub exit_code: Option<i32>,
    // Sessão interna do motor (ex.: thread do Codex), separada do contexto da tarefa.
    pub engine_session_id: Option<String>,
    pub usage: UsageInfo,
    pub tool_calls: u32,
    pub attempts: u32,
    pub initiator: RunInitiator,
    pub error_kind: Option<ErrorKind>,
    pub error_message: Option<String>,
    // Resposta final do agente (truncada).
    pub final_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunDetail {
    pub run: RunView,
    pub events: Vec<RunEvent>,
}

// ───────────────────────── Orquestração (Etapa 3A) ─────────────────────────

/// Modo da conversa. O Modo direto NUNCA é apresentado como ChatGPT Orquestrador.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ChatMode {
    Orchestrator,
    Direct,
}

/// Quem iniciou a tarefa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RunInitiator {
    User,
    Orchestrator,
}

/// Estado REAL do provedor do orquestrador. A UI só mostra "conectado" quando isto disser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
#[ts(export)]
pub enum ProviderStatus {
    Disconnected {
        reason: String,
    },
    Simulated {
        provider: String,
        label: String,
    },
    Connected {
        provider: String,
        label: String,
        models: Vec<String>,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ChatRole {
    User,
    Orchestrator,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub text: String,
    pub ts: String,
}

/// Resultado de um envio ao orquestrador.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OrchestratorReply {
    pub conversation_id: String,
    // Resposta final; `None` quando o turno parou antes (limite ou erro do provedor).
    pub text: Option<String>,
    pub steps: u32,
    pub tool_calls: u32,
    // Motivo da parada antecipada, em texto para o usuário.
    pub stopped: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let p = format!(
            "{}/../../protocol/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"))
    }

    fn sample_gateway() -> GatewayInfo {
        GatewayInfo {
            id: "local".into(),
            name: "Este Mac".into(),
            kind: GatewayKind::Local,
            status: GatewayStatus::Online,
            os: "macos-aarch64".into(),
            protocol_version: PROTOCOL_VERSION,
            engines: vec![EngineStatus {
                engine: EngineKind::Codex,
                installed: true,
                path: Some("/usr/local/bin/codex".into()),
                version: Some("0.0.0".into()),
                auth: AuthState::LoggedIn,
                trust: TrustState::Trusted,
                fingerprint: Some("ab12".into()),
                models: vec![ModelInfo {
                    id: "example-model".into(),
                    label: None,
                    source: ModelSource::Catalog,
                    confirmed: true,
                }],
                note: None,
            }],
            demo: false,
        }
    }

    fn sample_task() -> Task {
        Task {
            id: "t1".into(),
            gateway_id: "local".into(),
            project_id: "p1".into(),
            engine: EngineKind::Claude,
            model: None,
            mode: ExecMode::ReadOnly,
            prompt: "Resuma a arquitetura".into(),
            created_at: "2026-11-17T12:00:00Z".into(),
        }
    }

    fn sample_run() -> Run {
        Run {
            id: "r1".into(),
            task_id: "t1".into(),
            status: RunStatus::AwaitingApproval,
            started_at: None,
            finished_at: None,
            exit_code: None,
            evidence_refs: vec!["gw://local/runs/r1/log".into()],
        }
    }

    /// O JSON serializado deve ser idêntico ao fixture que o front TypeScript também valida.
    fn assert_matches_fixture<
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    >(
        value: &T,
        name: &str,
    ) {
        let expected: serde_json::Value = serde_json::from_str(&fixture(name)).unwrap();
        assert_eq!(serde_json::to_value(value).unwrap(), expected, "{name}");
        let back: T = serde_json::from_value(expected).unwrap();
        assert_eq!(&back, value, "roundtrip {name}");
    }

    #[test]
    fn gateway_matches_fixture() {
        assert_matches_fixture(&sample_gateway(), "gateway.json");
    }

    #[test]
    fn task_matches_fixture() {
        assert_matches_fixture(&sample_task(), "task.json");
    }

    #[test]
    fn run_matches_fixture() {
        assert_matches_fixture(&sample_run(), "run.json");
    }

    #[test]
    fn start_run_request_matches_fixture() {
        let v = StartRunRequest {
            workspace_id: "ws1".into(),
            engine: EngineKind::Codex,
            model: None,
            prompt: "Resuma a arquitetura".into(),
        };
        assert_matches_fixture(&v, "start_run_request.json");
    }

    #[test]
    fn run_event_matches_fixture_and_tags_are_snake_case() {
        let v = RunEvent {
            run_id: "r1".into(),
            seq: 3,
            ts: "2026-11-17T12:00:00Z".into(),
            payload: EventPayload::Usage {
                usage: UsageInfo {
                    input_tokens: Some(100),
                    output_tokens: Some(5),
                    ..Default::default()
                },
            },
        };
        assert_matches_fixture(&v, "run_event.json");
        let t = serde_json::to_value(EventPayload::ToolStarted {
            name: "shell".into(),
            summary: "ls".into(),
        })
        .unwrap();
        assert_eq!(t["type"], "tool_started");
    }

    #[test]
    fn provider_status_is_tagged_and_modes_are_snake_case() {
        let v = serde_json::to_value(ProviderStatus::Disconnected { reason: "x".into() }).unwrap();
        assert_eq!(v["state"], "disconnected");
        assert_eq!(
            serde_json::to_string(&ChatMode::Orchestrator).unwrap(),
            "\"orchestrator\""
        );
        assert_eq!(
            serde_json::to_string(&RunInitiator::Orchestrator).unwrap(),
            "\"orchestrator\""
        );
    }

    #[test]
    fn enums_serialize_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&RunStatus::AwaitingApproval).unwrap(),
            "\"awaiting_approval\""
        );
        assert_eq!(
            serde_json::to_string(&ExecMode::ReadOnly).unwrap(),
            "\"read_only\""
        );
        assert_eq!(
            serde_json::to_string(&AuthState::LoggedIn).unwrap(),
            "\"logged_in\""
        );
    }

    #[test]
    fn unknown_enum_value_is_rejected() {
        assert!(serde_json::from_str::<EngineKind>("\"gemini\"").is_err());
    }
}

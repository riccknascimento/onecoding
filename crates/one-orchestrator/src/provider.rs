//! Contrato do provedor do orquestrador. O provedor só TRADUZ o protocolo do modelo (texto e pedidos
//! de ferramenta); política, execução e persistência são do OneHub. Trocar de provedor (ChatGPT Plus,
//! outro) não muda nada fora desta interface.

use one_protocol::{ProviderStatus, UsageInfo};
use serde_json::Value;

/// Ferramenta oferecida ao modelo. `name` segue `^[a-zA-Z0-9_-]+$` (exigência das function tools).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema estrito (`additionalProperties: false`).
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: Value,
}

/// Item do histórico enviado ao provedor (o Hub guarda o histórico; o provedor não tem memória).
#[derive(Debug, Clone, PartialEq)]
pub enum ChatItem {
    User(String),
    Assistant(String),
    ToolCall(ToolCall),
    /// Resultado de ferramenta: DADO NÃO CONFIÁVEL, já truncado, redigido e marcado.
    ToolOutput {
        call_id: String,
        content: String,
    },
}

pub struct TurnRequest<'a> {
    pub instructions: &'a str,
    pub items: &'a [ChatItem],
    pub tools: &'a [ToolSpec],
}

/// Eventos incrementais (streaming) de um turno.
#[derive(Debug, Clone, PartialEq)]
pub enum TurnEvent {
    TextDelta(String),
    ToolCall(ToolCall),
    Usage(UsageInfo),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TurnOutput {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: UsageInfo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// Sem provedor conectado (ou sessão expirada).
    NotConnected(String),
    /// Cota ou limite do plano esgotado: pausar, não trocar de cobrança.
    RateLimited(String),
    /// O recurso pedido não é suportado pelo provedor.
    Unsupported(String),
    Failed(String),
}

impl ProviderError {
    pub fn message(&self) -> &str {
        match self {
            ProviderError::NotConnected(m)
            | ProviderError::RateLimited(m)
            | ProviderError::Unsupported(m)
            | ProviderError::Failed(m) => m,
        }
    }
}

pub trait OrchestratorProvider: Send + Sync {
    /// Identificador estável (ex.: `chatgpt_plan`). Aparece na interface.
    fn id(&self) -> &'static str;
    /// Estado REAL. A interface só diz "conectado" quando isto disser.
    fn status(&self) -> ProviderStatus;
    /// Um turno do modelo. Pode emitir eventos em streaming; devolve o resultado consolidado.
    fn complete(
        &self,
        req: &TurnRequest<'_>,
        sink: &mut dyn FnMut(TurnEvent),
    ) -> Result<TurnOutput, ProviderError>;
}

/// O único provedor registrado no app até a Etapa 3B: sempre desconectado, nunca responde.
pub struct DisconnectedProvider {
    reason: String,
}

impl DisconnectedProvider {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl OrchestratorProvider for DisconnectedProvider {
    fn id(&self) -> &'static str {
        "none"
    }
    fn status(&self) -> ProviderStatus {
        ProviderStatus::Disconnected {
            reason: self.reason.clone(),
        }
    }
    fn complete(
        &self,
        _req: &TurnRequest<'_>,
        _sink: &mut dyn FnMut(TurnEvent),
    ) -> Result<TurnOutput, ProviderError> {
        Err(ProviderError::NotConnected(self.reason.clone()))
    }
}

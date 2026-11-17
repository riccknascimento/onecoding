//! OneOrchestrator (Etapa 3A): contrato de provedor, catálogo tipado de ferramentas e laço de turno.
//! Nenhum provedor real existe ainda (a Etapa 3B traz o ChatGPT Plus); o app registra só o
//! `DisconnectedProvider`. O `ScriptedProvider` serve exclusivamente a testes.

pub mod provider;
pub mod scripted;
pub mod session;
pub mod tools;

pub use provider::{
    ChatItem, DisconnectedProvider, OrchestratorProvider, ProviderError, ToolCall, ToolSpec,
    TurnEvent, TurnOutput, TurnRequest,
};
pub use scripted::ScriptedProvider;
pub use session::{Orchestrator, OrchestratorError, OrchestratorLimits, DEFAULT_INSTRUCTIONS};
pub use tools::{catalog, ToolErrorKind, ToolOutcome, ToolPolicy, ToolRuntime};

//! Provedor SIMULADO, exclusivamente para testes. Não é registrado no app e nunca deve aparecer como
//! conectado para o usuário: responde apenas o roteiro que o teste definiu.

use std::sync::Mutex;

use one_protocol::ProviderStatus;

use crate::provider::{
    ChatItem, OrchestratorProvider, ProviderError, TurnEvent, TurnOutput, TurnRequest,
};

pub struct ScriptedProvider {
    steps: Mutex<Vec<Result<TurnOutput, ProviderError>>>,
    /// Histórico visto em cada chamada (para o teste verificar o que o provedor recebeu).
    pub seen: Mutex<Vec<Vec<ChatItem>>>,
    /// Instruções vistas na última chamada.
    pub last_instructions: Mutex<String>,
    /// Nomes das ferramentas oferecidas na última chamada.
    pub last_tools: Mutex<Vec<String>>,
}

impl ScriptedProvider {
    pub fn new(steps: Vec<Result<TurnOutput, ProviderError>>) -> Self {
        let mut s = steps;
        s.reverse(); // pop() devolve na ordem original
        Self {
            steps: Mutex::new(s),
            seen: Mutex::new(vec![]),
            last_instructions: Mutex::new(String::new()),
            last_tools: Mutex::new(vec![]),
        }
    }

    pub fn calls(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

impl OrchestratorProvider for ScriptedProvider {
    fn id(&self) -> &'static str {
        "scripted"
    }

    fn status(&self) -> ProviderStatus {
        ProviderStatus::Simulated {
            provider: "scripted".into(),
            label: "simulado (somente testes)".into(),
        }
    }

    fn complete(
        &self,
        req: &TurnRequest<'_>,
        sink: &mut dyn FnMut(TurnEvent),
    ) -> Result<TurnOutput, ProviderError> {
        self.seen.lock().unwrap().push(req.items.to_vec());
        *self.last_instructions.lock().unwrap() = req.instructions.to_string();
        *self.last_tools.lock().unwrap() = req.tools.iter().map(|t| t.name.to_string()).collect();
        let next = self.steps.lock().unwrap().pop();
        let out = next.unwrap_or_else(|| Err(ProviderError::Failed("roteiro esgotado".into())))?;
        if !out.text.is_empty() {
            sink(TurnEvent::TextDelta(out.text.clone()));
        }
        for c in &out.tool_calls {
            sink(TurnEvent::ToolCall(c.clone()));
        }
        Ok(out)
    }
}

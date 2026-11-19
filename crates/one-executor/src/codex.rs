//! Execução do Codex aprovado sobre um diretório isolado. Reutiliza o que o app já usa: aprovação e
//! identidade (`acquire`), argumentos com perfil de leitura confinada (`CodexAdapter`) e o parser de eventos.
//! O prompt segue por stdin, nunca como argumento.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use one_adapters::{
    acquire, CodexAdapter, Confinement, EngineAdapter, ExecRequest, Host, Limits, Outcome,
    StreamSpec, TrustStore,
};
use one_protocol::{EngineKind, EventPayload, ExecMode, TrustState};
use serde_json::{json, Value};

pub const CODEX_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_OUTPUT: usize = 4 * 1024 * 1024;

pub fn prompt_for(question: &str) -> String {
    format!(
        "Você está analisando um snapshot somente leitura de um projeto. Responda em português, somente com base nos \
         arquivos deste diretório, cite os arquivos e trechos que sustentam cada afirmação, e diga claramente o que \
         não foi possível verificar. Não modifique nenhum arquivo.\n\nPergunta: {question}"
    )
}

/// Executa o Codex aprovado no diretório. Retorna o texto final ou o motivo da falha (sem dados brutos).
/// `cancel` encerra o processo (grupo inteiro). `on_event` recebe só progresso real do Codex (ferramenta
/// iniciada e avisos), já redigidos pelo analisador; a resposta final não passa por ele.
pub fn run_codex(
    host: &dyn Host,
    trust: &dyn TrustStore,
    dir: &Path,
    question: &str,
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(&str, Value),
) -> Result<String, String> {
    run_codex_prompt(host, trust, dir, &prompt_for(question), cancel, on_event)
}

/// Mesma execução, com o prompt já montado (usado pelas delegações de OneAgent).
pub fn run_codex_prompt(
    host: &dyn Host,
    trust: &dyn TrustStore,
    dir: &Path,
    prompt: &str,
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(&str, Value),
) -> Result<String, String> {
    let exe = acquire(EngineKind::Codex, host, trust).map_err(|state| match state {
        TrustState::Changed => {
            "Codex mudou desde a aprovação: aprove de novo no OneAgent".to_string()
        }
        TrustState::Rejected => "Codex reprovado nas verificações de segurança".to_string(),
        _ => "Codex não aprovado neste Mac".to_string(),
    })?;
    let args = CodexAdapter
        .args(&ExecRequest {
            workspace: dir,
            confinement: Confinement::WorkspaceOnly,
            model: None,
            mode: ExecMode::ReadOnly,
            prompt,
        })
        .map_err(|e| e.to_string())?;
    let spec = StreamSpec {
        args,
        stdin: prompt.as_bytes().to_vec(),
        cwd: dir.to_path_buf(),
        limits: Limits {
            timeout: CODEX_TIMEOUT,
            max_output: MAX_OUTPUT,
        },
    };
    let mut parser = CodexAdapter.parser();
    let mut pending: Vec<u8> = Vec::new();
    let mut forward = |events: Vec<EventPayload>| {
        for ev in events {
            match ev {
                EventPayload::ToolStarted { summary, .. } => {
                    on_event("tool", json!({ "summary": summary }))
                }
                EventPayload::Warning { message } => {
                    on_event("warning", json!({ "message": message }))
                }
                _ => {}
            }
        }
    };
    let out = host.run_stream(&exe, &spec, Some(cancel), &mut |chunk| {
        pending.extend_from_slice(chunk);
        while let Some(pos) = pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = pending.drain(..=pos).collect();
            forward(parser.feed(&String::from_utf8_lossy(&line)));
        }
    });
    if !pending.is_empty() {
        forward(parser.feed(&String::from_utf8_lossy(&pending)));
    }
    let sum = parser.summary();
    if let Some(failure) = sum.failure {
        return Err(failure);
    }
    match (out.outcome, sum.final_text) {
        (Outcome::Exited(Some(0)), Some(text)) if !text.trim().is_empty() => Ok(text),
        (Outcome::Cancelled, _) => Err("cancelado".into()),
        (Outcome::TimedOut, _) => Err("o Codex excedeu o tempo limite".into()),
        (Outcome::OutputLimit, _) => Err("o Codex produziu saída acima do limite".into()),
        _ => Err("o Codex terminou sem resposta".into()),
    }
}

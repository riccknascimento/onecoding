//! Serviço de execução do Gateway local (Fase 2A).
//!
//! UI → comando Tauri → `ExecService::start` → thread própria → CLI aprovada → eventos no SQLite.
//! A UI só consulta o estado (polling); nada aqui depende da webview para andar.
//!
//! Garantias:
//! - só roda binário aprovado, com a identidade revalidada imediatamente antes do `exec`;
//! - só em workspace autorizado pelo usuário, em modo somente leitura;
//! - um workspace por vez; cancelamento, prazo e limite de saída; descendentes encerrados;
//! - estado final sempre gravado, inclusive em pânico (guarda) ou fechamento do app (reconcile).

pub mod workspace;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use one_adapters::proc::{Limits, Outcome};
use one_adapters::{
    acquire, CodexAdapter, Confinement, EngineAdapter, ExecRequest, Host, ParsedSummary,
    StreamSpec, TrustStore,
};
use one_protocol::{
    EngineKind, ErrorKind, EventPayload, ExecMode, RunStatus, StartRunRequest, TrustState,
};
use one_store::{FinishInfo, NewRun, Store};

pub const LOCAL_GATEWAY: &str = "local";
const MAX_PROMPT_BYTES: usize = 16 * 1024;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Aprovações de executáveis guardadas no SQLite.
pub struct StoreTrust(pub Arc<Mutex<Store>>);

fn engine_key(e: EngineKind) -> &'static str {
    match e {
        EngineKind::Codex => "codex",
        EngineKind::Claude => "claude",
        EngineKind::Cursor => "cursor",
    }
}

impl TrustStore for StoreTrust {
    fn approved_fingerprint(&self, engine: EngineKind) -> Option<String> {
        lock(&self.0)
            .trusted_get(engine_key(engine))
            .ok()
            .flatten()
            .map(|(fp, _)| fp)
    }
    fn approve(
        &self,
        engine: EngineKind,
        fingerprint: &str,
        canonical_path: &str,
    ) -> Result<(), String> {
        lock(&self.0)
            .trusted_put(engine_key(engine), fingerprint, canonical_path)
            .map_err(|e| e.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartError {
    pub kind: ErrorKind,
    pub message: String,
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

fn err(kind: ErrorKind, message: impl Into<String>) -> StartError {
    StartError {
        kind,
        message: message.into(),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ExecLimits {
    pub timeout: Duration,
    pub max_output: usize,
}

impl Default for ExecLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10 * 60),
            max_output: 8 * 1024 * 1024,
        }
    }
}

pub struct ExecService {
    store: Arc<Mutex<Store>>,
    host: Arc<dyn Host + Send>,
    adapters: Vec<Box<dyn EngineAdapter>>,
    limits: ExecLimits,
    /// workspace_id → run_id da execução ativa (uma por workspace).
    active: Mutex<HashMap<String, String>>,
    cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl ExecService {
    pub fn new(
        store: Arc<Mutex<Store>>,
        host: Arc<dyn Host + Send>,
        limits: ExecLimits,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            host,
            adapters: vec![Box::new(CodexAdapter)],
            limits,
            active: Mutex::new(HashMap::new()),
            cancels: Mutex::new(HashMap::new()),
        })
    }

    fn adapter(&self, e: EngineKind) -> Option<&dyn EngineAdapter> {
        self.adapters
            .iter()
            .find(|a| a.engine() == e)
            .map(|a| a.as_ref())
    }

    fn audit(&self, action: &str, subject: &str, outcome: &str) {
        let _ = lock(&self.store).audit_append("user", action, subject, outcome);
    }

    /// Valida, reserva o workspace, cria a execução e a inicia em segundo plano. Devolve o `run_id`.
    pub fn start(self: &Arc<Self>, req: &StartRunRequest) -> Result<String, StartError> {
        let blocked = |e: StartError| {
            self.audit(
                "run_blocked",
                &format!("{}:{:?}", req.workspace_id, e.kind),
                "blocked",
            );
            e
        };
        let adapter = self.adapter(req.engine).ok_or_else(|| {
            blocked(err(
                ErrorKind::InvalidRequest,
                "este motor ainda não é suportado",
            ))
        })?;
        let prompt = req.prompt.trim();
        if prompt.is_empty() || prompt.len() > MAX_PROMPT_BYTES {
            return Err(blocked(err(
                ErrorKind::InvalidRequest,
                "prompt vazio ou acima de 16 KB",
            )));
        }
        if let Some(m) = &req.model {
            if !one_adapters::engine::valid_model(m) {
                return Err(blocked(err(
                    ErrorKind::InvalidRequest,
                    "nome de modelo inválido",
                )));
            }
        }
        // Workspace: precisa ter sido autorizado pelo usuário e continuar sendo o mesmo diretório.
        // O guard do Mutex precisa morrer antes de `blocked` (que audita e trava de novo).
        let found = lock(&self.store).workspace_get(&req.workspace_id);
        let ws = found
            .map_err(|e| err(ErrorKind::InvalidRequest, e.to_string()))?
            .filter(|w| w.gateway_id == LOCAL_GATEWAY)
            .ok_or_else(|| {
                blocked(err(
                    ErrorKind::WorkspaceNotAuthorized,
                    "workspace não autorizado",
                ))
            })?;
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let canonical = workspace::validate(Path::new(&ws.path), home.as_deref())
            .map_err(|m| blocked(err(ErrorKind::WorkspaceNotAuthorized, m)))?;
        if canonical != Path::new(&ws.path) {
            return Err(blocked(err(
                ErrorKind::WorkspaceNotAuthorized,
                "o caminho do workspace mudou desde a autorização",
            )));
        }
        let ws_fsid = dir_id(&canonical);
        if ws_fsid.is_none() {
            return Err(blocked(err(
                ErrorKind::WorkspaceNotAuthorized,
                "não foi possível identificar a pasta do workspace",
            )));
        }
        // Checagem antecipada de confiança (a decisiva é a de antes do exec).
        let trust = StoreTrust(self.store.clone());
        if let Err(state) = acquire(req.engine, self.host.as_ref(), &trust) {
            return Err(blocked(err(
                ErrorKind::NotApproved,
                not_approved_message(state),
            )));
        }
        // Valida os argumentos agora para recusar cedo (modelo, modo).
        adapter
            .args(&ExecRequest {
                workspace: &canonical,
                confinement: Confinement::WorkspaceOnly,
                model: req.model.as_deref(),
                mode: ExecMode::ReadOnly,
                prompt,
            })
            .map_err(|m| blocked(err(ErrorKind::InvalidRequest, m)))?;

        // Uma execução por workspace.
        let run_id = {
            let mut active = lock(&self.active);
            if active.contains_key(&ws.id) {
                return Err(blocked(err(
                    ErrorKind::WorkspaceBusy,
                    "já existe uma execução ativa neste workspace",
                )));
            }
            let id = lock(&self.store)
                .run_create(&NewRun {
                    workspace_id: &ws.id,
                    engine: engine_key(req.engine),
                    model: req.model.as_deref(),
                    mode: ExecMode::ReadOnly,
                    prompt,
                })
                .map_err(|e| err(ErrorKind::InvalidRequest, e.to_string()))?;
            active.insert(ws.id.clone(), id.clone());
            id
        };
        let cancel = Arc::new(AtomicBool::new(false));
        lock(&self.cancels).insert(run_id.clone(), cancel.clone());
        self.audit(
            "run_started",
            &format!("{run_id}:{}", engine_key(req.engine)),
            "started",
        );

        let svc = Arc::clone(self);
        let (rid, ws_id, engine, model, prompt) = (
            run_id.clone(),
            ws.id.clone(),
            req.engine,
            req.model.clone(),
            prompt.to_string(),
        );
        let spawned = std::thread::Builder::new()
            .name(format!("run-{rid}"))
            .spawn(move || {
                svc.execute(
                    &rid,
                    &ws_id,
                    &canonical,
                    ws_fsid,
                    engine,
                    model.as_deref(),
                    &prompt,
                    &cancel,
                )
            });
        if let Err(e) = spawned {
            // A thread não existe: sem isto a execução ficaria `queued` e o workspace ocupado.
            self.abort_start(
                &run_id,
                &ws.id,
                "não foi possível criar a thread de execução",
            );
            return Err(err(ErrorKind::SpawnFailed, e.to_string()));
        }
        Ok(run_id)
    }

    /// Desfaz o registro de uma execução que não chegou a iniciar: finaliza como falha e libera tudo.
    fn abort_start(&self, run_id: &str, ws_id: &str, message: &str) {
        self.emit(
            run_id,
            EventPayload::Error {
                kind: ErrorKind::SpawnFailed,
                message: message.into(),
            },
        );
        self.emit(
            run_id,
            EventPayload::Status {
                status: RunStatus::Failed,
            },
        );
        let _ = lock(&self.store).run_finish(
            run_id,
            &FinishInfo {
                status: Some(RunStatus::Failed),
                error_kind: Some(ErrorKind::SpawnFailed),
                error_message: Some(message.into()),
                ..Default::default()
            },
        );
        lock(&self.active).remove(ws_id);
        lock(&self.cancels).remove(run_id);
    }

    /// Pede o cancelamento. `false` se a execução não está ativa.
    pub fn cancel(&self, run_id: &str) -> bool {
        match lock(&self.cancels).get(run_id) {
            Some(flag) => {
                flag.store(true, Ordering::SeqCst);
                self.audit("run_cancel", run_id, "requested");
                true
            }
            None => false,
        }
    }

    pub fn is_active(&self, run_id: &str) -> bool {
        lock(&self.cancels).contains_key(run_id)
    }

    fn emit(&self, run_id: &str, p: EventPayload) {
        let _ = lock(&self.store).event_append(run_id, &p);
    }

    #[allow(clippy::too_many_arguments)]
    fn execute(
        self: &Arc<Self>,
        run_id: &str,
        ws_id: &str,
        workspace: &Path,
        ws_fsid: Option<(u64, u64)>,
        engine: EngineKind,
        model: Option<&str>,
        prompt: &str,
        cancel: &AtomicBool,
    ) {
        let mut guard = Guard {
            svc: self,
            run_id,
            ws_id,
            done: false,
        };
        let started = Instant::now();
        let _ = lock(&self.store).run_mark_running(run_id);
        self.emit(
            run_id,
            EventPayload::Status {
                status: RunStatus::Running,
            },
        );

        let finish = |status: RunStatus,
                      kind: Option<ErrorKind>,
                      msg: Option<String>,
                      code: Option<i32>,
                      sum: &ParsedSummary| {
            if let (Some(k), Some(m)) = (kind, &msg) {
                if !matches!(status, RunStatus::Succeeded) {
                    // evita duplicar o evento de erro que o próprio motor já emitiu
                    let already = lock(&self.store)
                        .events_after(run_id, 0)
                        .map(|e| {
                            e.iter()
                                .any(|x| matches!(x.payload, EventPayload::Error { .. }))
                        })
                        .unwrap_or(false);
                    if !already || k != ErrorKind::ExecutorFailed {
                        self.emit(
                            run_id,
                            EventPayload::Error {
                                kind: k,
                                message: m.clone(),
                            },
                        );
                    }
                }
            }
            self.emit(run_id, EventPayload::Status { status });
            let _ = lock(&self.store).run_finish(
                run_id,
                &FinishInfo {
                    status: Some(status),
                    exit_code: code,
                    error_kind: kind,
                    error_message: msg,
                    duration_ms: Some(started.elapsed().as_millis().min(u32::MAX as u128) as u32),
                    usage: sum.usage.clone(),
                    tool_calls: sum.tool_calls,
                    engine_session_id: sum.session_id.clone(),
                    model_reported: sum.model_reported.clone(),
                    final_text: sum.final_text.clone(),
                },
            );
        };

        let adapter = self.adapter(engine).expect("adaptador validado no start");
        let args = match adapter.args(&ExecRequest {
            workspace,
            // leitura confinada ao workspace; o modo de leitura ampla não é exposto pela UI
            confinement: Confinement::WorkspaceOnly,
            model,
            mode: ExecMode::ReadOnly,
            prompt,
        }) {
            Ok(a) => a,
            Err(m) => {
                finish(
                    RunStatus::Failed,
                    Some(ErrorKind::InvalidRequest),
                    Some(m),
                    None,
                    &ParsedSummary::default(),
                );
                guard.done = true;
                return;
            }
        };
        // O workspace também é revalidado agora: precisa continuar sendo exatamente o diretório autorizado.
        if !workspace_unchanged(workspace, ws_fsid) {
            finish(
                RunStatus::Failed,
                Some(ErrorKind::WorkspaceNotAuthorized),
                Some("o workspace mudou desde a autorização".into()),
                None,
                &ParsedSummary::default(),
            );
            guard.done = true;
            return;
        }
        // Revalidação IMEDIATAMENTE antes do exec: a identidade precisa ser a aprovada, agora.
        let exe = match acquire(engine, self.host.as_ref(), &StoreTrust(self.store.clone())) {
            Ok(e) => e,
            Err(state) => {
                finish(
                    RunStatus::Failed,
                    Some(ErrorKind::NotApproved),
                    Some(not_approved_message(state)),
                    None,
                    &ParsedSummary::default(),
                );
                guard.done = true;
                return;
            }
        };
        let spec = StreamSpec {
            args,
            stdin: prompt.as_bytes().to_vec(),
            cwd: workspace.to_path_buf(),
            limits: Limits {
                timeout: self.limits.timeout,
                max_output: self.limits.max_output,
            },
        };
        let mut parser = adapter.parser();
        let mut pending: Vec<u8> = Vec::new();
        let mut handle_line = |line: &str| {
            for ev in parser.feed(line) {
                self.emit(run_id, ev);
            }
        };
        let out = self
            .host
            .run_stream(&exe, &spec, Some(cancel), &mut |chunk| {
                pending.extend_from_slice(chunk);
                while let Some(pos) = pending.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = pending.drain(..=pos).collect();
                    handle_line(&String::from_utf8_lossy(&line));
                }
            });
        if !pending.is_empty() {
            handle_line(&String::from_utf8_lossy(&pending));
        }
        let sum = parser.summary();

        let (status, kind, msg, code) = match out.outcome {
            Outcome::Exited(Some(0))
                if sum.failure.is_none() && (sum.turn_completed || sum.final_text.is_some()) =>
            {
                (RunStatus::Succeeded, None, None, Some(0))
            }
            Outcome::Exited(Some(0)) => (
                RunStatus::Failed,
                Some(ErrorKind::ExecutorFailed),
                Some(
                    sum.failure
                        .clone()
                        .unwrap_or_else(|| "o motor terminou sem resposta".into()),
                ),
                Some(0),
            ),
            Outcome::Exited(code) => (
                RunStatus::Failed,
                Some(ErrorKind::ExecutorFailed),
                Some(sum.failure.clone().unwrap_or_else(|| {
                    format!(
                        "o motor terminou com falha (código {})",
                        code.map_or("sinal".into(), |c| c.to_string())
                    )
                })),
                code,
            ),
            Outcome::TimedOut => (
                RunStatus::Failed,
                Some(ErrorKind::Timeout),
                Some("tempo limite excedido; a execução foi encerrada".into()),
                None,
            ),
            Outcome::Cancelled => (
                RunStatus::Cancelled,
                Some(ErrorKind::Cancelled),
                Some("cancelada pelo usuário".into()),
                None,
            ),
            Outcome::OutputLimit => (
                RunStatus::Failed,
                Some(ErrorKind::OutputLimit),
                Some("limite de saída excedido; a execução foi encerrada".into()),
                None,
            ),
            Outcome::SpawnFailed => (
                RunStatus::Failed,
                Some(ErrorKind::SpawnFailed),
                Some("não foi possível iniciar o motor".into()),
                None,
            ),
        };
        finish(status, kind, msg, code, &sum);
        guard.done = true;
    }
}

/// Identidade do diretório no disco (dispositivo + inode): distingue "a mesma pasta" de "outra pasta
/// criada no mesmo caminho".
fn dir_id(p: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).ok().map(|m| (m.dev(), m.ino()))
}

/// O caminho (já canônico) continua sendo uma pasta válida e canônica de si mesma: nenhum componente
/// foi trocado por symlink nem a pasta foi movida.
/// e é o MESMO diretório (dispositivo + inode) que foi verificado no `start`.
fn workspace_unchanged(ws: &Path, expected: Option<(u64, u64)>) -> bool {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    expected.is_some()
        && dir_id(ws) == expected
        && workspace::validate(ws, home.as_deref()).is_ok_and(|c| c == ws)
}

fn not_approved_message(state: TrustState) -> String {
    match state {
        TrustState::Changed => {
            "o executável mudou desde a aprovação; valide de novo em Agent Gateways"
        }
        TrustState::Rejected => "o executável foi reprovado nas verificações de segurança",
        _ => "o executável ainda não foi aprovado; aprove em Agent Gateways",
    }
    .into()
}

/// Libera o workspace e a trava de cancelamento sempre; se a thread terminar sem estado final
/// (pânico), marca a execução como interrompida.
struct Guard<'a> {
    svc: &'a Arc<ExecService>,
    run_id: &'a str,
    ws_id: &'a str,
    done: bool,
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        if !self.done {
            let _ = lock(&self.svc.store).run_finish(
                self.run_id,
                &FinishInfo {
                    status: Some(RunStatus::Failed),
                    error_kind: Some(ErrorKind::Interrupted),
                    error_message: Some("a execução terminou de forma inesperada".into()),
                    ..Default::default()
                },
            );
        }
        lock(&self.svc.active).remove(self.ws_id);
        lock(&self.svc.cancels).remove(self.run_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_adapters::SystemHost;

    fn service() -> (Arc<ExecService>, Arc<Mutex<Store>>) {
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        lock(&store).ensure_local_gateway().unwrap();
        (
            ExecService::new(
                store.clone(),
                Arc::new(SystemHost::new()),
                ExecLimits::default(),
            ),
            store,
        )
    }

    /// Falha ao criar a thread: a execução vira `failed/spawn_failed` e o workspace é liberado.
    #[test]
    fn abort_start_finalizes_the_run_and_frees_the_workspace() {
        let (svc, store) = service();
        let ws = lock(&store)
            .workspace_add(LOCAL_GATEWAY, "/tmp/p", "p")
            .unwrap();
        let run = lock(&store)
            .run_create(&NewRun {
                workspace_id: &ws.id,
                engine: "codex",
                model: None,
                mode: ExecMode::ReadOnly,
                prompt: "x",
            })
            .unwrap();
        lock(&svc.active).insert(ws.id.clone(), run.clone());
        lock(&svc.cancels).insert(run.clone(), Arc::new(AtomicBool::new(false)));

        svc.abort_start(&run, &ws.id, "sem recursos");

        let v = lock(&store).run_get(&run).unwrap().unwrap();
        assert_eq!(
            (v.status, v.error_kind),
            (RunStatus::Failed, Some(ErrorKind::SpawnFailed))
        );
        assert!(
            lock(&svc.active).is_empty() && lock(&svc.cancels).is_empty(),
            "nada preso"
        );
        let ev = lock(&store).events_after(&run, 0).unwrap();
        assert!(matches!(
            ev.last().unwrap().payload,
            EventPayload::Status {
                status: RunStatus::Failed
            }
        ));
    }

    #[test]
    fn workspace_swapped_for_a_symlink_or_moved_is_detected() {
        let d = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("oa-wsu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (ws, other) = (d.join("ws"), d.join("outro"));
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let id = dir_id(&ws);
        assert!(workspace_unchanged(&ws, id));
        assert!(
            !workspace_unchanged(&ws, None),
            "sem identidade capturada não passa"
        );
        // OUTRA pasta criada no mesmo caminho (mesma string, inode diferente)
        std::fs::remove_dir_all(&ws).unwrap();
        std::fs::create_dir_all(&ws).unwrap();
        assert!(
            !workspace_unchanged(&ws, id),
            "pasta recriada no mesmo caminho"
        );
        // symlink no lugar da pasta autorizada
        let id = dir_id(&ws);
        std::fs::remove_dir_all(&ws).unwrap();
        std::os::unix::fs::symlink(&other, &ws).unwrap();
        assert!(
            !workspace_unchanged(&ws, id),
            "symlink no lugar da pasta autorizada"
        );
        std::fs::remove_file(&ws).unwrap();
        assert!(!workspace_unchanged(&ws, id), "pasta removida");
        let _ = std::fs::remove_dir_all(&d);
    }
}

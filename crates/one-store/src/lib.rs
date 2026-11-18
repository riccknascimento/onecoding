//! Persistência local (SQLite + FTS5) do OneAgent.
//!
//! Fase 1: conexão, migrações versionadas e verificação de saúde. As operações de domínio
//! (conversas, tarefas, OneContext) entram nas fases seguintes sobre este esquema.

use std::path::{Path, PathBuf};

use one_protocol::StoreStatus;
use rusqlite::Connection;

mod migrations;
mod runs;

pub use runs::{FinishInfo, NewRun};

pub use migrations::LATEST_VERSION;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("banco com versão {found} mais nova que o app suporta ({supported})")]
    SchemaTooNew { found: u32, supported: u32 },
    #[error("integridade do banco: {0}")]
    Integrity(String),
    #[error("migração para a versão {version} falhou: {source}")]
    Migration {
        version: u32,
        source: Box<StoreError>,
    },
    #[error("SQLite não aplicou foreign_keys=ON; recusando abrir o banco")]
    ForeignKeysDisabled,
}

pub type Result<T> = std::result::Result<T, StoreError>;

pub struct Store {
    conn: Connection,
}

/// Antes de migrar um banco existente, guarda uma cópia consistente (`VACUUM INTO`); nunca apaga.
fn backup_before_upgrade(conn: &Connection, path: &Path) -> Result<()> {
    let v = migrations::user_version(conn)?;
    if v == 0 || v >= LATEST_VERSION || path == Path::new(":memory:") {
        return Ok(());
    }
    let bak = PathBuf::from(format!("{}.pre-v{}.bak", path.display(), v + 1));
    if !bak.exists() {
        conn.execute("VACUUM INTO ?1", [bak.display().to_string()])?;
    }
    Ok(())
}

/// Resolve o caminho do banco: `ONEAGENT_DATA_DIR` (testes/dev isolado) tem prioridade sobre o
/// diretório de dados do app.
pub fn db_path(env_override: Option<&std::ffi::OsStr>, app_data_dir: &Path) -> PathBuf {
    let base = env_override
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| app_data_dir.to_path_buf());
    base.join("oneagent.db")
}

impl Store {
    /// Abre (criando, se preciso) o banco em `path` e aplica as migrações pendentes.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn, path)
    }

    /// Banco em memória (testes).
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?, Path::new(":memory:"))
    }

    fn init(conn: Connection, path: &Path) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let fk: i64 = conn.pragma_query_value(None, "foreign_keys", |r| r.get(0))?;
        if fk != 1 {
            return Err(StoreError::ForeignKeysDisabled);
        }
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        backup_before_upgrade(&conn, path)?;
        migrations::apply(&conn)?;
        Ok(Self { conn })
    }

    /// Identidade aprovada da CLI: `(fingerprint, canonical_path)`.
    pub fn trusted_get(&self, engine: &str) -> Result<Option<(String, String)>> {
        let mut st = self.conn.prepare(
            "SELECT fingerprint, canonical_path FROM trusted_executables WHERE engine = ?1",
        )?;
        let mut rows = st.query([engine])?;
        Ok(match rows.next()? {
            Some(r) => Some((r.get(0)?, r.get(1)?)),
            None => None,
        })
    }

    /// Registra (ou substitui) a aprovação e grava na trilha de auditoria, na mesma transação.
    pub fn trusted_put(&self, engine: &str, fingerprint: &str, canonical_path: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO trusted_executables(engine, fingerprint, canonical_path) VALUES (?1, ?2, ?3)
             ON CONFLICT(engine) DO UPDATE SET fingerprint = ?2, canonical_path = ?3,
               approved_at = strftime('%Y-%m-%dT%H:%M:%SZ','now')",
            [engine, fingerprint, canonical_path],
        )?;
        tx.execute(
            "INSERT INTO audit_log(actor, action, subject, outcome) VALUES ('user', 'approve_executable', ?1, 'approved')",
            [format!("{engine}:{fingerprint}")],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn schema_version(&self) -> Result<u32> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?)
    }

    /// `true` se o SQLite embutido tem FTS5 e a tabela de busca do OneContext existe.
    pub fn fts5_available(&self) -> bool {
        self.conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'context_chunks_fts'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n == 1)
            .unwrap_or(false)
    }

    pub fn integrity_ok(&self) -> bool {
        self.conn
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .map(|s| s == "ok")
            .unwrap_or(false)
    }

    pub fn status(&self) -> Result<StoreStatus> {
        Ok(StoreStatus {
            schema_version: self.schema_version()?,
            fts5: self.fts5_available(),
            integrity_ok: self.integrity_ok(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("oneagent-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("nested").join("one.db")
    }

    #[test]
    fn env_override_wins_for_db_path() {
        let app = Path::new("/app/data");
        assert_eq!(db_path(None, app), PathBuf::from("/app/data/oneagent.db"));
        assert_eq!(
            db_path(Some("".as_ref()), app),
            PathBuf::from("/app/data/oneagent.db")
        );
        assert_eq!(
            db_path(Some("/tmp/x".as_ref()), app),
            PathBuf::from("/tmp/x/oneagent.db")
        );
    }

    #[test]
    fn creates_schema_and_reports_status() {
        let s = Store::open_in_memory().unwrap();
        let st = s.status().unwrap();
        assert_eq!(st.schema_version, LATEST_VERSION);
        assert!(st.fts5, "FTS5 deve estar disponível no SQLite embutido");
        assert!(st.integrity_ok);
    }

    #[test]
    fn open_is_idempotent_and_creates_parent_dirs() {
        let p = temp_db("idem");
        {
            let s = Store::open(&p).unwrap();
            s.conn()
                .execute(
                    "INSERT INTO gateways(id,name,kind) VALUES('g1','Mac','local')",
                    [],
                )
                .unwrap();
        }
        let s = Store::open(&p).unwrap(); // reabre: não reaplica nem perde dados
        let n: i64 = s
            .conn()
            .query_row("SELECT count(*) FROM gateways", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(s.schema_version().unwrap(), LATEST_VERSION);
        let _ = std::fs::remove_dir_all(p.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn rejects_database_newer_than_app() {
        let p = temp_db("newer");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        Connection::open(&p)
            .unwrap()
            .pragma_update(None, "user_version", LATEST_VERSION + 1)
            .unwrap();
        assert!(matches!(
            Store::open(&p),
            Err(StoreError::SchemaTooNew { .. })
        ));
        let _ = std::fs::remove_dir_all(p.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn fts5_search_over_context_chunks_with_provenance() {
        let s = Store::open_in_memory().unwrap();
        let c = s.conn();
        c.execute(
            "INSERT INTO projects(id,name,gateway_id,path) VALUES('p1','demo','g1','/x')",
            [],
        )
        .unwrap_err(); // FK: gateway inexistente deve falhar
        c.execute(
            "INSERT INTO gateways(id,name,kind) VALUES('g1','Mac','local')",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO projects(id,name,gateway_id,path) VALUES('p1','demo','g1','/x')",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO context_sources(id,project_id,origin,ref,content_hash) VALUES('s1','p1','file','README.md','abc')",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO context_chunks(id,source_id,module,body,status) VALUES('c1','s1','core','O gateway executa tarefas em diretórios autorizados','validated')",
            [],
        )
        .unwrap();
        let hit: String = c
            .query_row(
                "SELECT ch.id FROM context_chunks_fts f JOIN context_chunks ch ON ch.rowid = f.rowid WHERE context_chunks_fts MATCH 'autorizados'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hit, "c1");
        c.execute("DELETE FROM context_chunks WHERE id='c1'", [])
            .unwrap();
        let n: i64 = c
            .query_row("SELECT count(*) FROM context_chunks_fts WHERE context_chunks_fts MATCH 'autorizados'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "índice FTS acompanha o delete");
    }

    #[test]
    fn rejects_invalid_enum_values() {
        let s = Store::open_in_memory().unwrap();
        assert!(s
            .conn()
            .execute(
                "INSERT INTO gateways(id,name,kind) VALUES('g','x','weird')",
                []
            )
            .is_err());
    }

    fn v1_db(name: &str) -> PathBuf {
        let p = temp_db(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let c = Connection::open(&p).unwrap();
        c.pragma_update(None, "foreign_keys", "ON").unwrap();
        migrations::apply_with(&c, &migrations::MIGRATIONS[..1]).unwrap();
        c.execute_batch(
            "INSERT INTO gateways(id,name,kind) VALUES('gA','A','local'),('gB','B','remote');
             INSERT INTO projects(id,name,gateway_id,path) VALUES('pA','pa','gA','/a'),('pB','pb','gB','/b');
             INSERT INTO tasks(id,gateway_id,project_id,engine,mode,prompt) VALUES('t1','gA','pA','codex','read_only','x');
             INSERT INTO runs(id,task_id,status) VALUES('r1','t1','queued');",
        )
        .unwrap();
        p
    }

    #[test]
    fn upgrade_v1_to_v2_preserves_data_and_isolates_gateways() {
        let p = v1_db("upgrade");
        assert_eq!(
            Connection::open(&p)
                .unwrap()
                .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                .unwrap(),
            1
        );

        let s = Store::open(&p).unwrap();
        assert_eq!(s.schema_version().unwrap(), LATEST_VERSION);
        let c = s.conn();
        // dados preservados, inclusive a referência de runs -> tasks
        let n: i64 = c
            .query_row(
                "SELECT count(*) FROM tasks t JOIN runs r ON r.task_id = t.id WHERE t.id='t1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
        assert!(s.integrity_ok());
        let fk: i64 = c
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1, "foreign_keys restaurado após a migração");
        // par válido aceito, par Gateway A + projeto de B rejeitado
        c.execute("INSERT INTO tasks(id,gateway_id,project_id,engine,mode,prompt) VALUES('t2','gB','pB','claude','read_only','y')", []).unwrap();
        let bad = c.execute("INSERT INTO tasks(id,gateway_id,project_id,engine,mode,prompt) VALUES('t3','gA','pB','claude','read_only','z')", []);
        assert!(
            bad.is_err(),
            "tarefa do gateway A com projeto do gateway B deve ser rejeitada"
        );
        assert!(c.execute("INSERT INTO tasks(id,gateway_id,project_id,engine,mode,prompt) VALUES('t4','gA','inexistente','codex','read_only','w')", []).is_err());
        // backup consistente criado, e o banco original não foi apagado
        let bak = PathBuf::from(format!("{}.pre-v2.bak", p.display()));
        assert!(bak.exists());
        assert_eq!(
            Connection::open(&bak)
                .unwrap()
                .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                .unwrap(),
            1
        );
        drop(s);
        // idempotente: reabrir não reaplica
        assert_eq!(
            Store::open(&p).unwrap().schema_version().unwrap(),
            LATEST_VERSION
        );
        let _ = std::fs::remove_dir_all(p.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn upgrade_aborts_without_deleting_data_when_existing_rows_violate_v2() {
        let p = v1_db("violates");
        {
            // dado legado inconsistente (possível só em V1): tarefa do gateway A com projeto de B
            let c = Connection::open(&p).unwrap();
            c.execute("INSERT INTO tasks(id,gateway_id,project_id,engine,mode,prompt) VALUES('bad','gA','pB','codex','read_only','x')", []).unwrap();
        }
        let err = Store::open(&p).err().expect("deve falhar");
        assert!(
            matches!(err, StoreError::Migration { version: 2, .. }),
            "{err}"
        );
        let c = Connection::open(&p).unwrap();
        let v: u32 = c
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, 1, "rollback: versão inalterada");
        let n: i64 = c
            .query_row("SELECT count(*) FROM tasks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2, "nenhum dado apagado");
        let _ = std::fs::remove_dir_all(p.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn failing_migration_rolls_back_and_restores_foreign_keys() {
        let c = Connection::open_in_memory().unwrap();
        c.pragma_update(None, "foreign_keys", "ON").unwrap();
        let ms = [
            migrations::Migration {
                sql: "CREATE TABLE a(x INTEGER);",
                fk_off: false,
            },
            migrations::Migration {
                sql: "CREATE TABLE b(x INTEGER); INSERT INTO nao_existe VALUES(1);",
                fk_off: true,
            },
        ];
        let err = migrations::apply_with(&c, &ms).unwrap_err();
        assert!(matches!(err, StoreError::Migration { version: 2, .. }));
        assert_eq!(migrations::user_version(&c).unwrap(), 1);
        let b: i64 = c
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='b'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(b, 0, "tabela da migração falha não persiste");
        let fk: i64 = c
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1);
    }

    #[test]
    fn trusted_executable_roundtrip_is_audited() {
        let s = Store::open_in_memory().unwrap();
        assert_eq!(s.trusted_get("codex").unwrap(), None);
        s.trusted_put("codex", "aaa", "/x/codex").unwrap();
        s.trusted_put("codex", "bbb", "/y/codex").unwrap(); // reaprovação substitui
        assert_eq!(
            s.trusted_get("codex").unwrap(),
            Some(("bbb".into(), "/y/codex".into()))
        );
        let n: i64 = s
            .conn()
            .query_row(
                "SELECT count(*) FROM audit_log WHERE action='approve_executable'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 2);
        assert!(
            s.trusted_put("gemini", "a", "/p").is_err(),
            "CHECK de motor"
        );
    }

    #[test]
    fn audit_log_is_append_only() {
        let s = Store::open_in_memory().unwrap();
        s.trusted_put("codex", "a", "/p").unwrap();
        assert!(s
            .conn()
            .execute("UPDATE audit_log SET outcome='x'", [])
            .is_err());
        assert!(s.conn().execute("DELETE FROM audit_log", []).is_err());
    }

    #[test]
    fn run_lifecycle_persists_events_usage_and_survives_reopen() {
        use one_protocol::{ErrorKind, EventPayload, ExecMode, RunStatus, UsageInfo};
        let p = temp_db("runs");
        let run_id = {
            let s = Store::open(&p).unwrap();
            s.ensure_local_gateway().unwrap();
            let ws = s.workspace_add("local", "/tmp/proj", "proj").unwrap();
            assert_eq!(
                s.workspace_add("local", "/tmp/proj", "outro").unwrap().id,
                ws.id,
                "idempotente por caminho"
            );
            let id = s
                .run_create(&NewRun {
                    workspace_id: &ws.id,
                    engine: "codex",
                    model: None,
                    mode: ExecMode::ReadOnly,
                    prompt: "oi",
                })
                .unwrap();
            s.run_mark_running(&id).unwrap();
            let e1 = s
                .event_append(
                    &id,
                    &EventPayload::Message {
                        text: "olá".into()
                    },
                )
                .unwrap()
                .unwrap();
            let e2 = s
                .event_append(
                    &id,
                    &EventPayload::Status {
                        status: RunStatus::Running,
                    },
                )
                .unwrap()
                .unwrap();
            assert_eq!((e1.seq, e2.seq), (1, 2));
            s.run_finish(
                &id,
                &FinishInfo {
                    status: Some(RunStatus::Succeeded),
                    exit_code: Some(0),
                    duration_ms: Some(1234),
                    usage: UsageInfo {
                        input_tokens: Some(10),
                        output_tokens: Some(2),
                        ..Default::default()
                    },
                    tool_calls: 3,
                    engine_session_id: Some("th1".into()),
                    final_text: Some("pronto".into()),
                    ..Default::default()
                },
            )
            .unwrap();
            // um estado final não é sobrescrito
            s.run_finish(
                &id,
                &FinishInfo {
                    status: Some(RunStatus::Failed),
                    error_kind: Some(ErrorKind::Timeout),
                    ..Default::default()
                },
            )
            .unwrap();
            id
        };
        let s = Store::open(&p).unwrap(); // "reinício do app"
        let v = s.run_get(&run_id).unwrap().unwrap();
        assert_eq!(v.status, RunStatus::Succeeded);
        assert_eq!(
            (
                v.usage.input_tokens,
                v.usage.cached_tokens,
                v.tool_calls,
                v.duration_ms
            ),
            (Some(10), None, 3, Some(1234))
        );
        assert_eq!(v.model_reported, None, "modelo ausente continua ausente");
        assert_eq!(v.engine_session_id.as_deref(), Some("th1"));
        assert_eq!(s.events_after(&run_id, 0).unwrap().len(), 2);
        assert_eq!(s.events_after(&run_id, 1).unwrap()[0].seq, 2);
        assert_eq!(s.runs_list(10).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(p.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn reconcile_marks_active_runs_as_interrupted_and_keeps_finished_ones() {
        use one_protocol::{ErrorKind, ExecMode, RunStatus};
        let s = Store::open_in_memory().unwrap();
        s.ensure_local_gateway().unwrap();
        let ws = s.workspace_add("local", "/tmp/p", "p").unwrap();
        let mk = || {
            s.run_create(&NewRun {
                workspace_id: &ws.id,
                engine: "codex",
                model: Some("m"),
                mode: ExecMode::ReadOnly,
                prompt: "x",
            })
            .unwrap()
        };
        let (a, b) = (mk(), mk());
        s.run_mark_running(&a).unwrap();
        s.run_finish(
            &b,
            &FinishInfo {
                status: Some(RunStatus::Succeeded),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(s.reconcile_interrupted().unwrap(), 1);
        let va = s.run_get(&a).unwrap().unwrap();
        assert_eq!(
            (va.status, va.error_kind),
            (RunStatus::Failed, Some(ErrorKind::Interrupted))
        );
        assert_eq!(s.run_get(&b).unwrap().unwrap().status, RunStatus::Succeeded);
    }

    #[test]
    fn event_limits_and_sql_safety() {
        use one_protocol::{EventPayload, ExecMode};
        let s = Store::open_in_memory().unwrap();
        s.ensure_local_gateway().unwrap();
        let ws = s
            .workspace_add("local", "/tmp/o'; DROP TABLE runs;--", "x\" OR 1=1")
            .unwrap();
        let id = s
            .run_create(&NewRun {
                workspace_id: &ws.id,
                engine: "codex",
                model: None,
                mode: ExecMode::ReadOnly,
                prompt: "'); DROP TABLE tasks;--",
            })
            .unwrap();
        assert!(
            s.run_get(&id).unwrap().is_some(),
            "entradas hostis são só dados"
        );
        assert!(
            s.event_append(
                &id,
                &EventPayload::Message {
                    text: "a".repeat(20_000)
                }
            )
            .is_err(),
            "payload grande é recusado"
        );
        for _ in 0..2000 {
            s.event_append(
                &id,
                &EventPayload::Warning {
                    message: "w".into(),
                },
            )
            .unwrap()
            .unwrap();
        }
        assert!(
            s.event_append(
                &id,
                &EventPayload::Warning {
                    message: "w".into()
                }
            )
            .unwrap()
            .is_none(),
            "limite por execução"
        );
    }

    #[test]
    fn upgrade_v2_to_v3_keeps_existing_runs() {
        let p = temp_db("v3up");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        {
            let c = Connection::open(&p).unwrap();
            c.pragma_update(None, "foreign_keys", "ON").unwrap();
            migrations::apply_with(&c, &migrations::MIGRATIONS[..2]).unwrap();
            c.execute_batch(
                "INSERT INTO gateways(id,name,kind) VALUES('g','G','local');
                 INSERT INTO projects(id,name,gateway_id,path) VALUES('p','p','g','/p');
                 INSERT INTO tasks(id,gateway_id,project_id,engine,mode,prompt) VALUES('t','g','p','codex','read_only','x');
                 INSERT INTO runs(id,task_id,status) VALUES('r','t','succeeded');",
            ).unwrap();
        }
        let s = Store::open(&p).unwrap();
        assert_eq!(s.schema_version().unwrap(), LATEST_VERSION);
        let v = s.run_get("r").unwrap().unwrap();
        assert_eq!((v.tool_calls, v.attempts, v.model_reported), (0, 1, None));
        assert!(PathBuf::from(format!("{}.pre-v3.bak", p.display())).exists());
        let _ = std::fs::remove_dir_all(p.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn orchestrator_conversations_messages_events_and_initiator() {
        use one_protocol::{ChatMode, ChatRole, ExecMode, RunInitiator};
        let s = Store::open_in_memory().unwrap();
        s.ensure_local_gateway().unwrap();
        let ws = s.workspace_add("local", "/tmp/p", "p").unwrap();
        let conv = s
            .conversation_create(ChatMode::Orchestrator, "investigar")
            .unwrap();
        assert_eq!(
            s.conversation_mode(&conv).unwrap(),
            Some(ChatMode::Orchestrator)
        );
        s.message_append(&conv, ChatRole::User, "oi").unwrap();
        s.message_append(&conv, ChatRole::Orchestrator, "olá")
            .unwrap();
        let m = s.messages_list(&conv).unwrap();
        assert_eq!(
            (m.len(), m[0].role, m[1].role),
            (2, ChatRole::User, ChatRole::Orchestrator)
        );
        s.orch_event_append(&conv, "tool_call", "{}").unwrap();
        s.orch_event_append(&conv, "tool_result", "{}").unwrap();
        assert_eq!(
            s.orch_events(&conv)
                .unwrap()
                .iter()
                .map(|e| e.0.as_str())
                .collect::<Vec<_>>(),
            ["tool_call", "tool_result"]
        );
        assert!(
            s.orch_event_append(&conv, "invalido", "{}").is_err(),
            "CHECK de tipo"
        );

        let direct = s
            .run_create(&NewRun {
                workspace_id: &ws.id,
                engine: "codex",
                model: None,
                mode: ExecMode::ReadOnly,
                prompt: "d",
            })
            .unwrap();
        let orch = s
            .run_create_for(
                &NewRun {
                    workspace_id: &ws.id,
                    engine: "codex",
                    model: None,
                    mode: ExecMode::ReadOnly,
                    prompt: "o",
                },
                Some(&conv),
                RunInitiator::Orchestrator,
            )
            .unwrap();
        assert_eq!(
            s.run_get(&direct).unwrap().unwrap().initiator,
            RunInitiator::User
        );
        assert_eq!(
            s.run_get(&orch).unwrap().unwrap().initiator,
            RunInitiator::Orchestrator
        );
        assert_eq!(
            s.run_conversation(&orch).unwrap().as_deref(),
            Some(conv.as_str())
        );
        assert_ne!(
            s.run_conversation(&direct).unwrap().as_deref(),
            Some(conv.as_str()),
            "o direto tem a própria conversa"
        );
        assert_eq!(
            s.conversation_mode(&s.run_conversation(&direct).unwrap().unwrap())
                .unwrap(),
            Some(ChatMode::Direct)
        );
        assert_eq!(s.orchestrator_run_count(&conv).unwrap(), 1);
    }

    #[test]
    fn chat_history_survives_reopen_and_tracks_state_usage_and_isolation() {
        use one_protocol::{ChatMessageState, ChatRole, UsageInfo};
        let p = temp_db("chat-history");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let conv;
        let other;
        {
            let s = Store::open(&p).unwrap();
            conv = s.chat_conversation_create().unwrap();
            other = s.chat_conversation_create().unwrap();
            s.chat_message_insert(
                &conv,
                ChatRole::User,
                "pergunta",
                Some("model-a"),
                ChatMessageState::Completed,
            )
            .unwrap();
            let assistant = s
                .chat_message_insert(
                    &conv,
                    ChatRole::Orchestrator,
                    "",
                    Some("model-a"),
                    ChatMessageState::Streaming,
                )
                .unwrap();
            s.chat_message_delta(&assistant, "resposta ").unwrap();
            s.chat_message_delta(&assistant, "persistida").unwrap();
            s.chat_message_finish(
                &conv,
                &assistant,
                ChatMessageState::Completed,
                Some("model-a"),
                &UsageInfo {
                    input_tokens: Some(10),
                    output_tokens: Some(4),
                    cached_tokens: None,
                    reasoning_tokens: None,
                },
                None,
            )
            .unwrap();
            s.chat_message_insert(
                &other,
                ChatRole::Orchestrator,
                "ativa",
                Some("model-b"),
                ChatMessageState::Streaming,
            )
            .unwrap();
        }
        let s = Store::open(&p).unwrap();
        let rows = s.chat_messages_list(&conv).unwrap();
        assert_eq!(rows.len(), 2, "deltas atualizam um único registro");
        assert_eq!(rows[1].text, "resposta persistida");
        assert_eq!(rows[1].state, ChatMessageState::Completed);
        assert_eq!(rows[1].provider.as_deref(), Some("chatgpt_plan"));
        assert_eq!(rows[1].model.as_deref(), Some("model-a"));
        assert_eq!(rows[1].usage.input_tokens, Some(10));
        assert_eq!(s.chat_messages_list(&other).unwrap().len(), 1);
        s.chat_generation_reconcile().unwrap();
        assert_eq!(
            s.chat_messages_list(&other).unwrap()[0].state,
            ChatMessageState::Interrupted
        );
        assert_eq!(s.chat_conversations_list().unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(p.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn upgrade_v3_to_v4_keeps_runs_as_direct_user_initiated() {
        use one_protocol::{ChatMode, RunInitiator};
        let p = temp_db("v4up");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        {
            let c = Connection::open(&p).unwrap();
            c.pragma_update(None, "foreign_keys", "ON").unwrap();
            migrations::apply_with(&c, &migrations::MIGRATIONS[..3]).unwrap();
            c.execute_batch(
                "INSERT INTO gateways(id,name,kind) VALUES('g','G','local');
                 INSERT INTO projects(id,name,gateway_id,path) VALUES('p','p','g','/p');
                 INSERT INTO conversations(id,project_id,title) VALUES('c','p','t');
                 INSERT INTO tasks(id,conversation_id,gateway_id,project_id,engine,mode,prompt) VALUES('t','c','g','p','codex','read_only','x');
                 INSERT INTO runs(id,task_id,status) VALUES('r','t','succeeded');",
            ).unwrap();
        }
        let s = Store::open(&p).unwrap();
        assert_eq!(s.schema_version().unwrap(), LATEST_VERSION);
        assert_eq!(
            s.run_get("r").unwrap().unwrap().initiator,
            RunInitiator::User
        );
        assert_eq!(s.conversation_mode("c").unwrap(), Some(ChatMode::Direct));
        assert!(PathBuf::from(format!("{}.pre-v4.bak", p.display())).exists());
        let _ = std::fs::remove_dir_all(p.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn upgrade_v5_to_v6_keeps_conversations_and_adds_empty_context() {
        use one_protocol::ChatMode;
        let p = temp_db("v6up");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        {
            let c = Connection::open(&p).unwrap();
            c.pragma_update(None, "foreign_keys", "ON").unwrap();
            migrations::apply_with(&c, &migrations::MIGRATIONS[..5]).unwrap();
            c.execute_batch(
                "INSERT INTO conversations(id,title,mode,provider,model,updated_at) VALUES('co','Plano','orchestrator','chatgpt_plan','gpt-a','2026-11-17T12:00:00Z');
                 INSERT INTO messages(id,conversation_id,role,body,provider,model) VALUES('m1','co','user','oi','chatgpt_plan','gpt-a');
                 INSERT INTO conversations(id,title,mode) VALUES('cd','Direta','direct');",
            )
            .unwrap();
        }
        let s = Store::open(&p).unwrap();
        assert_eq!(s.schema_version().unwrap(), LATEST_VERSION);
        assert_eq!(LATEST_VERSION, 6);
        assert!(PathBuf::from(format!("{}.pre-v6.bak", p.display())).exists());
        let list = s.chat_conversations_list().unwrap();
        assert_eq!(list.len(), 1, "só conversas do orquestrador");
        assert_eq!(list[0].title, "Plano");
        assert_eq!(list[0].model.as_deref(), Some("gpt-a"));
        assert_eq!(list[0].context, one_protocol::ChatContext::default());
        assert_eq!(s.chat_messages_list("co").unwrap().len(), 1);
        assert_eq!(s.conversation_mode("cd").unwrap(), Some(ChatMode::Direct));
        let _ = std::fs::remove_dir_all(p.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn chat_context_persists_per_conversation_across_reopen() {
        use one_protocol::{ChatContext, ExecutorPreference};
        let p = temp_db("ctx");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let (a, b, direct);
        {
            let s = Store::open(&p).unwrap();
            a = s.chat_conversation_create().unwrap();
            b = s.chat_conversation_create().unwrap();
            direct = s
                .conversation_create(one_protocol::ChatMode::Direct, "d")
                .unwrap();
            let ctx = ChatContext {
                gateway_id: Some("local".into()),
                workspace_id: Some("ws-removida".into()), // sem FK: pasta removida não quebra a conversa
                executor_engine: Some(ExecutorPreference::Auto),
                executor_model: None,
                orchestrator_model: Some("gpt-5".into()),
            };
            assert!(s.chat_conversation_context_set(&a, &ctx).unwrap());
            assert!(
                !s.chat_conversation_context_set(&direct, &ctx).unwrap(),
                "Modo direto não recebe contexto de conversa"
            );
            assert!(!s
                .chat_conversation_context_set("cv_nao_existe", &ctx)
                .unwrap());
        }
        let s = Store::open(&p).unwrap();
        let list = s.chat_conversations_list().unwrap();
        let ca = list.iter().find(|c| c.id == a).unwrap();
        assert_eq!(ca.context.gateway_id.as_deref(), Some("local"));
        assert_eq!(ca.context.workspace_id.as_deref(), Some("ws-removida"));
        assert_eq!(ca.context.executor_engine, Some(ExecutorPreference::Auto));
        assert_eq!(ca.context.orchestrator_model.as_deref(), Some("gpt-5"));
        let cb = list.iter().find(|c| c.id == b).unwrap();
        assert_eq!(cb.context, ChatContext::default(), "isolado por conversa");
        // CHECK do banco recusa motor desconhecido
        assert!(s
            .conn
            .execute(
                "UPDATE conversations SET ctx_executor_engine='gemini' WHERE id=?1",
                [&a]
            )
            .is_err());
        let _ = std::fs::remove_dir_all(p.parent().unwrap().parent().unwrap());
    }
}

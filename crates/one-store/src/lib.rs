//! Persistência local (SQLite + FTS5) do OneAgent.
//!
//! Fase 1: conexão, migrações versionadas e verificação de saúde. As operações de domínio
//! (conversas, tarefas, OneContext) entram nas fases seguintes sobre este esquema.

use std::path::{Path, PathBuf};

use one_protocol::StoreStatus;
use rusqlite::Connection;

mod migrations;

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
        assert_eq!(s.schema_version().unwrap(), 2);
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
        assert_eq!(Store::open(&p).unwrap().schema_version().unwrap(), 2);
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
}

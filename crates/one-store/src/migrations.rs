//! Migrações versionadas por `PRAGMA user_version`. Nunca editar uma migração publicada:
//! adicionar uma nova entrada em `MIGRATIONS`.

use rusqlite::Connection;

use crate::{Result, StoreError};

const V1: &str = r#"
CREATE TABLE gateways (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('local','remote')),
  -- identidade pública do Gateway (fingerprint); nenhuma credencial é guardada aqui
  fingerprint TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
) STRICT;

CREATE TABLE projects (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  gateway_id TEXT NOT NULL REFERENCES gateways(id) ON DELETE CASCADE,
  path TEXT NOT NULL,            -- caminho no Gateway
  instructions TEXT,             -- instruções do projeto (nível 3 da hierarquia)
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  UNIQUE (gateway_id, path)
) STRICT;

CREATE TABLE conversations (
  id TEXT PRIMARY KEY,
  project_id TEXT REFERENCES projects(id) ON DELETE SET NULL,
  title TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
) STRICT;

CREATE TABLE messages (
  id TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK (role IN ('user','orchestrator','system')),
  body TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
) STRICT;

CREATE TABLE tasks (
  id TEXT PRIMARY KEY,
  conversation_id TEXT REFERENCES conversations(id) ON DELETE SET NULL,
  gateway_id TEXT NOT NULL REFERENCES gateways(id),
  project_id TEXT NOT NULL REFERENCES projects(id),
  engine TEXT NOT NULL CHECK (engine IN ('codex','claude','cursor')),
  model TEXT,
  mode TEXT NOT NULL CHECK (mode IN ('read_only','write')),
  prompt TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
) STRICT;

CREATE TABLE runs (
  id TEXT PRIMARY KEY,
  task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  status TEXT NOT NULL CHECK (status IN ('queued','running','awaiting_approval','succeeded','failed','cancelled')),
  started_at TEXT,
  finished_at TEXT,
  exit_code INTEGER,
  summary TEXT,                  -- resumo estruturado; o log completo fica no Gateway
  evidence_refs TEXT NOT NULL DEFAULT '[]'  -- JSON: referências a evidências remotas
) STRICT;

CREATE TABLE run_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
  ts TEXT NOT NULL,
  kind TEXT NOT NULL,
  payload TEXT NOT NULL          -- JSON já com segredos redigidos
) STRICT;
CREATE INDEX run_events_run ON run_events(run_id, id);

-- Trilha de auditoria append-only (hash encadeado preenchido pelo Hub nas fases seguintes).
CREATE TABLE audit_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  actor TEXT NOT NULL,
  action TEXT NOT NULL,
  subject TEXT,
  outcome TEXT NOT NULL,
  prev_hash TEXT,
  hash TEXT
) STRICT;
CREATE TRIGGER audit_log_no_update BEFORE UPDATE ON audit_log
BEGIN SELECT RAISE(ABORT, 'audit_log é append-only'); END;
CREATE TRIGGER audit_log_no_delete BEFORE DELETE ON audit_log
BEGIN SELECT RAISE(ABORT, 'audit_log é append-only'); END;

-- OneContext: fontes com proveniência e trechos pesquisáveis (ver docs/CONTEXT.md).
CREATE TABLE context_sources (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  origin TEXT NOT NULL,          -- file | commit | run | message | url
  ref TEXT NOT NULL,             -- caminho, SHA, run_id...
  version TEXT,                  -- commit/versão
  content_hash TEXT NOT NULL,
  captured_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
) STRICT;

CREATE TABLE context_chunks (
  id TEXT PRIMARY KEY,
  source_id TEXT NOT NULL REFERENCES context_sources(id) ON DELETE CASCADE,
  module TEXT,
  body TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('proposed','validated','official'))
) STRICT;

CREATE VIRTUAL TABLE context_chunks_fts USING fts5(
  body, module,
  content = 'context_chunks', content_rowid = 'rowid',
  tokenize = 'unicode61 remove_diacritics 2'
);
CREATE TRIGGER context_chunks_ai AFTER INSERT ON context_chunks BEGIN
  INSERT INTO context_chunks_fts(rowid, body, module) VALUES (new.rowid, new.body, new.module);
END;
CREATE TRIGGER context_chunks_ad AFTER DELETE ON context_chunks BEGIN
  INSERT INTO context_chunks_fts(context_chunks_fts, rowid, body, module) VALUES ('delete', old.rowid, old.body, old.module);
END;
CREATE TRIGGER context_chunks_au AFTER UPDATE ON context_chunks BEGIN
  INSERT INTO context_chunks_fts(context_chunks_fts, rowid, body, module) VALUES ('delete', old.rowid, old.body, old.module);
  INSERT INTO context_chunks_fts(rowid, body, module) VALUES (new.rowid, new.body, new.module);
END;
"#;

/// V2: (a) integridade Gateway/projeto em `tasks` via FK composta; (b) identidade aprovada das CLIs.
/// Reconstrói `tasks` (SQLite não adiciona FK por ALTER) seguindo o procedimento oficial: com
/// `foreign_keys=OFF` fora da transação, copia, troca, `foreign_key_check` e só então confirma.
const V2: &str = r#"
CREATE UNIQUE INDEX projects_id_gateway ON projects(id, gateway_id);

CREATE TABLE tasks_new (
  id TEXT PRIMARY KEY,
  conversation_id TEXT REFERENCES conversations(id) ON DELETE SET NULL,
  gateway_id TEXT NOT NULL,
  project_id TEXT NOT NULL,
  engine TEXT NOT NULL CHECK (engine IN ('codex','claude','cursor')),
  model TEXT,
  mode TEXT NOT NULL CHECK (mode IN ('read_only','write')),
  prompt TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  -- o projeto precisa pertencer ao MESMO gateway da tarefa
  FOREIGN KEY (project_id, gateway_id) REFERENCES projects(id, gateway_id)
) STRICT;
INSERT INTO tasks_new (id, conversation_id, gateway_id, project_id, engine, model, mode, prompt, created_at)
  SELECT id, conversation_id, gateway_id, project_id, engine, model, mode, prompt, created_at FROM tasks;
DROP TABLE tasks;
ALTER TABLE tasks_new RENAME TO tasks;

-- Identidade aprovada pelo usuário de cada CLI (uma por motor; reaprovar substitui).
CREATE TABLE trusted_executables (
  engine TEXT PRIMARY KEY CHECK (engine IN ('codex','claude','cursor')),
  fingerprint TEXT NOT NULL,
  canonical_path TEXT NOT NULL,
  approved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
) STRICT;
"#;

/// V3: execuções reais (Fase 2A). Métricas OneUsage por execução e a sessão interna do motor, que fica
/// SEPARADA do contexto da tarefa (base do futuro OneContext Handoff). Só `ADD COLUMN`: preserva dados.
const V3: &str = r#"
ALTER TABLE runs ADD COLUMN engine_session_id TEXT;
ALTER TABLE runs ADD COLUMN model_reported TEXT;      -- só o que o executor informou
ALTER TABLE runs ADD COLUMN duration_ms INTEGER;
ALTER TABLE runs ADD COLUMN input_tokens INTEGER;
ALTER TABLE runs ADD COLUMN output_tokens INTEGER;
ALTER TABLE runs ADD COLUMN cached_tokens INTEGER;
ALTER TABLE runs ADD COLUMN reasoning_tokens INTEGER;
ALTER TABLE runs ADD COLUMN tool_calls INTEGER NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN attempts INTEGER NOT NULL DEFAULT 1;
ALTER TABLE runs ADD COLUMN error_kind TEXT;
ALTER TABLE runs ADD COLUMN error_message TEXT;
ALTER TABLE runs ADD COLUMN final_text TEXT;
CREATE INDEX runs_task ON runs(task_id);
CREATE INDEX runs_status ON runs(status);

ALTER TABLE run_events ADD COLUMN seq INTEGER NOT NULL DEFAULT 0;
CREATE UNIQUE INDEX run_events_seq ON run_events(run_id, seq) WHERE seq > 0;
"#;

pub struct Migration {
    pub sql: &'static str,
    /// Exige `PRAGMA foreign_keys=OFF` durante a migração (reconstrução de tabela).
    pub fk_off: bool,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        sql: V1,
        fk_off: false,
    },
    Migration {
        sql: V2,
        fk_off: true,
    },
    Migration {
        sql: V3,
        fk_off: false,
    },
];

pub const LATEST_VERSION: u32 = MIGRATIONS.len() as u32;

pub fn user_version(conn: &Connection) -> Result<u32> {
    Ok(conn.pragma_query_value(None, "user_version", |r| r.get(0))?)
}

/// `integrity_check` + `foreign_key_check`; qualquer linha reportada é falha.
pub fn check_integrity(conn: &Connection) -> Result<()> {
    let ic: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if ic != "ok" {
        return Err(StoreError::Integrity(format!("integrity_check: {ic}")));
    }
    let violations: i64 =
        conn.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })?;
    if violations > 0 {
        return Err(StoreError::Integrity(format!(
            "{violations} violação(ões) de chave estrangeira"
        )));
    }
    Ok(())
}

pub fn apply(conn: &Connection) -> Result<()> {
    apply_with(conn, MIGRATIONS)
}

/// Aplica as pendentes, cada uma em transação própria (falha = rollback e versão inalterada).
pub fn apply_with(conn: &Connection, migrations: &[Migration]) -> Result<()> {
    let current = user_version(conn)?;
    let latest = migrations.len() as u32;
    if current > latest {
        return Err(StoreError::SchemaTooNew {
            found: current,
            supported: latest,
        });
    }
    if current == latest {
        return Ok(());
    }
    if current > 0 {
        check_integrity(conn)?; // não migrar sobre um banco já inconsistente
    }
    for (i, m) in migrations.iter().enumerate().skip(current as usize) {
        let version = (i + 1) as u32;
        if m.fk_off {
            conn.pragma_update(None, "foreign_keys", "OFF")?;
        }
        let result = run_one(conn, m, version);
        if m.fk_off {
            conn.pragma_update(None, "foreign_keys", "ON")?; // sempre restaura, inclusive em erro
        }
        result.map_err(|e| StoreError::Migration {
            version,
            source: Box::new(e),
        })?;
    }
    check_integrity(conn)
}

fn run_one(conn: &Connection, m: &Migration, version: u32) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(m.sql)?;
    if m.fk_off {
        // Dados que violariam a nova regra abortam a migração (rollback), nunca são apagados.
        check_integrity(&tx)?;
    }
    tx.pragma_update(None, "user_version", version)?;
    tx.commit()?;
    Ok(())
}

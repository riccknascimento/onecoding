//! Conexão PostgreSQL (tokio-postgres + deadpool) e migrations versionadas.
//! Migrations: arquivos `NNNN_nome.sql` em `migrations/` (ou `ONEAGENT_CENTRAL_MIGRATIONS`), aplicados em
//! ordem, cada um numa transação, com checksum SHA-256 registrado. Migração já aplicada e alterada = erro.
//! Um advisory lock impede que duas instâncias migrem ao mesmo tempo.

use std::error::Error;
use std::path::{Path, PathBuf};

use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use sha2::{Digest, Sha256};
use tokio_postgres::NoTls;

pub type DbPool = Pool;

pub fn connect(url: &str) -> Result<DbPool, Box<dyn Error>> {
    let cfg: tokio_postgres::Config = url.parse().map_err(|_| "URL do PostgreSQL inválida")?;
    let mgr = Manager::from_config(
        cfg,
        NoTls,
        ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        },
    );
    Ok(Pool::builder(mgr).max_size(10).build()?)
}

pub fn migrations_dir() -> PathBuf {
    std::env::var_os("ONEAGENT_CENTRAL_MIGRATIONS")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"))
}

fn hex_sha256(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Lê `NNNN_nome.sql` ordenados pela versão numérica.
/// Migration lida do disco: (versão, nome do arquivo, SQL).
pub type Migration = (i32, String, String);

pub fn load_migrations(dir: &Path) -> Result<Vec<Migration>, Box<dyn Error>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(String::from) else {
            continue;
        };
        // arquivos ocultos (ex.: "._0001_init.sql" criado pelo macOS) não são migrations
        if name.starts_with('.') || !name.ends_with(".sql") {
            continue;
        }
        let version: i32 = name
            .split('_')
            .next()
            .and_then(|v| v.parse().ok())
            .ok_or("nome de migration inválido: use NNNN_nome.sql")?;
        out.push((version, name, std::fs::read_to_string(&path)?));
    }
    out.sort_by_key(|m| m.0);
    Ok(out)
}

pub async fn migrate(pool: &DbPool) -> Result<usize, Box<dyn Error>> {
    let mut client = pool.get().await?;
    client.batch_execute(
        "SELECT pg_advisory_lock(7101); \
         CREATE TABLE IF NOT EXISTS schema_migrations (version integer PRIMARY KEY, name text NOT NULL, \
         sha256 text NOT NULL, applied_at timestamptz NOT NULL DEFAULT now());",
    )
    .await?;
    let result = apply_all(&mut client).await;
    // libera o lock mesmo em erro
    let _ = client
        .batch_execute("SELECT pg_advisory_unlock(7101)")
        .await;
    result
}

async fn apply_all(client: &mut deadpool_postgres::Object) -> Result<usize, Box<dyn Error>> {
    let mut applied = 0;
    for (version, name, sql) in load_migrations(&migrations_dir())? {
        let sum = hex_sha256(&sql);
        if let Some(row) = client
            .query_opt(
                "SELECT sha256 FROM schema_migrations WHERE version = $1",
                &[&version],
            )
            .await?
        {
            let recorded: String = row.get(0);
            if recorded != sum {
                return Err(format!("migration {name} foi alterada depois de aplicada").into());
            }
            continue;
        }
        let tx = client.transaction().await?;
        tx.batch_execute(&sql).await?;
        tx.execute(
            "INSERT INTO schema_migrations(version, name, sha256) VALUES ($1, $2, $3)",
            &[&version, &name, &sum],
        )
        .await?;
        tx.commit().await?;
        applied += 1;
    }
    Ok(applied)
}

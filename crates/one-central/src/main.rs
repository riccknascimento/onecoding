//! Binário do OneHub Central.
//!
//! Serviço: `one-central` (configuração por ambiente):
//! - `ONEAGENT_CENTRAL_DATABASE_URL` (obrigatória; nunca impressa)
//! - `ONEAGENT_CENTRAL_BIND` (padrão `127.0.0.1:8787`; endereço não local exige `ONEAGENT_CENTRAL_ALLOW_PUBLIC_BIND=1`)
//! - `ONEAGENT_CENTRAL_COOKIE_SECURE` (padrão `1`: cookie de sessão com `Secure`; `0` só para desenvolvimento em http)
//! - `ONEAGENT_CENTRAL_TRUSTED_PROXIES` (IPs separados por vírgula de proxies confiáveis, ex.: o Caddy; só deles o X-Forwarded-For entra no limite de login por origem)
//! - `ONEAGENT_CENTRAL_ALLOWED_ORIGINS` (lista separada por vírgula de origens cross-site autorizadas; vazio = nenhuma)
//!
//! Bootstrap do administrador inicial (uso único): `one-central bootstrap-admin`, com o e-mail em
//! `ONEAGENT_BOOTSTRAP_EMAIL` e a senha na entrada padrão (uma linha). Nada é impresso além do id.

use std::io::BufRead;

use one_central::routes::Settings;
use one_central::{bootstrap, db, is_loopback_bind, orchestrator::UnavailableProvider, routes};

fn database_url() -> Result<String, Box<dyn std::error::Error>> {
    Ok(std::env::var("ONEAGENT_CENTRAL_DATABASE_URL")
        .map_err(|_| "defina ONEAGENT_CENTRAL_DATABASE_URL")?)
}

async fn run_bootstrap_admin() -> Result<(), Box<dyn std::error::Error>> {
    let email =
        std::env::var("ONEAGENT_BOOTSTRAP_EMAIL").map_err(|_| "defina ONEAGENT_BOOTSTRAP_EMAIL")?;
    let mut password = String::new();
    std::io::stdin().lock().read_line(&mut password)?;
    let password = password.trim_end_matches(['\r', '\n']);
    let pool = db::connect(&database_url()?)?;
    db::migrate(&pool)
        .await
        .map_err(|_| "falha nas migrations")?;
    match bootstrap::bootstrap_admin(&pool, &email, password).await {
        Ok(id) => {
            println!("administrador criado: {id}");
            Ok(())
        }
        Err(e) => Err(e.to_string().into()),
    }
}

fn settings_from_env() -> Settings {
    let cookie_secure = std::env::var("ONEAGENT_CENTRAL_COOKIE_SECURE").as_deref() != Ok("0");
    let allowed_origins = std::env::var("ONEAGENT_CENTRAL_ALLOWED_ORIGINS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|o| !o.is_empty())
        .map(String::from)
        .collect();
    // IPs de proxies confiáveis (ex.: o Caddy): só deles o X-Forwarded-For é considerado (ver routes::client_origin)
    let trusted_proxies = std::env::var("ONEAGENT_CENTRAL_TRUSTED_PROXIES")
        .unwrap_or_default()
        .split(',')
        .filter_map(|ip| ip.trim().parse().ok())
        .collect();
    // Chave-mestra das API Keys de provedores: arquivo fora do banco. Falha de leitura não derruba o serviço;
    // os endpoints de provedores respondem 503 até a chave ser corrigida. O conteúdo nunca é impresso.
    let provider_key = match std::env::var("ONEAGENT_CENTRAL_PROVIDER_KEY_FILE") {
        Ok(path) => match one_central::provider_keys::load_master_key(&path) {
            Ok(key) => Some(key),
            Err(motivo) => {
                eprintln!("chave-mestra de provedores indisponível: {motivo}");
                None
            }
        },
        Err(_) => None,
    };
    Settings {
        cookie_secure,
        allowed_origins,
        trusted_proxies,
        provider_key,
    }
}

/// Aplica só as migrations pendentes, em ordem, e encerra. Passo separado do deploy (ver docs/CENTRAL-OPERACAO.md).
async fn run_migrate() -> Result<(), Box<dyn std::error::Error>> {
    let pool = db::connect(&database_url()?)?;
    let applied = db::migrate(&pool)
        .await
        .map_err(|e| format!("falha nas migrations: {e}"))?;
    println!("migrations aplicadas agora: {applied}");
    Ok(())
}

async fn run_server() -> Result<(), Box<dyn std::error::Error>> {
    let bind = std::env::var("ONEAGENT_CENTRAL_BIND").unwrap_or_else(|_| "127.0.0.1:8787".into());
    let public_ok = std::env::var("ONEAGENT_CENTRAL_ALLOW_PUBLIC_BIND").as_deref() == Ok("1");
    if !is_loopback_bind(&bind) && !public_ok {
        return Err(
            "bind não local recusado: publique só atrás de HTTPS e com autorização explícita"
                .into(),
        );
    }
    let pool = db::connect(&database_url()?)?;
    let applied = db::migrate(&pool)
        .await
        .map_err(|e| format!("falha nas migrations: {e}"))?;
    eprintln!("migrations aplicadas agora: {applied}");
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    eprintln!("OneHub Central ouvindo em {bind}");
    let settings = settings_from_env();
    let app =
        routes::router_with_settings(pool, std::sync::Arc::new(UnavailableProvider), settings);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match std::env::args().nth(1).as_deref() {
        Some("bootstrap-admin") => run_bootstrap_admin().await,
        Some("migrate") => run_migrate().await,
        None => run_server().await,
        Some(_) => Err(
            "comando desconhecido (use: migrate, bootstrap-admin, ou nenhum argumento para o serviço)"
                .into(),
        ),
    }
}

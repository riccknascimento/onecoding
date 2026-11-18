use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use url::Url;
use uuid::Uuid;

const SERVICE: &str = "com.oneagent.chatgpt-plan";
const ACCOUNT: &str = "active-profile";
const HOST_ACCOUNT: &str = "host-id";
const AUTH_ENDPOINT: &str = "https://auth.openai.com/api/accounts/authorize";
const TOKEN_ENDPOINT: &str = "https://auth.openai.com/api/accounts/oauth/token";
const RESOURCE: &str = "https://api.openai.com/v1";
const ISSUER: &str = "https://auth.openai.com";
const PROFILE_NAME: &str = "OneAgent";
const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
const REQUESTED_SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatGptAuthError {
    Cancelled,
    Timeout,
    Keychain(String),
    Network(String),
    OAuth(String),
    ReauthenticationRequired(String),
    InvalidResponse(String),
    PlanUsageNotAuthorized,
}

impl std::fmt::Display for ChatGptAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(f, "Autorização cancelada no navegador."),
            Self::Timeout => write!(f, "A autorização expirou antes do retorno do navegador."),
            Self::Keychain(e) => write!(f, "Falha no macOS Keychain: {e}"),
            Self::Network(e)
            | Self::OAuth(e)
            | Self::ReauthenticationRequired(e)
            | Self::InvalidResponse(e) => f.write_str(e),
            Self::PlanUsageNotAuthorized => write!(
                f,
                "Login concluído, mas ChatGPT Plan Usage não foi autorizado."
            ),
        }
    }
}

impl std::error::Error for ChatGptAuthError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Credentials {
    pub(super) client_id: String,
    pub(super) subject: String,
    pub(super) email: Option<String>,
    pub(super) id_token: String,
    pub(super) access_token: String,
    pub(super) refresh_token: String,
    pub(super) scopes: Vec<String>,
    pub(super) expires_at: u64,
    pub(super) models: Vec<AvailableModel>,
    #[serde(default)]
    pub(super) pending: Option<PendingTokens>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PendingTokens {
    access_token: String,
    refresh_token: String,
    id_token: String,
    scopes: Vec<String>,
    expires_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AvailableModel {
    pub slug: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSummary {
    pub email: Option<String>,
    pub plan_usage_authorized: bool,
    pub models: Vec<AvailableModel>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    id_token: String,
    expires_in: u64,
    scope: String,
}

#[derive(Deserialize)]
struct IdClaims {
    sub: String,
    nonce: Option<String>,
    email: Option<String>,
    email_verified: Option<bool>,
}

#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    jwks_uri: String,
    revocation_endpoint: Option<String>,
}

#[derive(Deserialize)]
struct ModelList {
    models: Vec<ModelItem>,
}

#[derive(Deserialize)]
struct ModelItem {
    slug: String,
    display_name: String,
    visibility: String,
}

pub struct ChatGptAuth {
    http: Client,
}

impl ChatGptAuth {
    pub fn new() -> Result<Self, ChatGptAuthError> {
        let http = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| ChatGptAuthError::Network(e.to_string()))?;
        Ok(Self { http })
    }

    pub fn account(&self) -> Result<Option<AccountSummary>, ChatGptAuthError> {
        self.load().map(|c| c.map(|c| summary(&c)))
    }

    pub fn connect(&self) -> Result<AccountSummary, ChatGptAuthError> {
        let mut credentials = self.load()?;
        let client_id = match credentials.as_ref() {
            Some(c) => c.client_id.clone(),
            None => self
                .load_registration()?
                .unwrap_or_else(|| "dynamic_agent_client".into()),
        };
        let host_id = self.host_id()?;
        let state = random_url_token(32)?;
        let nonce = random_url_token(32)?;
        let verifier = random_url_token(32)?;
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .map_err(|e| {
                ChatGptAuthError::Network(format!("Não foi possível abrir callback local: {e}"))
            })?;
        listener
            .set_nonblocking(true)
            .map_err(|e| ChatGptAuthError::Network(e.to_string()))?;
        let redirect_uri = format!(
            "http://127.0.0.1:{}/auth/callback",
            listener.local_addr().unwrap().port()
        );
        let mut authorize = Url::parse(AUTH_ENDPOINT).expect("authorize endpoint");
        {
            let mut q = authorize.query_pairs_mut();
            q.append_pair("client_id", &client_id)
                .append_pair("ext_agent_host_id", &host_id)
                .append_pair("response_type", "code")
                .append_pair("redirect_uri", &redirect_uri)
                .append_pair("scope", REQUESTED_SCOPES)
                .append_pair("resource", RESOURCE)
                .append_pair("state", &state)
                .append_pair("nonce", &nonce)
                .append_pair("code_challenge_method", "S256")
                .append_pair("code_challenge", &challenge);
            if client_id == "dynamic_agent_client" {
                q.append_pair("agent_name_hint", PROFILE_NAME);
            } else if credentials
                .as_ref()
                .is_none_or(|c| !has_plan_scope(&c.scopes))
            {
                q.append_pair("prompt", "consent");
            }
        }

        Command::new("open")
            .arg(authorize.as_str())
            .status()
            .map_err(|e| {
                ChatGptAuthError::Network(format!("Não foi possível abrir o navegador: {e}"))
            })?
            .success()
            .then_some(())
            .ok_or_else(|| {
                ChatGptAuthError::Network("O macOS não abriu o navegador do sistema.".into())
            })?;

        let callback = wait_for_callback(listener, &state, Duration::from_secs(300))?;
        if let Some(issued) = callback.client_id {
            if client_id != "dynamic_agent_client" && issued != client_id {
                return Err(ChatGptAuthError::InvalidResponse(
                    "A OpenAI retornou um client_id diferente do perfil selecionado.".into(),
                ));
            }
            credentials
                .get_or_insert_with(|| Credentials {
                    client_id: issued.clone(),
                    subject: String::new(),
                    email: None,
                    id_token: String::new(),
                    access_token: String::new(),
                    refresh_token: String::new(),
                    scopes: vec![],
                    expires_at: 0,
                    models: vec![],
                    pending: None,
                })
                .client_id = issued.clone();
            self.save_registration(&issued)?;
        }
        let code = callback.code.ok_or_else(|| {
            if callback.error.as_deref() == Some("access_denied") {
                ChatGptAuthError::Cancelled
            } else {
                ChatGptAuthError::OAuth(
                    callback
                        .error
                        .unwrap_or_else(|| "Callback sem código OAuth.".into()),
                )
            }
        })?;
        let saved_registration = self.load_registration()?;
        let issued_client_id = credentials
            .as_ref()
            .map(|c| c.client_id.clone())
            .or(saved_registration)
            .ok_or_else(|| {
                ChatGptAuthError::InvalidResponse("client_id ausente no callback.".into())
            })?;
        if client_id == "dynamic_agent_client" && issued_client_id == "dynamic_agent_client" {
            return Err(ChatGptAuthError::InvalidResponse(
                "O registro terminou sem emitir um client_id. Nenhuma credencial foi salva.".into(),
            ));
        }

        let token: TokenResponse = self
            .http
            .post(TOKEN_ENDPOINT)
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", issued_client_id.as_str()),
                ("code", code.as_str()),
                ("code_verifier", verifier.as_str()),
                ("redirect_uri", redirect_uri.as_str()),
                ("resource", RESOURCE),
            ])
            .send()
            .map_err(|e| ChatGptAuthError::Network(format!("Falha ao trocar o código OAuth: {e}")))
            .and_then(parse_response)?;
        let scopes: Vec<String> = token
            .scope
            .split_ascii_whitespace()
            .map(str::to_owned)
            .collect();
        let claims = self.validate_id_token(&token.id_token, &issued_client_id, Some(&nonce))?;
        let mut next = Credentials {
            client_id: issued_client_id,
            subject: claims.sub,
            email: claims.email.filter(|_| claims.email_verified == Some(true)),
            id_token: token.id_token,
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            scopes,
            expires_at: now_seconds().saturating_add(token.expires_in),
            models: vec![],
            pending: None,
        };
        self.save(&next)?;
        if has_plan_scope(&next.scopes) {
            next.models = self.list_models(&next.access_token)?;
            self.save(&next)?;
        }
        Ok(summary(&next))
    }

    pub fn disconnect(&self) -> Result<(), ChatGptAuthError> {
        let Some(c) = self.load()? else {
            return Ok(());
        };
        let remote = (|| {
            let discovery = self.discovery()?;
            if discovery.issuer != ISSUER {
                return Err(ChatGptAuthError::InvalidResponse(
                    "Issuer OpenID inesperado durante revogação.".into(),
                ));
            }
            let url = discovery.revocation_endpoint.as_deref().ok_or_else(|| {
                ChatGptAuthError::InvalidResponse("Discovery sem revocation_endpoint.".into())
            })?;
            let revoke_url = Url::parse(url).map_err(|_| {
                ChatGptAuthError::InvalidResponse("revocation_endpoint inválido.".into())
            })?;
            if revoke_url.scheme() != "https" || revoke_url.host_str() != Some("auth.openai.com") {
                return Err(ChatGptAuthError::InvalidResponse(
                    "revocation_endpoint fora do host oficial.".into(),
                ));
            }
            let response = self
                .http
                .post(revoke_url)
                .form(&[
                    ("token", c.refresh_token.as_str()),
                    ("token_type_hint", "refresh_token"),
                    ("client_id", c.client_id.as_str()),
                ])
                .send()
                .map_err(|e| ChatGptAuthError::Network(e.to_string()))?;
            if response.status().is_success() {
                Ok(())
            } else {
                Err(http_error(response.status(), response.headers(), None))
            }
        })();
        self.delete()?;
        remote.map_err(|e| {
            ChatGptAuthError::OAuth(format!(
                "Credencial local removida; revogação remota não confirmada. {e}"
            ))
        })
    }

    fn host_id(&self) -> Result<String, ChatGptAuthError> {
        let entry = keyring::Entry::new(SERVICE, HOST_ACCOUNT)
            .map_err(|e| ChatGptAuthError::Keychain(e.to_string()))?;
        match entry.get_password() {
            Ok(id) => Ok(id),
            Err(keyring::Error::NoEntry) => {
                let id = format!("urn:uuid:{}", Uuid::new_v4());
                entry
                    .set_password(&id)
                    .map_err(|e| ChatGptAuthError::Keychain(e.to_string()))?;
                Ok(id)
            }
            Err(e) => Err(ChatGptAuthError::Keychain(e.to_string())),
        }
    }

    pub(super) fn load(&self) -> Result<Option<Credentials>, ChatGptAuthError> {
        let entry = keyring::Entry::new(SERVICE, ACCOUNT)
            .map_err(|e| ChatGptAuthError::Keychain(e.to_string()))?;
        match entry.get_password() {
            Ok(json) => serde_json::from_str(&json).map(Some).map_err(|_| {
                ChatGptAuthError::Keychain("A credencial salva não pôde ser lida.".into())
            }),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(ChatGptAuthError::Keychain(e.to_string())),
        }
    }

    pub(super) fn save(&self, c: &Credentials) -> Result<(), ChatGptAuthError> {
        let json = serde_json::to_string(c)
            .map_err(|e| ChatGptAuthError::InvalidResponse(e.to_string()))?;
        keyring::Entry::new(SERVICE, ACCOUNT)
            .map_err(|e| ChatGptAuthError::Keychain(e.to_string()))?
            .set_password(&json)
            .map_err(|e| ChatGptAuthError::Keychain(e.to_string()))
    }

    fn delete(&self) -> Result<(), ChatGptAuthError> {
        let entry = keyring::Entry::new(SERVICE, ACCOUNT)
            .map_err(|e| ChatGptAuthError::Keychain(e.to_string()))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(ChatGptAuthError::Keychain(e.to_string())),
        }
    }

    pub(super) fn clear_tokens(&self) -> Result<(), ChatGptAuthError> {
        self.delete()
    }

    fn save_registration(&self, id: &str) -> Result<(), ChatGptAuthError> {
        keyring::Entry::new(SERVICE, "issued-client-id")
            .map_err(|e| ChatGptAuthError::Keychain(e.to_string()))?
            .set_password(id)
            .map_err(|e| ChatGptAuthError::Keychain(e.to_string()))
    }

    fn load_registration(&self) -> Result<Option<String>, ChatGptAuthError> {
        let entry = keyring::Entry::new(SERVICE, "issued-client-id")
            .map_err(|e| ChatGptAuthError::Keychain(e.to_string()))?;
        match entry.get_password() {
            Ok(id) => Ok(Some(id)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(ChatGptAuthError::Keychain(e.to_string())),
        }
    }

    fn discovery(&self) -> Result<Discovery, ChatGptAuthError> {
        self.http
            .get("https://auth.openai.com/.well-known/openid-configuration")
            .send()
            .map_err(|e| ChatGptAuthError::Network(e.to_string()))
            .and_then(parse_response::<Discovery>)
    }

    fn validate_id_token(
        &self,
        token: &str,
        client_id: &str,
        nonce: Option<&str>,
    ) -> Result<IdClaims, ChatGptAuthError> {
        let discovery = self.discovery()?;
        let jwks_url = Url::parse(&discovery.jwks_uri)
            .map_err(|_| ChatGptAuthError::InvalidResponse("jwks_uri inválido.".into()))?;
        if discovery.issuer != ISSUER
            || jwks_url.scheme() != "https"
            || jwks_url.host_str() != Some("auth.openai.com")
        {
            return Err(ChatGptAuthError::InvalidResponse(
                "Issuer OpenID inesperado.".into(),
            ));
        }
        let jwks = self
            .http
            .get(jwks_url)
            .send()
            .map_err(|e| ChatGptAuthError::Network(e.to_string()))
            .and_then(parse_response::<JwkSet>)?;
        let header = decode_header(token)
            .map_err(|_| ChatGptAuthError::InvalidResponse("ID token inválido.".into()))?;
        if header.alg != Algorithm::RS256 {
            return Err(ChatGptAuthError::InvalidResponse(
                "Algoritmo de ID token não suportado.".into(),
            ));
        }
        let kid = header.kid.ok_or_else(|| {
            ChatGptAuthError::InvalidResponse("ID token sem chave de assinatura.".into())
        })?;
        let jwk = jwks
            .keys
            .iter()
            .find(|j| j.common.key_id.as_deref() == Some(&kid))
            .ok_or_else(|| {
                ChatGptAuthError::InvalidResponse(
                    "Chave de assinatura OpenID não encontrada.".into(),
                )
            })?;
        let key = DecodingKey::from_jwk(jwk)
            .map_err(|_| ChatGptAuthError::InvalidResponse("Chave OpenID inválida.".into()))?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[ISSUER]);
        validation.set_audience(&[client_id]);
        let claims = decode::<IdClaims>(token, &key, &validation)
            .map_err(|_| {
                ChatGptAuthError::InvalidResponse(
                    "Assinatura ou claims do ID token não são válidos.".into(),
                )
            })?
            .claims;
        if nonce.is_some_and(|nonce| claims.nonce.as_deref() != Some(nonce)) {
            return Err(ChatGptAuthError::InvalidResponse(
                "Nonce do ID token não corresponde à tentativa.".into(),
            ));
        }
        Ok(claims)
    }

    pub(super) fn token_for_request(&self) -> Result<(Credentials, String), ChatGptAuthError> {
        let mut c = self
            .load()?
            .ok_or_else(|| ChatGptAuthError::Keychain("Nenhuma conta ChatGPT conectada.".into()))?;
        if let Some(pending) = c.pending.clone() {
            self.commit_pending(&mut c, pending)?;
        }
        if !has_plan_scope(&c.scopes) {
            return Err(ChatGptAuthError::PlanUsageNotAuthorized);
        }
        if c.expires_at <= now_seconds().saturating_add(60) {
            let token = self
                .http
                .post(TOKEN_ENDPOINT)
                .form(&[
                    ("grant_type", "refresh_token"),
                    ("client_id", c.client_id.as_str()),
                    ("refresh_token", c.refresh_token.as_str()),
                    ("resource", RESOURCE),
                ])
                .send()
                .map_err(|e| ChatGptAuthError::Network(e.to_string()))
                .and_then(parse_response)
                .map_err(classify_refresh_error);
            let token: TokenResponse = match token {
                Ok(token) => token,
                Err(error @ ChatGptAuthError::ReauthenticationRequired(_)) => {
                    self.delete()?;
                    return Err(error);
                }
                Err(error) => return Err(error),
            };
            let scopes: Vec<String> = token
                .scope
                .split_ascii_whitespace()
                .map(str::to_owned)
                .collect();
            // Persist all rotated credentials before ID-token verification so a transient JWKS
            // outage never causes the consumed refresh token to be reused.
            c.pending = Some(PendingTokens {
                access_token: token.access_token,
                refresh_token: token.refresh_token,
                id_token: token.id_token,
                scopes,
                expires_at: now_seconds().saturating_add(token.expires_in),
            });
            self.save(&c)?;
            let pending = c.pending.clone().expect("pending refresh was saved");
            self.commit_pending(&mut c, pending)?;
        }
        if !has_plan_scope(&c.scopes) {
            return Err(ChatGptAuthError::PlanUsageNotAuthorized);
        }
        Ok((c.clone(), c.access_token.clone()))
    }

    fn commit_pending(
        &self,
        c: &mut Credentials,
        pending: PendingTokens,
    ) -> Result<(), ChatGptAuthError> {
        let claims = self.validate_id_token(&pending.id_token, &c.client_id, None)?;
        if claims.sub != c.subject {
            self.delete()?;
            return Err(ChatGptAuthError::ReauthenticationRequired(
                "A identidade renovada não corresponde à conta conectada. Entre novamente.".into(),
            ));
        }
        c.access_token = pending.access_token;
        c.refresh_token = pending.refresh_token;
        c.id_token = pending.id_token;
        c.scopes = pending.scopes;
        c.expires_at = pending.expires_at;
        c.pending = None;
        self.save(c)?;
        Ok(())
    }

    pub(super) fn reload_models(&self) -> Result<AccountSummary, ChatGptAuthError> {
        let (mut c, access_token) = self.token_for_request()?;
        c.models = self.list_models(&access_token)?;
        self.save(&c)?;
        Ok(summary(&c))
    }

    fn list_models(&self, access_token: &str) -> Result<Vec<AvailableModel>, ChatGptAuthError> {
        let response = self
            .http
            .get(format!("{RESOURCE}/models"))
            .bearer_auth(access_token)
            .send()
            .map_err(|e| ChatGptAuthError::Network(e.to_string()))?;
        let catalog: ModelList = match parse_response(response) {
            Ok(catalog) => catalog,
            Err(error) if is_invalid_user_error(&error) => {
                self.delete()?;
                return Err(ChatGptAuthError::ReauthenticationRequired(
                    error.to_string(),
                ));
            }
            Err(error) => return Err(error),
        };
        Ok(catalog
            .models
            .into_iter()
            .filter(|m| m.visibility == "list")
            .map(|m| AvailableModel {
                slug: m.slug,
                display_name: m.display_name,
            })
            .collect())
    }
}

fn summary(c: &Credentials) -> AccountSummary {
    AccountSummary {
        email: c.email.clone(),
        plan_usage_authorized: has_plan_scope(&c.scopes),
        models: c.models.clone(),
    }
}

fn has_plan_scope(scopes: &[String]) -> bool {
    scopes.iter().any(|s| s == PLAN_SCOPE)
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn random_url_token(bytes: usize) -> Result<String, ChatGptAuthError> {
    let mut value = vec![0_u8; bytes];
    getrandom::fill(&mut value)
        .map_err(|e| ChatGptAuthError::Network(format!("CSPRNG indisponível: {e}")))?;
    Ok(URL_SAFE_NO_PAD.encode(value))
}

struct Callback {
    code: Option<String>,
    client_id: Option<String>,
    error: Option<String>,
}

fn wait_for_callback(
    listener: TcpListener,
    expected_state: &str,
    timeout: Duration,
) -> Result<Callback, ChatGptAuthError> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((mut stream, peer)) => {
                if peer.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST) {
                    continue;
                }
                return handle_callback(&mut stream, expected_state);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50))
            }
            Err(e) => {
                return Err(ChatGptAuthError::Network(format!(
                    "Callback local falhou: {e}"
                )))
            }
        }
    }
    Err(ChatGptAuthError::Timeout)
}

fn handle_callback(
    stream: &mut TcpStream,
    expected_state: &str,
) -> Result<Callback, ChatGptAuthError> {
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok();
    let mut request = [0_u8; 8192];
    let read = stream
        .read(&mut request)
        .map_err(|e| ChatGptAuthError::Network(e.to_string()))?;
    let request = std::str::from_utf8(&request[..read])
        .map_err(|_| ChatGptAuthError::InvalidResponse("Callback não UTF-8.".into()))?
        .to_owned();
    let callback = parse_callback_request(&request, expected_state)?;
    let page = if callback.error.is_some() {
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n<!doctype html><title>OneAgent</title><p>Autorizacao nao concluida. Volte ao OneAgent.</p>".as_slice()
    } else {
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n<!doctype html><title>OneAgent</title><p>Autorizacao concluida. Volte ao OneAgent.</p>".as_slice()
    };
    let _ = stream.write_all(page);
    Ok(callback)
}

fn parse_callback_request(
    request: &str,
    expected_state: &str,
) -> Result<Callback, ChatGptAuthError> {
    let first = request
        .lines()
        .next()
        .ok_or_else(|| ChatGptAuthError::InvalidResponse("Callback HTTP vazio.".into()))?;
    let target = first
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| ChatGptAuthError::InvalidResponse("Callback sem caminho.".into()))?;
    let callback_url = Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|_| ChatGptAuthError::InvalidResponse("Callback URL inválida.".into()))?;
    if callback_url.path() != "/auth/callback" {
        return Err(ChatGptAuthError::InvalidResponse(
            "Caminho de callback inesperado.".into(),
        ));
    }
    let query: std::collections::HashMap<String, String> = callback_url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if query.get("state").map(String::as_str) != Some(expected_state) {
        return Err(ChatGptAuthError::InvalidResponse(
            "State OAuth não corresponde à tentativa.".into(),
        ));
    }
    Ok(Callback {
        code: query.get("code").cloned(),
        client_id: query.get("client_id").cloned(),
        error: query.get("error").cloned(),
    })
}

fn parse_response<T: for<'de> Deserialize<'de>>(
    response: reqwest::blocking::Response,
) -> Result<T, ChatGptAuthError> {
    let status = response.status();
    let request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .map(str::to_owned);
    if !status.is_success() {
        let body = response.json::<Value>().unwrap_or(Value::Null);
        let code = body
            .get("error")
            .and_then(|e| e.as_str())
            .or_else(|| body.pointer("/error/code").and_then(Value::as_str));
        let message = code
            .map(|c| format!("OpenAI OAuth respondeu HTTP {status} ({c})."))
            .unwrap_or_else(|| format!("OpenAI OAuth respondeu HTTP {status}."));
        return Err(ChatGptAuthError::OAuth(match request_id {
            Some(id) => format!("{message} Request ID: {id}"),
            None => message,
        }));
    }
    response.json().map_err(|_| {
        ChatGptAuthError::InvalidResponse("Resposta OpenAI em formato inesperado.".into())
    })
}

fn http_error(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    code: Option<&str>,
) -> ChatGptAuthError {
    let request_id = headers.get("x-request-id").and_then(|h| h.to_str().ok());
    let message = code
        .map(|c| format!("OpenAI respondeu HTTP {status} ({c})."))
        .unwrap_or_else(|| format!("OpenAI respondeu HTTP {status}."));
    ChatGptAuthError::OAuth(match request_id {
        Some(id) => format!("{message} Request ID: {id}"),
        None => message,
    })
}

fn classify_refresh_error(error: ChatGptAuthError) -> ChatGptAuthError {
    const INVALID_REFRESH: &[&str] = &[
        "invalid_grant",
        "invalid_refresh_token",
        "token_expired",
        "refresh_token_expired",
        "refresh_token_invalidated",
        "refresh_token_reused",
    ];
    match &error {
        ChatGptAuthError::OAuth(message)
            if INVALID_REFRESH
                .iter()
                .any(|code| message.contains(&format!("({code})"))) =>
        {
            ChatGptAuthError::ReauthenticationRequired(format!(
                "A sessão ChatGPT expirou. Entre novamente. {message}"
            ))
        }
        _ => error,
    }
}

fn is_invalid_user_error(error: &ChatGptAuthError) -> bool {
    matches!(error, ChatGptAuthError::OAuth(message) if message.contains("(subscription_sharing_invalid_user)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_permission_requires_exact_granted_scope() {
        assert!(has_plan_scope(&[PLAN_SCOPE.into(), "openid".into()]));
        assert!(!has_plan_scope(&[
            "openid".into(),
            "resource.invoke".into()
        ]));
        assert!(!has_plan_scope(&["chatgpt.tokens.use.direct.extra".into()]));
    }

    #[test]
    fn pkce_tokens_are_url_safe_and_have_required_length() {
        let v = random_url_token(32).unwrap();
        assert_eq!(v.len(), 43);
        assert!(v
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-'));
    }

    #[test]
    fn loopback_callback_rejects_wrong_state_and_path() {
        for target in [
            "/auth/callback?state=wrong&code=x",
            "/other?state=expected&code=x",
        ] {
            let request = format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
            assert!(parse_callback_request(&request, "expected").is_err());
        }
    }
}

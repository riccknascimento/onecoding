//! Cliente HTTP do OneNode para o OneHub Central. Toda chamada leva a identidade do nó. Erros não ecoam
//! o corpo da resposta (que pode conter dados).

use serde_json::{json, Value};

#[derive(Debug, PartialEq, Eq)]
pub enum CentralError {
    /// Token inválido ou nó revogado: o agente deve parar.
    Unauthorized,
    /// Lease vencido, tarefa já finalizada ou evento duplicado.
    Conflict(String),
    /// Falha de rede ou erro do servidor: pode tentar de novo com espera.
    Transient(String),
    /// Nó suspenso pelo administrador: não é revogação. O serviço espera e tenta de novo depois.
    Suspended,
}

/// Catálogo enviado pelo OneHub: versão e projetos com raiz pedida.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CatalogSnapshot {
    pub version: i64,
    pub projects: Vec<CatalogProject>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CatalogProject {
    pub project_id: String,
    pub name: String,
    pub root_path: String,
}

#[derive(Debug, Clone)]
pub struct Task {
    pub id: String,
    pub project_id: String,
    pub tool: Option<String>,
    pub args: Value,
    pub lease_token: String,
}

pub struct Central {
    base: String,
    token: String,
    http: reqwest::blocking::Client,
}

impl Central {
    pub fn new(base: &str, token: &str) -> Result<Self, String> {
        let http = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|_| "não foi possível criar o cliente HTTP".to_string())?;
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
            http,
        })
    }

    fn map(status: reqwest::StatusCode) -> Option<CentralError> {
        match status.as_u16() {
            200..=299 => None,
            401 | 403 => Some(CentralError::Unauthorized),
            409 => Some(CentralError::Conflict("conflito".into())),
            423 => Some(CentralError::Suspended),
            _ => Some(CentralError::Transient(format!("HTTP {}", status.as_u16()))),
        }
    }

    fn transport(e: reqwest::Error) -> CentralError {
        CentralError::Transient(if e.is_timeout() {
            "tempo esgotado".into()
        } else {
            "conexão indisponível".into()
        })
    }

    pub fn pair(base: &str, node_id: &str, code: &str) -> Result<String, CentralError> {
        let http = reqwest::blocking::Client::new();
        let res = http
            .post(format!("{}/node/v1/pair", base.trim_end_matches('/')))
            .json(&json!({ "node_id": node_id, "code": code }))
            .send()
            .map_err(Self::transport)?;
        if let Some(e) = Self::map(res.status()) {
            return Err(e);
        }
        let v: Value = res
            .json()
            .map_err(|_| CentralError::Transient("resposta inválida".into()))?;
        v.get("token")
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or(CentralError::Transient("resposta sem token".into()))
    }

    /// Catálogo atual deste OneNode (projetos com concessão ativa e raiz pedida).
    pub fn catalog(&self) -> Result<CatalogSnapshot, CentralError> {
        let res = self
            .http
            .get(format!("{}/node/v1/catalog", self.base))
            .bearer_auth(&self.token)
            .send()
            .map_err(Self::transport)?;
        if let Some(e) = Self::map(res.status()) {
            return Err(e);
        }
        res.json()
            .map_err(|_| CentralError::Transient("catálogo inválido".into()))
    }

    /// Confirma a versão aplicada, com o que foi aceito e o que foi recusado (motivo curto).
    pub fn report_applied(
        &self,
        version: i64,
        accepted: &[String],
        rejected: &[(String, String)],
    ) -> Result<(), CentralError> {
        let rejected: Vec<Value> = rejected
            .iter()
            .map(|(id, reason)| json!({ "project_id": id, "reason": reason }))
            .collect();
        let res = self
            .http
            .post(format!("{}/node/v1/catalog/applied", self.base))
            .bearer_auth(&self.token)
            .json(&json!({ "version": version, "accepted": accepted, "rejected": rejected }))
            .send()
            .map_err(Self::transport)?;
        Self::map(res.status()).map_or(Ok(()), Err)
    }

    pub fn heartbeat(&self) -> Result<(), CentralError> {
        let res = self
            .http
            .post(format!("{}/node/v1/heartbeat", self.base))
            .bearer_auth(&self.token)
            .send()
            .map_err(Self::transport)?;
        Self::map(res.status()).map_or(Ok(()), Err)
    }

    pub fn next_task(&self) -> Result<Option<Task>, CentralError> {
        let res = self
            .http
            .get(format!("{}/node/v1/tasks/next", self.base))
            .bearer_auth(&self.token)
            .send()
            .map_err(Self::transport)?;
        if res.status().as_u16() == 204 {
            return Ok(None);
        }
        if let Some(e) = Self::map(res.status()) {
            return Err(e);
        }
        let v: Value = res
            .json()
            .map_err(|_| CentralError::Transient("tarefa inválida".into()))?;
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(String::from);
        Ok(Some(Task {
            id: text("id").ok_or(CentralError::Transient("tarefa sem id".into()))?,
            project_id: text("project_id")
                .ok_or(CentralError::Transient("tarefa sem projeto".into()))?,
            tool: text("tool"),
            args: v.get("args").cloned().unwrap_or_else(|| json!({})),
            lease_token: text("lease_token")
                .ok_or(CentralError::Transient("tarefa sem lease".into()))?,
        }))
    }

    pub fn event(
        &self,
        task: &str,
        lease: &str,
        seq: i64,
        kind: &str,
        payload: Value,
    ) -> Result<(), CentralError> {
        let res = self
            .http
            .post(format!("{}/node/v1/tasks/{task}/events", self.base))
            .bearer_auth(&self.token)
            .json(&json!({ "lease_token": lease, "seq": seq, "kind": kind, "payload": payload }))
            .send()
            .map_err(Self::transport)?;
        Self::map(res.status()).map_or(Ok(()), Err)
    }

    pub fn finish(
        &self,
        task: &str,
        lease: &str,
        status: &str,
        result: Value,
    ) -> Result<(), CentralError> {
        let res = self
            .http
            .post(format!("{}/node/v1/tasks/{task}/finish", self.base))
            .bearer_auth(&self.token)
            .json(&json!({ "lease_token": lease, "status": status, "result": result }))
            .send()
            .map_err(Self::transport)?;
        Self::map(res.status()).map_or(Ok(()), Err)
    }
}

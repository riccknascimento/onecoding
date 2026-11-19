//! Cliente HTTP do OneExecutor para o OneHub Central. Chamadas bloqueantes (usadas fora do runtime async).

use serde_json::{json, Value};

#[derive(Debug, PartialEq, Eq)]
pub enum CentralError {
    Unauthorized,
    Conflict(String),
    Transient(String),
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: String,
    pub question: String,
    pub lease_token: String,
    /// "analysis" (snapshot de projeto) ou "delegation" (agente, workspace vazio, somente leitura).
    pub kind: String,
}

/// Ferramentas remotas do OneNode, pedidas pelo Central. Trait para permitir teste sem rede.
pub trait ToolBroker {
    fn request_tool(
        &self,
        job: &str,
        lease: &str,
        tool: &str,
        path: &str,
    ) -> Result<String, CentralError>;
    fn tool_status(&self, job: &str, task: &str) -> Result<(String, Value), CentralError>;
}

pub struct Central {
    base: String,
    token: String,
    http: reqwest::blocking::Client,
}

fn map(status: reqwest::StatusCode) -> Option<CentralError> {
    match status.as_u16() {
        200..=299 => None,
        401 | 403 => Some(CentralError::Unauthorized),
        409 => Some(CentralError::Conflict("conflito".into())),
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

impl Central {
    pub fn new(base: &str, token: &str) -> Result<Self, String> {
        let http = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|_| "não foi possível criar o cliente HTTP".to_string())?;
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
            http,
        })
    }

    pub fn pair(base: &str, executor_id: &str, code: &str) -> Result<String, CentralError> {
        let http = reqwest::blocking::Client::new();
        let res = http
            .post(format!("{}/executor/v1/pair", base.trim_end_matches('/')))
            .json(&json!({ "executor_id": executor_id, "code": code }))
            .send()
            .map_err(transport)?;
        if let Some(e) = map(res.status()) {
            return Err(e);
        }
        let v: Value = res
            .json()
            .map_err(|_| CentralError::Transient("resposta inválida".into()))?;
        v.get("token")
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or(CentralError::Transient("sem token".into()))
    }

    fn post(&self, path: &str, body: Value) -> Result<reqwest::blocking::Response, CentralError> {
        let res = self
            .http
            .post(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .map_err(transport)?;
        match map(res.status()) {
            Some(e) => Err(e),
            None => Ok(res),
        }
    }

    /// Heartbeat com o estado dos motores (instalado, autenticado, aprovado). Lista sempre enviada.
    pub fn heartbeat(&self, engines: &[crate::engines::EngineReport]) -> Result<(), CentralError> {
        self.post("/executor/v1/heartbeat", json!(engines)).map(|_| ())
    }

    pub fn next_job(&self) -> Result<Option<Job>, CentralError> {
        let res = self
            .http
            .post(format!("{}/executor/v1/jobs/next", self.base))
            .bearer_auth(&self.token)
            .send()
            .map_err(transport)?;
        if res.status().as_u16() == 204 {
            return Ok(None);
        }
        if let Some(e) = map(res.status()) {
            return Err(e);
        }
        let v: Value = res
            .json()
            .map_err(|_| CentralError::Transient("job inválido".into()))?;
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(String::from);
        Ok(Some(Job {
            id: text("id").ok_or(CentralError::Transient("job sem id".into()))?,
            question: text("question").unwrap_or_default(),
            lease_token: text("lease_token")
                .ok_or(CentralError::Transient("job sem lease".into()))?,
            kind: text("kind").unwrap_or_else(|| "analysis".into()),
        }))
    }

    pub fn event(
        &self,
        job: &str,
        lease: &str,
        kind: &str,
        payload: Value,
    ) -> Result<(), CentralError> {
        self.post(
            &format!("/executor/v1/jobs/{job}/events"),
            json!({ "lease_token": lease, "kind": kind, "payload": payload }),
        )
        .map(|_| ())
    }

    /// Renova o lease do job e informa se o dono pediu cancelamento.
    pub fn job_heartbeat(&self, job: &str, lease: &str) -> Result<bool, CentralError> {
        let res = self.post(
            &format!("/executor/v1/jobs/{job}/heartbeat"),
            json!({ "lease_token": lease }),
        )?;
        let v: Value = res
            .json()
            .map_err(|_| CentralError::Transient("resposta inválida".into()))?;
        Ok(v.get("cancel_requested")
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }

    /// Confirma o cancelamento ao Central. Só é aceito quando o dono pediu cancelamento.
    pub fn finish_cancelled(&self, job: &str, lease: &str) -> Result<(), CentralError> {
        self.post(
            &format!("/executor/v1/jobs/{job}/finish"),
            json!({ "lease_token": lease, "status": "cancelled" }),
        )
        .map(|_| ())
    }

    pub fn finish(
        &self,
        job: &str,
        lease: &str,
        ok: bool,
        answer: Option<&str>,
        evidence: &[String],
        error: Option<&str>,
    ) -> Result<(), CentralError> {
        let body = json!({
            "lease_token": lease,
            "status": if ok { "succeeded" } else { "failed" },
            "answer": answer,
            "evidence": evidence,
            "error": error,
        });
        self.post(&format!("/executor/v1/jobs/{job}/finish"), body)
            .map(|_| ())
    }
}

impl ToolBroker for Central {
    fn request_tool(
        &self,
        job: &str,
        lease: &str,
        tool: &str,
        path: &str,
    ) -> Result<String, CentralError> {
        let res = self.post(
            &format!("/executor/v1/jobs/{job}/tools"),
            json!({ "lease_token": lease, "tool": tool, "path": path }),
        )?;
        let v: Value = res
            .json()
            .map_err(|_| CentralError::Transient("resposta inválida".into()))?;
        v.get("task_id")
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or(CentralError::Transient("sem tarefa".into()))
    }

    fn tool_status(&self, job: &str, task: &str) -> Result<(String, Value), CentralError> {
        let res = self
            .http
            .get(format!("{}/executor/v1/jobs/{job}/tasks/{task}", self.base))
            .bearer_auth(&self.token)
            .send()
            .map_err(transport)?;
        if let Some(e) = map(res.status()) {
            return Err(e);
        }
        let v: Value = res
            .json()
            .map_err(|_| CentralError::Transient("resposta inválida".into()))?;
        Ok((
            v.get("status")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            v.get("result").cloned().unwrap_or(Value::Null),
        ))
    }
}

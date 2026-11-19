//! Instalação do OneNode num servidor Linux, em passos idempotentes. Cada passo devolve um resumo curto e
//! sanitizado (nunca a saída bruta do servidor). Credenciais e código de pareamento não entram em argumentos
//! de processo: vão pela entrada padrão do canal SSH e são apagados logo depois.

use std::path::Path;

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::ssh::{Auth, ExecOut, Session, SshError};

pub const MIN_GLIBC: (u32, u32) = (2, 31);
pub const BIN_DIR: &str = "/opt/onenode/bin";
pub const STATE_FILE: &str = "/opt/onenode/state/identity.json";
pub const WORKSPACE: &str = "/opt/onenode/workspace";
pub const CONFIG_FILE: &str = "/etc/onenode/node.json";
pub const POLICY_FILE: &str = "/etc/onenode/policy.json";
pub const PAIR_CODE_FILE: &str = "/etc/onenode/pair.code";
pub const UNIT_FILE: &str = "/etc/systemd/system/onenode.service";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StepStatus {
    Ok,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Step {
    pub name: &'static str,
    pub status: StepStatus,
    pub detail: String,
}

/// Dados que a instalação precisa. A chave de pareamento só existe aqui, em memória.
pub struct InstallSpec<'a> {
    pub central_url: &'a str,
    pub node_id: Uuid,
    /// `None` quando o servidor já tem identidade do mesmo OneNode: reaproveita, sem novo pareamento.
    pub pairing_code: Option<&'a str>,
    pub binary: &'a Path,
    pub binary_sha256: &'a str,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Preflight {
    pub arch: String,
    pub os: String,
    pub glibc: (u32, u32),
    pub systemd: bool,
    pub root: bool,
    /// Bundle de CAs do sistema presente. Se ausente, o instalador o instala antes do OneNode.
    pub ca_bundle: bool,
}

/// Quoting POSIX seguro para valores que vão dentro de `sh -c`.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Comando de preflight: uma linha por informação, sem nada sensível.
pub const PREFLIGHT_CMD: &str = "uname -m; . /etc/os-release 2>/dev/null; echo \"$ID $VERSION_ID\"; ldd --version 2>&1 | head -n1; \
     if [ -d /run/systemd/system ]; then echo systemd=yes; else echo systemd=no; fi; id -u; \
     if [ -s /etc/ssl/certs/ca-certificates.crt ]; then echo ca=yes; else echo ca=no; fi";

/// Pré-requisito que o instalador instala quando falta: sem CAs do sistema o `one-node` não cria o cliente HTTP.
pub const INSTALL_CA_CMD: &str =
    "apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends ca-certificates";

pub fn parse_glibc(line: &str) -> Option<(u32, u32)> {
    let version = line.split_whitespace().last()?;
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

pub fn parse_preflight(text: &str) -> Result<Preflight, String> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 6 {
        return Err("resposta incompleta do servidor no preflight".into());
    }
    let arch = lines[0].trim().to_string();
    let os = lines[1].trim().to_string();
    let glibc = parse_glibc(lines[2]).ok_or("não foi possível ler a versão do glibc")?;
    let systemd = lines[3].trim() == "systemd=yes";
    let root = lines[4].trim() == "0";
    let ca_bundle = lines[5].trim() == "ca=yes";
    Ok(Preflight { arch, os, glibc, systemd, root, ca_bundle })
}

/// Valida o preflight e devolve a mensagem de erro acionável, se houver.
pub fn check_preflight(p: &Preflight) -> Result<(), String> {
    if p.arch != "x86_64" {
        return Err(format!("arquitetura {} não suportada (necessário x86_64)", p.arch));
    }
    if p.glibc < MIN_GLIBC {
        return Err(format!(
            "glibc {}.{} é antiga (mínimo {}.{})",
            p.glibc.0, p.glibc.1, MIN_GLIBC.0, MIN_GLIBC.1
        ));
    }
    if !p.systemd {
        return Err("systemd não encontrado neste servidor".into());
    }
    if !p.root {
        return Err("sem privilégio de administrador: use root ou um usuário com sudo".into());
    }
    Ok(())
}

/// Lê o node_id gravado na identidade existente, sem tocar no token.
pub fn parse_identity_node_id(text: &str) -> Option<String> {
    let idx = text.find("\"node_id\"")?;
    let rest = &text[idx + "\"node_id\"".len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let end = rest.find('"')?;
    let id = &rest[..end];
    Uuid::parse_str(id).ok().map(|_| id.to_string())
}

pub fn config_json(central_url: &str) -> String {
    serde_json::json!({
        "central_url": central_url,
        "state_file": STATE_FILE,
        "projects": []
    })
    .to_string()
}

/// Política inicial: só a pasta de teste. Nunca sobrescrita se já existir (o administrador a controla).
pub fn default_policy_json() -> String {
    serde_json::json!({ "allowed_roots": [WORKSPACE] }).to_string()
}

pub fn unit_file() -> String {
    format!(
        "[Unit]\n\
         Description=OneNode (OneCoding) - agente somente leitura\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         StartLimitIntervalSec=300\n\
         StartLimitBurst=5\n\
         \n\
         [Service]\n\
         User=onenode\n\
         Group=onenode\n\
         Environment=ONEAGENT_NODE_CONFIG={CONFIG_FILE}\n\
         ExecStart={BIN_DIR}/one-node run\n\
         Restart=on-failure\n\
         RestartSec=15\n\
         NoNewPrivileges=yes\n\
         ProtectSystem=strict\n\
         ProtectHome=yes\n\
         PrivateTmp=yes\n\
         ReadWritePaths=/opt/onenode/state\n\
         ReadOnlyPaths=/opt/onenode/workspace /etc/onenode\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n"
    )
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Remove segredos de qualquer texto que vá ao navegador.
pub fn redact(text: &str, secrets: &[&str]) -> String {
    let mut out = text.to_string();
    for s in secrets {
        if !s.is_empty() {
            out = out.replace(s, "<redigido>");
        }
    }
    out.chars().take(300).collect()
}

/// Executor remoto: envolve comandos em sudo quando o login não é root, sem nunca montar a senha em argv.
pub struct Remote<'a> {
    pub session: &'a mut Session,
    pub user: &'a str,
    pub auth: &'a Auth,
}

impl<'a> Remote<'a> {
    fn is_root(&self) -> bool {
        self.user == "root"
    }

    /// Executa `cmd` como administrador. `payload` vai pela entrada padrão.
    /// Com senha e sudo: a primeira linha da entrada é a senha, lida pelo `sudo -S`.
    pub async fn admin(&mut self, cmd: &str, payload: Option<&[u8]>) -> Result<ExecOut, SshError> {
        if self.is_root() {
            return self.session.exec(cmd, payload).await;
        }
        let wrapped = format!("sudo -S -p '' sh -c {}", sh_quote(cmd));
        match self.auth {
            Auth::Password(pw) => {
                let mut input = pw.expose().as_bytes().to_vec();
                input.push(b'\n');
                if let Some(p) = payload {
                    input.extend_from_slice(p);
                }
                let out = self.session.exec(&wrapped, Some(&input)).await;
                zeroize_vec(&mut input);
                out
            }
            Auth::Key { .. } => {
                let wrapped = format!("sudo -n sh -c {}", sh_quote(cmd));
                self.session.exec(&wrapped, payload).await
            }
        }
    }
}

fn zeroize_vec(v: &mut Vec<u8>) {
    use zeroize::Zeroize;
    v.zeroize();
}

fn report(on_step: &mut impl FnMut(&Step), name: &'static str, status: StepStatus, detail: String) {
    on_step(&Step { name, status, detail });
}

/// Executa a instalação completa, reportando cada passo ao chamador.
pub async fn install(
    remote: &mut Remote<'_>,
    spec: &InstallSpec<'_>,
    mut on_step: impl FnMut(&Step),
) -> Result<(), String> {
    let secrets: Vec<&str> = spec.pairing_code.into_iter().collect();

    // 1. preflight
    let pre = remote.session.exec(PREFLIGHT_CMD, None).await.map_err(|e| e.to_string())?;
    let parsed = match parse_preflight(&pre.stdout_text()) {
        Ok(p) => p,
        Err(e) => {
            report(&mut on_step, "preflight", StepStatus::Failed, e.clone());
            return Err(e);
        }
    };
    // root real é verificado pelo id -u remoto, já que o sudo pode ter sido necessário
    let mut parsed = parsed;
    parsed.root = parsed.root || remote.is_root();
    if let Err(e) = check_preflight(&parsed) {
        report(&mut on_step, "preflight", StepStatus::Failed, e.clone());
        return Err(e);
    }
    report(
        &mut on_step,
        "preflight",
        StepStatus::Ok,
        format!("{} · {} · glibc {}.{} · systemd", parsed.os, parsed.arch, parsed.glibc.0, parsed.glibc.1),
    );

    // 1b. CAs do sistema: sem elas o one-node não consegue criar o cliente HTTP do OneHub
    if !parsed.ca_bundle {
        let out = remote.admin(INSTALL_CA_CMD, None).await.map_err(|e| e.to_string())?;
        if !out.ok() {
            let e = "não foi possível instalar ca-certificates (pré-requisito do OneNode)".to_string();
            report(&mut on_step, "ca_certificates", StepStatus::Failed, e.clone());
            return Err(e);
        }
        report(&mut on_step, "ca_certificates", StepStatus::Ok, "ca-certificates instalado".into());
    } else {
        report(&mut on_step, "ca_certificates", StepStatus::Skipped, "já presente".into());
    }

    // 2. usuário e pastas (idempotente)
    let dirs = format!(
        "id -u onenode >/dev/null 2>&1 || useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin onenode && \
         install -d -m 0755 {BIN_DIR} && install -d -m 0700 -o onenode -g onenode /opt/onenode/state && \
         install -d -m 0755 {WORKSPACE} && install -d -m 0750 -o root -g onenode /etc/onenode"
    );
    let out = remote.admin(&dirs, None).await.map_err(|e| e.to_string())?;
    if !out.ok() {
        let e = "não foi possível criar usuário e pastas".to_string();
        report(&mut on_step, "usuario_e_pastas", StepStatus::Failed, e.clone());
        return Err(e);
    }
    report(&mut on_step, "usuario_e_pastas", StepStatus::Ok, "usuário onenode e pastas prontos".into());

    // 3. identidade existente: reaproveita o mesmo OneNode; nunca duplica com outro
    let ident = remote
        .admin(&format!("test -f {STATE_FILE} && cat {STATE_FILE} || true"), None)
        .await
        .map_err(|e| e.to_string())?;
    let existing = ident.stdout_text();
    let existing_id = if existing.is_empty() { None } else { parse_identity_node_id(&existing) };
    let reuse = match (existing.is_empty(), existing_id) {
        (true, _) => false,
        (false, Some(id)) if id == spec.node_id.to_string() => true,
        (false, Some(_)) => {
            let e = "servidor já pareado com outro OneNode: remova a identidade antiga antes de reutilizá-lo".to_string();
            report(&mut on_step, "identidade", StepStatus::Failed, e.clone());
            return Err(e);
        }
        (false, None) => {
            let e = "identidade existente ilegível: não será sobrescrita".to_string();
            report(&mut on_step, "identidade", StepStatus::Failed, e.clone());
            return Err(e);
        }
    };
    report(
        &mut on_step,
        "identidade",
        if reuse { StepStatus::Ok } else { StepStatus::Skipped },
        if reuse { "identidade existente reaproveitada".into() } else { "nenhuma identidade anterior".into() },
    );

    // 4. binário, com verificação de sha256 no servidor
    let bin = std::fs::read(spec.binary).map_err(|_| "binário do OneNode indisponível no provisionador".to_string())?;
    if sha256_hex(&bin) != spec.binary_sha256 {
        let e = "binário local não confere com o sha256 esperado".to_string();
        report(&mut on_step, "binario", StepStatus::Failed, e.clone());
        return Err(e);
    }
    let upload = format!(
        "cat > {BIN_DIR}/one-node.new && chmod 0755 {BIN_DIR}/one-node.new && \
         test \"$(sha256sum {BIN_DIR}/one-node.new | cut -d' ' -f1)\" = {} && \
         ( [ -f {BIN_DIR}/one-node ] && cp -p {BIN_DIR}/one-node {BIN_DIR}/one-node.prev || true ) && \
         mv -f {BIN_DIR}/one-node.new {BIN_DIR}/one-node",
        sh_quote(spec.binary_sha256)
    );
    let out = remote.admin(&upload, Some(&bin)).await.map_err(|e| e.to_string())?;
    if !out.ok() {
        let _ = remote.admin(&format!("rm -f {BIN_DIR}/one-node.new"), None).await;
        let e = "envio do binário falhou ou sha256 não confere".to_string();
        report(&mut on_step, "binario", StepStatus::Failed, e.clone());
        return Err(e);
    }
    report(&mut on_step, "binario", StepStatus::Ok, format!("sha256 {}", &spec.binary_sha256[..12]));

    // 5. configuração e política local. Cada arquivo tem a própria entrada padrão: nada se mistura.
    let cfg_cmd = format!("umask 027 && cat > {CONFIG_FILE} && chown root:onenode {CONFIG_FILE} && chmod 0640 {CONFIG_FILE}");
    let mut cfg_payload = config_json(spec.central_url).into_bytes();
    cfg_payload.push(b'\n');
    let out = remote.admin(&cfg_cmd, Some(&cfg_payload)).await.map_err(|e| e.to_string())?;
    if !out.ok() {
        let e = "não foi possível gravar a configuração".to_string();
        report(&mut on_step, "configuracao", StepStatus::Failed, e.clone());
        return Err(e);
    }
    // a política existente nunca é sobrescrita: quem a controla é o administrador do servidor
    let pol_cmd = format!("test -f {POLICY_FILE} || (umask 022 && cat > {POLICY_FILE} && chmod 0644 {POLICY_FILE})");
    let mut pol_payload = default_policy_json().into_bytes();
    pol_payload.push(b'\n');
    let out = remote.admin(&pol_cmd, Some(&pol_payload)).await.map_err(|e| e.to_string())?;
    if !out.ok() {
        let e = "não foi possível gravar a política local".to_string();
        report(&mut on_step, "configuracao", StepStatus::Failed, e.clone());
        return Err(e);
    }
    report(&mut on_step, "configuracao", StepStatus::Ok, "configuração e política local gravadas".into());

    // 6. pareamento automático (apenas sem identidade reaproveitada)
    if reuse {
        report(&mut on_step, "pareamento", StepStatus::Skipped, "não necessário: identidade existente".into());
    } else {
        let code = spec.pairing_code.ok_or("código de pareamento ausente")?;
        let pair_cmd = format!(
            "umask 077 && cat > {PAIR_CODE_FILE} && chown onenode {PAIR_CODE_FILE} && \
             runuser -u onenode -- env ONEAGENT_NODE_CONFIG={CONFIG_FILE} ONEAGENT_NODE_ID={} {BIN_DIR}/one-node pair < {PAIR_CODE_FILE}; \
             rc=$?; rm -f {PAIR_CODE_FILE}; exit $rc",
            spec.node_id
        );
        let out = remote.admin(&pair_cmd, Some(code.as_bytes())).await.map_err(|e| e.to_string())?;
        // limpeza defensiva: o arquivo com o código nunca pode sobrar
        let _ = remote.admin(&format!("rm -f {PAIR_CODE_FILE}"), None).await;
        if !out.ok() {
            let e = redact(&format!("{} {}", out.stdout_text(), String::from_utf8_lossy(&out.stderr)), &secrets);
            let e = if e.trim().is_empty() { "pareamento recusado pelo OneHub".to_string() } else { e };
            report(&mut on_step, "pareamento", StepStatus::Failed, e.clone());
            return Err(e);
        }
        report(&mut on_step, "pareamento", StepStatus::Ok, "OneNode pareado com o OneHub".into());
    }

    // 7. serviço systemd: só é habilitado e iniciado depois de tudo acima
    let unit = unit_file();
    let svc = format!(
        "umask 022 && cat > {UNIT_FILE} && systemctl daemon-reload && systemctl enable onenode >/dev/null 2>&1 && systemctl restart onenode"
    );
    let out = remote.admin(&svc, Some(unit.as_bytes())).await.map_err(|e| e.to_string())?;
    if !out.ok() {
        let e = "não foi possível habilitar o serviço systemd".to_string();
        report(&mut on_step, "servico", StepStatus::Failed, e.clone());
        return Err(e);
    }
    report(&mut on_step, "servico", StepStatus::Ok, "serviço onenode habilitado e iniciado".into());

    // 8. processo ativo (o heartbeat é confirmado pelo OneHub, não por este passo)
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let active = remote.admin("systemctl is-active onenode", None).await.map_err(|e| e.to_string())?;
    if active.stdout_text() != "active" {
        let e = "o serviço não ficou ativo: veja os logs do OneNode no servidor".to_string();
        report(&mut on_step, "servico_ativo", StepStatus::Failed, e.clone());
        return Err(e);
    }
    report(&mut on_step, "servico_ativo", StepStatus::Ok, "processo ativo".into());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_neutralizes_single_quotes() {
        assert_eq!(sh_quote("a'b"), "'a'\\''b'");
        assert_eq!(sh_quote("sem aspas"), "'sem aspas'");
    }

    #[test]
    fn preflight_accepts_supported_and_names_each_requirement() {
        let ok = parse_preflight("x86_64\nubuntu 22.04\nldd (Ubuntu GLIBC 2.35-0ubuntu3) 2.35\nsystemd=yes\n0\nca=yes\n").unwrap();
        assert!(check_preflight(&ok).is_ok());
        let arm = Preflight { arch: "aarch64".into(), ..ok };
        assert!(check_preflight(&arm).unwrap_err().contains("arquitetura"));
        let old = Preflight { glibc: (2, 17), arch: "x86_64".into(), os: "x".into(), systemd: true, root: true, ca_bundle: true };
        assert!(check_preflight(&old).unwrap_err().contains("glibc"));
        let nosd = Preflight { systemd: false, ..old };
        assert!(check_preflight(&Preflight { glibc: (2, 35), ..nosd }).unwrap_err().contains("systemd"));
    }

    #[test]
    fn preflight_reports_missing_ca_bundle_so_the_installer_can_fix_it() {
        let text = "x86_64\nubuntu 22.04\nldd (Ubuntu GLIBC 2.35) 2.35\nsystemd=yes\n0\nca=no\n";
        let p = parse_preflight(text).unwrap();
        assert!(!p.ca_bundle);
        assert!(check_preflight(&p).is_ok(), "ausência de CA é corrigível, não bloqueia");
        assert!(parse_preflight("x86_64\nubuntu\nldd (x) 2.35\nsystemd=yes\n0\nca=yes\n").unwrap().ca_bundle);
    }

    #[test]
    fn identity_node_id_is_read_without_exposing_the_token() {
        let text = r#"{"node_id":"5b45f0f9-8748-4116-898b-f5d80333691b","token":"segredo-do-token"}"#;
        assert_eq!(parse_identity_node_id(text).as_deref(), Some("5b45f0f9-8748-4116-898b-f5d80333691b"));
        assert_eq!(parse_identity_node_id("{\"node_id\":\"nao-e-uuid\"}"), None);
        assert_eq!(parse_identity_node_id("{}"), None);
    }

    #[test]
    fn generated_config_has_no_projects_and_fixed_state_path() {
        let cfg: serde_json::Value = serde_json::from_str(&config_json("https://hub.oneagente.tech")).unwrap();
        assert_eq!(cfg["projects"], serde_json::json!([]));
        assert_eq!(cfg["state_file"], serde_json::json!(STATE_FILE));
        assert_eq!(cfg["central_url"], serde_json::json!("https://hub.oneagente.tech"));
    }

    #[test]
    fn unit_uses_the_hardened_profile_and_never_runs_as_root() {
        let u = unit_file();
        assert!(u.contains("User=onenode"));
        assert!(u.contains("NoNewPrivileges=yes"));
        assert!(u.contains("ProtectSystem=strict"));
        assert!(u.contains("ReadOnlyPaths=/opt/onenode/workspace /etc/onenode"));
        assert!(!u.contains("User=root"));
    }

    #[test]
    fn redaction_removes_the_pairing_code_from_any_message() {
        let msg = redact("erro com zXeqNgKVu1 dentro", &["zXeqNgKVu1"]);
        assert!(!msg.contains("zXeqNgKVu1"));
        assert!(msg.contains("<redigido>"));
    }
}

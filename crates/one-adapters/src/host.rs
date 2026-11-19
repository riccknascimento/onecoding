//! Acesso ao sistema: descobrir binários (sem executar) e executar os JÁ CONFIÁVEIS, com limites.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use std::sync::atomic::AtomicBool;

use crate::proc::{run_bounded, run_streaming, Limits, RunOutput};
use crate::trust::{self, Inspected, TrustedExe};

pub trait Host: Sync {
    /// Descobre e inspeciona o binário `name`. NUNCA executa nada.
    fn inspect(&self, name: &str) -> Option<Inspected>;
    /// Executa um binário já aprovado. Sem shell; grupo de processos próprio; com prazo e limite.
    fn run(&self, exe: &TrustedExe, args: &[&str], limits: Limits) -> RunOutput;
    /// Retorna um host isolado para o HOME de uma conta, quando o host suporta perfis.
    fn with_profile_home(&self, _home: &Path) -> Option<Box<dyn Host>> {
        None
    }
    /// Execução em streaming (eventos em tempo real), com stdin e diretório de trabalho. A saída
    /// chega em `on_data`; `cancel` encerra o grupo de processos.
    fn run_stream(
        &self,
        _exe: &TrustedExe,
        _spec: &StreamSpec,
        _cancel: Option<&AtomicBool>,
        _on_data: &mut dyn FnMut(&[u8]),
    ) -> RunOutput {
        RunOutput {
            stdout: String::new(),
            stderr: String::new(),
            outcome: crate::proc::Outcome::SpawnFailed,
        }
    }
}

/// Parâmetros de uma execução em streaming. Só dados: não há campo para shell.
pub struct StreamSpec {
    pub args: Vec<String>,
    pub stdin: Vec<u8>,
    pub cwd: PathBuf,
    pub limits: Limits,
}

/// Implementação real. Apps abertos pelo Finder não herdam o PATH do terminal (nvm, ~/.local/bin),
/// então os diretórios usuais são pesquisados explicitamente. Achar ≠ confiar: veja `trust`.
pub struct SystemHost {
    /// Onde procurar as CLIs (diretórios do usuário primeiro).
    search_dirs: Vec<PathBuf>,
    /// Onde resolver interpretadores (`#!/usr/bin/env bash`) e o PATH do filho: diretórios do
    /// sistema primeiro (protegidos), para que um `bash`/`node` em pasta gravável não os sombreie.
    runtime_dirs: Vec<PathBuf>,
    /// Perfil isolado de uma conta: HOME e diretórios XDG próprios. `None` = ambiente do serviço.
    profile_home: Option<PathBuf>,
}

const SYSTEM_DIRS: [&str; 4] = ["/usr/bin", "/bin", "/usr/sbin", "/sbin"];

fn system_first(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = SYSTEM_DIRS.iter().map(PathBuf::from).collect();
    v.extend(
        dirs.iter()
            .filter(|d| !SYSTEM_DIRS.iter().any(|s| d.as_path() == Path::new(s)))
            .cloned(),
    );
    v
}

impl SystemHost {
    pub fn new() -> Self {
        let mut dirs: Vec<PathBuf> = vec![];
        if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
            dirs.push(home.join(".local/bin"));
            if let Ok(rd) = std::fs::read_dir(home.join(".nvm/versions/node")) {
                let mut nodes: Vec<PathBuf> = rd.flatten().map(|e| e.path().join("bin")).collect();
                nodes.sort();
                nodes.reverse();
                dirs.extend(nodes);
            }
            dirs.push(home.join(".cargo/bin"));
        }
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from));
        if let Some(p) = env::var_os("PATH") {
            dirs.extend(env::split_paths(&p));
        }
        Self::with_dirs(dirs)
    }

    /// Diretórios explícitos (testes).
    pub fn with_dirs(search_dirs: Vec<PathBuf>) -> Self {
        let runtime_dirs = system_first(&search_dirs);
        Self {
            search_dirs,
            runtime_dirs,
            profile_home: None,
        }
    }

    /// Mesmo host (mesmos diretórios de busca e de execução), com o login de uma conta isolada em `home`.
    /// A aprovação do binário continua global; só as credenciais da CLI mudam de conta para conta.
    pub fn with_profile_home(&self, home: PathBuf) -> Self {
        Self {
            search_dirs: self.search_dirs.clone(),
            runtime_dirs: self.runtime_dirs.clone(),
            profile_home: Some(home),
        }
    }
}

impl SystemHost {
    /// Comando sem shell, ambiente por allowlist e PATH com diretórios do sistema primeiro.
    fn command(&self, exe: &TrustedExe, args: Vec<String>) -> Command {
        let mut cmd = Command::new(exe.path());
        cmd.args(args).env_clear();
        if let Ok(path) = env::join_paths(&self.runtime_dirs) {
            cmd.env("PATH", path);
        }
        match &self.profile_home {
            Some(home) => {
                // perfil isolado: nenhuma configuração de outra conta ou do serviço é herdada
                cmd.env("HOME", home)
                    .env("XDG_CONFIG_HOME", home.join(".config"))
                    .env("XDG_DATA_HOME", home.join(".local/share"))
                    .env("XDG_STATE_HOME", home.join(".local/state"))
                    .env("XDG_CACHE_HOME", home.join(".cache"));
            }
            None => {
                // Allowlist de ambiente: o suficiente para a CLI achar a própria configuração/login.
                for k in ["HOME", "USER", "LANG", "TMPDIR", "XDG_CONFIG_HOME"] {
                    if let Some(v) = env::var_os(k) {
                        cmd.env(k, v);
                    }
                }
            }
        }
        cmd.env("NO_COLOR", "1").env("CI", "1");
        cmd
    }
}

impl Default for SystemHost {
    fn default() -> Self {
        Self::new()
    }
}

impl Host for SystemHost {
    fn with_profile_home(&self, home: &Path) -> Option<Box<dyn Host>> {
        Some(Box::new(SystemHost::with_profile_home(self, home.to_path_buf())))
    }

    fn inspect(&self, name: &str) -> Option<Inspected> {
        // O primeiro candidato vence; se ele for reprovado, não pulamos para o próximo (isso
        // esconderia um binário plantado antes do legítimo).
        let found = self
            .search_dirs
            .iter()
            .map(|d| d.join(name))
            .find(|p| is_executable(p))?;
        Some(trust::inspect(&found, &self.runtime_dirs))
    }

    fn run(&self, exe: &TrustedExe, args: &[&str], limits: Limits) -> RunOutput {
        run_bounded(
            self.command(exe, args.iter().map(|a| a.to_string()).collect()),
            limits,
            None,
        )
    }

    fn run_stream(
        &self,
        exe: &TrustedExe,
        spec: &StreamSpec,
        cancel: Option<&AtomicBool>,
        on_data: &mut dyn FnMut(&[u8]),
    ) -> RunOutput {
        let mut cmd = self.command(exe, spec.args.clone());
        cmd.current_dir(&spec.cwd);
        run_streaming(cmd, spec.limits, cancel, Some(&spec.stdin), false, on_data)
    }
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(all(test, unix))]
mod tests {
    use std::time::Duration;

    use super::*;

    const LIM: Limits = Limits {
        timeout: Duration::from_secs(5),
        max_output: 1 << 20,
    };

    #[test]
    fn missing_binary_is_none() {
        assert!(SystemHost::new()
            .inspect("binario-que-nao-existe-xyz")
            .is_none());
    }

    #[test]
    fn does_not_leak_environment() {
        std::env::set_var("ONEAGENT_TEST_SECRET", "segredo");
        let h = SystemHost::new();
        let out = h.run(&TrustedExe::new(PathBuf::from("/usr/bin/env")), &[], LIM);
        assert!(out.ok());
        assert!(!out.stdout.contains("ONEAGENT_TEST_SECRET"));
        assert!(out.stdout.contains("NO_COLOR=1"));
    }
}

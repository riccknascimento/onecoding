//! Execução de processos com limites: grupo de processos próprio, leitura de saída sem threads
//! (não bloqueante), prazo total, limite de volume e cancelamento. Nada aqui usa shell.
//!
//! Por que sem threads de leitura: uma thread bloqueada em `read` só termina no EOF do pipe, e um
//! descendente que herda o stdout pode mantê-lo aberto para sempre. Lendo em modo não bloqueante
//! no próprio laço, o prazo vale sempre e não sobra thread nem descritor pendente.

use std::io::{self, Read};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Como a execução terminou.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Terminou sozinha; `None` = morta por sinal.
    Exited(Option<i32>),
    TimedOut,
    Cancelled,
    /// Passou do limite de saída; o grupo foi encerrado.
    OutputLimit,
    /// Não foi possível iniciar (ou a plataforma não é suportada).
    SpawnFailed,
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub timeout: Duration,
    pub max_output: usize,
}

#[derive(Debug, Clone)]
pub struct RunOutput {
    pub stdout: String,
    pub outcome: Outcome,
}

impl RunOutput {
    pub fn ok(&self) -> bool {
        self.outcome == Outcome::Exited(Some(0))
    }
    pub fn timed_out(&self) -> bool {
        self.outcome == Outcome::TimedOut
    }
}

/// Tempo extra, depois que o processo principal sai, para esvaziar o pipe antes de desistir.
const DRAIN_GRACE: Duration = Duration::from_millis(300);
const POLL: Duration = Duration::from_millis(10);

#[cfg(unix)]
mod sys {
    use std::io;
    use std::os::unix::io::AsRawFd;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, ChildStdout, Command};

    /// O filho vira líder de um grupo novo: todos os descendentes herdam o grupo.
    pub fn prepare(cmd: &mut Command) {
        cmd.process_group(0);
    }

    /// Mata o grupo inteiro (idempotente: grupo vazio dá ESRCH, ignorado).
    pub fn kill_group(child: &mut Child) {
        // SAFETY: `kill` é segura de chamar com qualquer pid/sinal; falhas são ignoradas.
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
        let _ = child.kill();
    }

    pub fn set_nonblocking(out: &ChildStdout) -> io::Result<()> {
        let fd = out.as_raw_fd();
        // SAFETY: fcntl em descritor válido, de propriedade de `out`.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }
}

/// Windows: usar Job Objects (grupo) e I/O sobreposto. Por ora falha de forma explícita.
#[cfg(not(unix))]
mod sys {
    use std::io;
    use std::process::{Child, ChildStdout, Command};

    pub fn prepare(_: &mut Command) {}
    pub fn kill_group(child: &mut Child) {
        let _ = child.kill();
    }
    pub fn set_nonblocking(_: &ChildStdout) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "plataforma ainda não suportada",
        ))
    }
}

/// Executa `cmd` (já com programa, argumentos e ambiente) sob `limits`. Nunca bloqueia além de
/// `timeout` + pequena folga; ao terminar, nenhum processo do grupo sobrevive.
pub fn run_bounded(mut cmd: Command, limits: Limits, cancel: Option<&AtomicBool>) -> RunOutput {
    let failed = |outcome| RunOutput {
        stdout: String::new(),
        outcome,
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    sys::prepare(&mut cmd);
    let Ok(mut child) = cmd.spawn() else {
        return failed(Outcome::SpawnFailed);
    };
    let mut stdout = child.stdout.take().expect("stdout piped");
    if sys::set_nonblocking(&stdout).is_err() {
        sys::kill_group(&mut child);
        let _ = child.wait();
        return failed(Outcome::SpawnFailed);
    }

    let start = Instant::now();
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut eof = false;
    let mut exit: Option<std::process::ExitStatus> = None;
    let mut exited_at: Option<Instant> = None;
    let mut forced: Option<Outcome> = None;

    loop {
        // 1) esvazia o que já chegou ao pipe
        while !eof {
            match stdout.read(&mut chunk) {
                Ok(0) => eof = true,
                Ok(n) => {
                    let room = limits.max_output.saturating_sub(buf.len());
                    buf.extend_from_slice(&chunk[..n.min(room)]);
                    if n > room {
                        forced = Some(Outcome::OutputLimit);
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => eof = true,
            }
        }
        if forced.is_some() {
            break;
        }
        // 2) cancelamento e prazo
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            forced = Some(Outcome::Cancelled);
            break;
        }
        if start.elapsed() >= limits.timeout {
            forced = Some(Outcome::TimedOut);
            break;
        }
        // 3) o processo principal saiu? então encerra descendentes que ficaram no grupo
        if exit.is_none() {
            if let Ok(Some(status)) = child.try_wait() {
                exit = Some(status);
                exited_at = Some(Instant::now());
                sys::kill_group(&mut child);
            }
        }
        if exit.is_some() && eof {
            break;
        }
        if exited_at.is_some_and(|t| t.elapsed() >= DRAIN_GRACE) {
            break; // algo fora do grupo ainda segura o pipe: para de coletar e fecha o descritor
        }
        std::thread::sleep(POLL);
    }

    sys::kill_group(&mut child);
    let status = match exit {
        Some(s) => s,
        None => match child.wait() {
            Ok(s) => s,
            Err(_) => return failed(Outcome::SpawnFailed),
        },
    };
    let outcome = forced.unwrap_or(Outcome::Exited(status.code()));
    RunOutput {
        stdout: String::from_utf8_lossy(&buf).into_owned(),
        outcome,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn sh(script: &str) -> Command {
        let mut c = Command::new("/bin/sh"); // fixture de teste; o código de produção nunca usa shell
        c.args(["-c", script]);
        c
    }

    fn lim(ms: u64, max: usize) -> Limits {
        Limits {
            timeout: Duration::from_millis(ms),
            max_output: max,
        }
    }

    fn alive(pid: i32) -> bool {
        // SAFETY: sinal 0 só verifica a existência do processo.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    fn assert_dies(pid: i32) {
        for _ in 0..100 {
            if !alive(pid) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("processo {pid} continua vivo");
    }

    #[test]
    fn normal_exit_captures_stdout() {
        let out = run_bounded(sh("echo oi"), lim(5000, 1024), None);
        assert_eq!(out.outcome, Outcome::Exited(Some(0)));
        assert_eq!(out.stdout.trim(), "oi");
        assert!(out.ok());
    }

    #[test]
    fn nonzero_exit_is_distinct_from_timeout() {
        let out = run_bounded(sh("exit 3"), lim(5000, 1024), None);
        assert_eq!(out.outcome, Outcome::Exited(Some(3)));
        assert!(!out.ok() && !out.timed_out());
    }

    /// Regressão do achado da auditoria: o pai sai, mas um descendente mantém o stdout aberto.
    #[test]
    fn descendant_holding_stdout_does_not_block_and_is_killed() {
        let t = Instant::now();
        let out = run_bounded(sh("sleep 30 & echo $!; exit 0"), lim(10_000, 1024), None);
        assert!(
            t.elapsed() < Duration::from_secs(3),
            "demorou {:?}",
            t.elapsed()
        );
        assert_eq!(out.outcome, Outcome::Exited(Some(0)));
        let pid: i32 = out.stdout.trim().parse().expect("pid do descendente");
        assert_dies(pid);
    }

    #[test]
    fn timeout_kills_whole_group_quickly() {
        let t = Instant::now();
        let out = run_bounded(sh("sleep 30 & echo $!; wait"), lim(300, 1024), None);
        assert!(t.elapsed() < Duration::from_secs(3));
        assert_eq!(out.outcome, Outcome::TimedOut);
        let pid: i32 = out.stdout.trim().parse().expect("pid do descendente");
        assert_dies(pid);
    }

    #[test]
    fn output_flood_is_capped_and_stopped() {
        let t = Instant::now();
        let out = run_bounded(sh("yes"), lim(10_000, 1000), None);
        assert_eq!(out.outcome, Outcome::OutputLimit);
        assert!(out.stdout.len() <= 1000);
        assert!(t.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn cancel_stops_the_group() {
        let flag = Arc::new(AtomicBool::new(false));
        let f2 = flag.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            f2.store(true, Ordering::Relaxed);
        });
        let t = Instant::now();
        let out = run_bounded(
            sh("sleep 30 & echo $!; wait"),
            lim(10_000, 1024),
            Some(&flag),
        );
        assert_eq!(out.outcome, Outcome::Cancelled);
        assert!(t.elapsed() < Duration::from_secs(3));
        assert_dies(out.stdout.trim().parse().unwrap());
    }

    #[test]
    fn spawn_failure_is_reported() {
        let out = run_bounded(Command::new("/nao/existe"), lim(1000, 1024), None);
        assert_eq!(out.outcome, Outcome::SpawnFailed);
    }
}

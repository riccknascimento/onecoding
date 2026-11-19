//! OneExecutor: executa a CLI aprovada (hoje o Codex) sobre um snapshot isolado do projeto.
//! Os arquivos chegam por ferramentas remotas do OneNode, pedidas ao OneHub Central. O executor não tem
//! credenciais de IA na configuração, não lê o disco do OneNode e não recebe shell.

pub mod agent;
pub mod central;
pub mod engines;
pub mod codex;
pub mod config;
pub mod job;

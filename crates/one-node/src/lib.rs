//! OneNode: agente de um servidor de cliente. Conexão de SAÍDA ao OneHub Central (polling autenticado),
//! heartbeat, lease de tarefas e um catálogo fechado de ferramentas de leitura confinadas aos projetos
//! autorizados. Não contém nem recebe credenciais pessoais de IA.

pub mod agent;
pub mod central;
pub mod config;
pub mod confine;
pub mod state;
pub mod tools;
pub mod catalog;
pub mod policy;

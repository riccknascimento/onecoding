//! OneNode provisioner: fingerprint sem credenciais, sessão SSH com fingerprint validada no handshake,
//! instalação do OneNode e API local autenticada usada somente pelo OneHub Central.

pub mod guard;
pub mod install;
pub mod scan;
pub mod service;
pub mod ssh;

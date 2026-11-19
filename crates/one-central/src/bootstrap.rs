//! Bootstrap do administrador inicial. Único caminho para criar o primeiro usuário: CLI local
//! (`one-central bootstrap-admin`), com a senha lida da entrada padrão. Não existe rota HTTP para isso.
//! Só funciona enquanto não houver nenhum usuário; um advisory lock impede corrida entre duas execuções.

use uuid::Uuid;

use crate::auth::hash_password;
use crate::db::DbPool;

#[derive(Debug, PartialEq, Eq)]
pub enum BootstrapError {
    AlreadyInitialized,
    InvalidInput(&'static str),
    Internal,
}

impl std::fmt::Display for BootstrapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BootstrapError::AlreadyInitialized => {
                write!(f, "já existe um usuário: bootstrap recusado")
            }
            BootstrapError::InvalidInput(m) => write!(f, "{m}"),
            BootstrapError::Internal => write!(f, "falha interna no bootstrap"),
        }
    }
}

pub fn validate_admin_input(email: &str, password: &str) -> Result<String, BootstrapError> {
    let email = email.trim().to_lowercase();
    if email.len() < 3 || email.len() > 254 || !email.contains('@') {
        return Err(BootstrapError::InvalidInput("e-mail inválido"));
    }
    if password.chars().count() < 12 {
        return Err(BootstrapError::InvalidInput(
            "a senha precisa ter pelo menos 12 caracteres",
        ));
    }
    Ok(email)
}

pub async fn bootstrap_admin(
    pool: &DbPool,
    email: &str,
    password: &str,
) -> Result<Uuid, BootstrapError> {
    let email = validate_admin_input(email, password)?;
    let hash = hash_password(password).map_err(|_| BootstrapError::Internal)?;
    let mut client = pool.get().await.map_err(|_| BootstrapError::Internal)?;
    let tx = client
        .transaction()
        .await
        .map_err(|_| BootstrapError::Internal)?;
    tx.execute("SELECT pg_advisory_xact_lock(4242)", &[])
        .await
        .map_err(|_| BootstrapError::Internal)?;
    let exists: bool = tx
        .query_one("SELECT EXISTS (SELECT 1 FROM users)", &[])
        .await
        .map_err(|_| BootstrapError::Internal)?
        .get(0);
    if exists {
        return Err(BootstrapError::AlreadyInitialized);
    }
    let id: Uuid = tx
        .query_one(
            "INSERT INTO users(email, password_hash, role) VALUES ($1, $2, 'admin') RETURNING id",
            &[&email, &hash],
        )
        .await
        .map_err(|_| BootstrapError::Internal)?
        .get(0);
    tx.execute(
        "INSERT INTO audit_log(actor, action, subject, outcome) VALUES ('cli', 'bootstrap_admin', $1, 'created')",
        &[&id.to_string()],
    )
    .await
    .map_err(|_| BootstrapError::Internal)?;
    tx.commit().await.map_err(|_| BootstrapError::Internal)?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_input_requires_email_and_strong_password_without_defaults() {
        assert!(validate_admin_input("sem-arroba", "senha-forte-123").is_err());
        assert!(validate_admin_input("a@b.co", "curta").is_err());
        assert_eq!(
            validate_admin_input(" Admin@Exemplo.COM ", "senha-forte-123").unwrap(),
            "admin@exemplo.com"
        );
    }
}

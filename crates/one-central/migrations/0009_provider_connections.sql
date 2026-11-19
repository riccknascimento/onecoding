-- One Provider AI: conexões por API Key de provedores de IA.
-- A chave é cifrada no servidor (AES-256-GCM) com uma chave-mestra fora do banco. O banco guarda só o
-- texto cifrado, o nonce, a versão da chave e uma dica (últimos 4 caracteres). Nenhum valor completo volta ao navegador.

CREATE TABLE provider_connections (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name            text NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    provider        text NOT NULL CHECK (provider IN ('openrouter', 'openai', 'gemini', 'anthropic')),
    key_ciphertext  bytea NOT NULL,
    key_nonce       bytea NOT NULL CHECK (octet_length(key_nonce) = 12),
    key_version     integer NOT NULL DEFAULT 1 CHECK (key_version >= 1),
    key_hint        text NOT NULL CHECK (length(key_hint) BETWEEN 1 AND 16),
    default_model   text CHECK (default_model IS NULL OR length(default_model) BETWEEN 1 AND 120),
    status          text NOT NULL DEFAULT 'unverified' CHECK (status IN ('unverified', 'connected', 'error')),
    last_checked_at timestamptz,
    created_by      text NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now()
);

-- One Provider AI: contas de CLI (uma linha por conta autenticada) e sessões de login/status/logout.
-- Credenciais NUNCA entram aqui: elas ficam no perfil isolado de cada conta, no OneExecutor, sob permissão 0700.
-- A tabela guarda só o estado, a URL oficial e o código de dispositivo de um login em andamento (apagados ao final),
-- e o código colado pelo usuário (apagado na primeira entrega ao executor).

CREATE TABLE cli_connections (
    id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name              text NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    engine            text NOT NULL CHECK (engine IN ('codex', 'claude', 'cursor')),
    executor_id       uuid NOT NULL REFERENCES executors(id) ON DELETE CASCADE,
    usage_authorized  boolean NOT NULL DEFAULT false,
    auth_state        text NOT NULL DEFAULT 'unknown' CHECK (auth_state IN ('unknown', 'authenticated', 'unauthenticated')),
    auth_checked_at   timestamptz,
    created_by        text NOT NULL,
    created_at        timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now(),
    UNIQUE (executor_id, name)
);

CREATE TABLE cli_sessions (
    id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    connection_id     uuid NOT NULL REFERENCES cli_connections(id) ON DELETE CASCADE,
    action            text NOT NULL CHECK (action IN ('login', 'status', 'logout')),
    state             text NOT NULL DEFAULT 'pending'
                      CHECK (state IN ('pending', 'running', 'awaiting_code', 'succeeded', 'failed', 'timed_out', 'cancelled')),
    cancel_requested  boolean NOT NULL DEFAULT false,
    url               text CHECK (url IS NULL OR length(url) <= 2048),
    device_code       text CHECK (device_code IS NULL OR length(device_code) <= 16),
    pasted_code       text CHECK (pasted_code IS NULL OR length(pasted_code) <= 256),
    result            text CHECK (result IS NULL OR length(result) <= 200),
    created_by        text NOT NULL,
    created_at        timestamptz NOT NULL DEFAULT now(),
    started_at        timestamptz,
    finished_at       timestamptz,
    expires_at        timestamptz NOT NULL
);

-- no máximo uma sessão ativa por conta: um login não atropela outro
CREATE UNIQUE INDEX cli_sessions_one_active
    ON cli_sessions (connection_id)
    WHERE state IN ('pending', 'running', 'awaiting_code');

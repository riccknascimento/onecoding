-- OneHub Central, esquema inicial. PostgreSQL é a fonte principal de verdade.
-- Nunca armazenar senhas em texto puro, tokens de sessão, tokens de OneNode ou credenciais de CLIs/ChatGPT:
-- só hashes (SHA-256 para tokens de alta entropia, Argon2id para senhas).

CREATE TABLE users (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    email         text NOT NULL UNIQUE CHECK (length(email) BETWEEN 3 AND 254),
    password_hash text NOT NULL,
    role          text NOT NULL CHECK (role IN ('admin', 'member')),
    disabled      boolean NOT NULL DEFAULT false,
    created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE sessions (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash  bytea NOT NULL UNIQUE,
    created_at  timestamptz NOT NULL DEFAULT now(),
    expires_at  timestamptz NOT NULL,
    revoked_at  timestamptz
);
CREATE INDEX sessions_user ON sessions(user_id);

-- Identidade técnica de cada OneNode: sem credenciais pessoais de IA.
CREATE TABLE onenodes (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name            text NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    pairing_hash    bytea,
    pairing_expires timestamptz,
    token_hash      bytea UNIQUE,
    paired_at       timestamptz,
    last_seen_at    timestamptz,
    revoked_at      timestamptz,
    created_at      timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE projects (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name       text NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    created_by uuid NOT NULL REFERENCES users(id),
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Autorização por par OneNode × projeto. Sem concessão ativa, nenhuma tarefa é aceita.
CREATE TABLE node_project_grants (
    node_id    uuid NOT NULL REFERENCES onenodes(id),
    project_id uuid NOT NULL REFERENCES projects(id),
    granted_by uuid NOT NULL REFERENCES users(id),
    granted_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz,
    PRIMARY KEY (node_id, project_id)
);

CREATE TABLE conversations (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id   uuid NOT NULL REFERENCES users(id),
    title      text NOT NULL DEFAULT 'Nova conversa' CHECK (length(title) <= 200),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX conversations_owner ON conversations(owner_id, updated_at DESC);

CREATE TABLE messages (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    conversation_id uuid NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    role            text NOT NULL CHECK (role IN ('user', 'assistant', 'system')),
    body            text NOT NULL CHECK (length(body) <= 16384),
    created_at      timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX messages_conversation ON messages(conversation_id, created_at);

-- Tarefas: uma única execução por tarefa; a idempotência evita duplicidade por reenvio.
CREATE TABLE tasks (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    conversation_id uuid REFERENCES conversations(id) ON DELETE SET NULL,
    node_id         uuid NOT NULL REFERENCES onenodes(id),
    project_id      uuid NOT NULL REFERENCES projects(id),
    created_by      uuid NOT NULL REFERENCES users(id),
    prompt          text NOT NULL CHECK (length(prompt) BETWEEN 1 AND 16384),
    mode            text NOT NULL DEFAULT 'read_only' CHECK (mode = 'read_only'),
    status          text NOT NULL DEFAULT 'queued'
                    CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'cancelled')),
    lease_expires   timestamptz,
    -- token do lease atual: só o nó que recebeu o lease pode registrar eventos ou finalizar
    lease_token     uuid,
    result          jsonb,
    idempotency_key text NOT NULL CHECK (length(idempotency_key) BETWEEN 8 AND 128),
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    UNIQUE (created_by, idempotency_key)
);
CREATE INDEX tasks_node_status ON tasks(node_id, status, created_at);

CREATE TABLE task_events (
    task_id    uuid NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    seq        integer NOT NULL CHECK (seq >= 0),
    kind       text NOT NULL CHECK (length(kind) BETWEEN 1 AND 64),
    payload    jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (task_id, seq)
);

CREATE TABLE audit_log (
    id      bigserial PRIMARY KEY,
    ts      timestamptz NOT NULL DEFAULT now(),
    actor   text NOT NULL,
    action  text NOT NULL,
    subject text NOT NULL,
    outcome text NOT NULL
);

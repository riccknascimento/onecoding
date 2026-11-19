-- OneExecutor confiável: identidade própria, jobs de análise em contexto isolado e eventos.
-- O executor pede ferramentas remotas pelo Central (tarefas ligadas ao job). Nunca recebe credenciais de IA.
CREATE TABLE executors (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name            text NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    engine          text NOT NULL DEFAULT 'codex' CHECK (engine IN ('codex')),
    pairing_hash    bytea,
    pairing_expires timestamptz,
    token_hash      bytea UNIQUE,
    paired_at       timestamptz,
    last_seen_at    timestamptz,
    revoked_at      timestamptz,
    created_at      timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE jobs (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    conversation_id uuid NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    node_id         uuid NOT NULL REFERENCES onenodes(id),
    project_id      uuid NOT NULL REFERENCES projects(id),
    created_by      uuid NOT NULL REFERENCES users(id),
    kind            text NOT NULL DEFAULT 'analysis' CHECK (kind IN ('analysis')),
    question        text NOT NULL CHECK (length(question) BETWEEN 1 AND 16384),
    status          text NOT NULL DEFAULT 'queued'
                    CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'cancelled')),
    executor_id     uuid REFERENCES executors(id),
    lease_token     uuid,
    lease_expires   timestamptz,
    answer          text,
    evidence        jsonb NOT NULL DEFAULT '[]'::jsonb,
    error           text,
    idempotency_key text NOT NULL CHECK (length(idempotency_key) BETWEEN 8 AND 128),
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    UNIQUE (created_by, idempotency_key)
);
CREATE INDEX jobs_status ON jobs(status, created_at);

CREATE TABLE job_events (
    job_id     uuid NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    seq        integer NOT NULL CHECK (seq >= 0),
    kind       text NOT NULL CHECK (length(kind) BETWEEN 1 AND 64),
    payload    jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (job_id, seq)
);

-- Ferramentas pedidas pelo executor ficam ligadas ao job (e ao mesmo OneNode/projeto).
ALTER TABLE tasks ADD COLUMN job_id uuid REFERENCES jobs(id) ON DELETE CASCADE;
ALTER TABLE messages ADD COLUMN job_id uuid REFERENCES jobs(id) ON DELETE SET NULL;
ALTER TABLE messages DROP CONSTRAINT messages_state_check;
ALTER TABLE messages ADD CONSTRAINT messages_state_check
    CHECK (state IN ('completed', 'unavailable', 'rejected', 'task', 'failed'));

-- Chat do OneHub: mensagens com estado e tarefa associada; contexto (OneNode e projeto) da conversa.
ALTER TABLE messages ADD COLUMN state text NOT NULL DEFAULT 'completed'
    CHECK (state IN ('completed', 'unavailable', 'rejected', 'task'));
ALTER TABLE messages ADD COLUMN task_id uuid REFERENCES tasks(id) ON DELETE SET NULL;
ALTER TABLE conversations ADD COLUMN node_id uuid REFERENCES onenodes(id) ON DELETE SET NULL;
ALTER TABLE conversations ADD COLUMN project_id uuid REFERENCES projects(id) ON DELETE SET NULL;

-- Proteção contra força bruta: falhas de login por e-mail (independe de existir a conta).
CREATE TABLE login_failures (
    id        bigserial PRIMARY KEY,
    email     text NOT NULL CHECK (length(email) <= 254),
    failed_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX login_failures_email_time ON login_failures(email, failed_at);

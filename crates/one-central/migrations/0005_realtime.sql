-- Tempo real: log de eventos por conversa com sequência sem lacunas visíveis, e cancelamento de jobs
-- que só se conclui quando o OneExecutor confirma. Não altera tabelas das migrations anteriores.
-- A sequência é alocada com UPDATE na linha da conversa (lock), dentro da mesma transação da mudança:
-- quem grava depois só obtém o próximo número após o commit anterior, então a ordem coincide com a de commit.
ALTER TABLE conversations ADD COLUMN last_event_seq bigint NOT NULL DEFAULT 0 CHECK (last_event_seq >= 0);

CREATE TABLE conversation_events (
    conversation_id uuid NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    seq             bigint NOT NULL CHECK (seq > 0),
    kind            text NOT NULL CHECK (length(kind) BETWEEN 1 AND 64),
    payload         jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (conversation_id, seq)
);

ALTER TABLE jobs ADD COLUMN cancel_requested_at timestamptz;

ALTER TABLE messages DROP CONSTRAINT messages_state_check;
ALTER TABLE messages ADD CONSTRAINT messages_state_check
    CHECK (state IN ('completed', 'unavailable', 'rejected', 'task', 'failed', 'cancelled'));

-- Persistent provider selection and prompts. Enabling inference remains gated by runtime support.
CREATE TABLE orchestrator_settings (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    enabled boolean NOT NULL DEFAULT false,
    provider_connection_id uuid REFERENCES provider_connections(id) ON DELETE SET NULL,
    model text,
    system_prompt text NOT NULL DEFAULT '' CHECK (length(system_prompt) <= 16384),
    instructions text NOT NULL DEFAULT '' CHECK (length(instructions) <= 16384),
    updated_by text NOT NULL DEFAULT '',
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE executor_settings (
    executor_id uuid PRIMARY KEY REFERENCES executors(id) ON DELETE CASCADE,
    cli_connection_id uuid REFERENCES cli_connections(id) ON DELETE SET NULL,
    model text,
    instructions text NOT NULL DEFAULT '' CHECK (length(instructions) <= 16384),
    updated_by text NOT NULL DEFAULT '',
    updated_at timestamptz NOT NULL DEFAULT now()
);

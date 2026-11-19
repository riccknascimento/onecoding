-- Tarefas tipadas: o OneNode executa só ferramentas do catálogo fechado, nunca texto livre como comando.
ALTER TABLE tasks ADD COLUMN tool text;
ALTER TABLE tasks ADD COLUMN args jsonb NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE tasks ADD CONSTRAINT tasks_tool_allowlist CHECK (tool IS NULL OR tool IN ('files_list', 'files_read'));

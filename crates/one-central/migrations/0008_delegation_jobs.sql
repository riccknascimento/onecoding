-- Delegação de OneAgents (aditiva). Jobs de análise continuam exigindo nó e projeto; jobs de delegação
-- não têm projeto. O relatório de motores do OneExecutor fica no próprio executor (somente estado, sem segredo).

ALTER TABLE jobs ALTER COLUMN node_id DROP NOT NULL;
ALTER TABLE jobs ALTER COLUMN project_id DROP NOT NULL;
ALTER TABLE jobs DROP CONSTRAINT IF EXISTS jobs_kind_check;
ALTER TABLE jobs ADD CONSTRAINT jobs_kind_check CHECK (kind IN ('analysis', 'delegation'));
ALTER TABLE jobs ADD COLUMN agent_id text CHECK (agent_id IS NULL OR length(agent_id) <= 63);
ALTER TABLE jobs ADD COLUMN agent_version text CHECK (agent_version IS NULL OR length(agent_version) <= 16);
ALTER TABLE jobs ADD COLUMN delegation jsonb;
ALTER TABLE jobs ADD CONSTRAINT jobs_analysis_has_context CHECK (
    kind = 'delegation' OR (node_id IS NOT NULL AND project_id IS NOT NULL)
);
ALTER TABLE executors ADD COLUMN engines jsonb NOT NULL DEFAULT '[]'::jsonb;
ALTER TABLE executors ADD COLUMN engines_reported_at timestamptz;

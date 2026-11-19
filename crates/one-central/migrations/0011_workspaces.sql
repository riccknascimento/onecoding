-- Workspaces independentes de projetos. Fechar uma aba no navegador não apaga estes registros.
CREATE TABLE workspaces (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id   uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name       text NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX workspaces_owner_updated ON workspaces(owner_id, updated_at DESC);

-- Relação extensível: esta etapa seleciona um contexto por vez, mas não atrela o Workspace
-- a um único projeto para sempre. Um OneNode pode hospedar vários projetos/clients; o vínculo
-- não concede acesso ao filesystem, que depende da concessão ativa OneNode × projeto.
-- Metadados Git e autorização de OneExecutor serão associados por projeto em etapa própria;
-- eles não devem habilitar operações Git nem deploy implicitamente.
CREATE TABLE workspace_contexts (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id uuid NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    node_id      uuid REFERENCES onenodes(id),
    project_id   uuid REFERENCES projects(id),
    is_active    boolean NOT NULL DEFAULT false,
    created_at   timestamptz NOT NULL DEFAULT now(),
    CHECK (project_id IS NULL OR node_id IS NOT NULL)
);
CREATE UNIQUE INDEX workspace_context_one_active
    ON workspace_contexts(workspace_id) WHERE is_active;
CREATE UNIQUE INDEX workspace_context_unique
    ON workspace_contexts(
        workspace_id,
        (COALESCE(node_id, '00000000-0000-0000-0000-000000000000'::uuid)),
        (COALESCE(project_id, '00000000-0000-0000-0000-000000000000'::uuid))
    );

ALTER TABLE conversations ADD COLUMN workspace_id uuid;

-- Conversas existentes ficam num Workspace de compatibilidade do dono; mensagens e execuções não mudam.
WITH owners AS (
    SELECT DISTINCT owner_id FROM conversations
), created AS (
    INSERT INTO workspaces(owner_id, name)
    SELECT owner_id, 'Workspace existente' FROM owners
    RETURNING id, owner_id
)
UPDATE conversations c
SET workspace_id = created.id
FROM created
WHERE c.owner_id = created.owner_id;

-- O contexto padrão vem da conversa atualizada mais recentemente; os contextos históricos das conversas ficam intactos.
INSERT INTO workspace_contexts(workspace_id, node_id, project_id, is_active)
SELECT w.id, latest.node_id, latest.project_id, true
FROM (
    SELECT DISTINCT ON (owner_id) owner_id, node_id, project_id
    FROM conversations
    ORDER BY owner_id, updated_at DESC, id
) latest
JOIN workspaces w ON w.owner_id = latest.owner_id
WHERE latest.node_id IS NOT NULL OR latest.project_id IS NOT NULL;

-- A chave referenciada deve ser única; esta chave também impede ligar conversa ao Workspace de outro dono.
ALTER TABLE workspaces ADD CONSTRAINT workspaces_id_owner_unique UNIQUE (id, owner_id);
ALTER TABLE conversations
    ALTER COLUMN workspace_id SET NOT NULL,
    ADD CONSTRAINT conversations_workspace_owner
        FOREIGN KEY (workspace_id, owner_id) REFERENCES workspaces(id, owner_id);
CREATE INDEX conversations_workspace_updated ON conversations(workspace_id, updated_at DESC);

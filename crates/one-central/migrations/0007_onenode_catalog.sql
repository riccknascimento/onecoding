-- OneNode: catálogo de projetos mantido pelo OneHub, versão aplicada pelo nó e estado de suspensão.
-- A raiz de cada projeto é pedida pelo administrador por concessão; o nó só a usa depois de validá-la
-- contra a política local (/etc/onenode/policy.json). Nenhum segredo entra nestas colunas.

ALTER TABLE node_project_grants
    ADD COLUMN root_path text CHECK (root_path IS NULL OR length(root_path) BETWEEN 1 AND 512);

ALTER TABLE onenodes
    -- incrementado a cada mudança de concessão ou suspensão; o nó aplica quando a versão muda
    ADD COLUMN catalog_version bigint NOT NULL DEFAULT 1,
    ADD COLUMN applied_version bigint,
    ADD COLUMN applied_at timestamptz,
    -- relatório da última aplicação: quais projetos foram aceitos e quais recusados (com motivo curto)
    ADD COLUMN applied_report jsonb,
    ADD COLUMN suspended_at timestamptz,
    -- rótulo informativo do tipo de rede do servidor (padrão: acesso direto). Não guarda endereço nem credencial.
    ADD COLUMN access_type text NOT NULL DEFAULT 'direct' CHECK (access_type IN ('direct', 'tailscale'));


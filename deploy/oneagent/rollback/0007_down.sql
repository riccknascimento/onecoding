-- Reversão MANUAL da migration 0007 (não é aplicada pelo loader de migrations).
-- Usar somente se for necessário remover o catálogo. Antes: backup verificado do banco e da aplicação anterior.
-- A aplicação anterior ignora estas colunas; o rollback de imagem NÃO exige esta reversão.
ALTER TABLE onenodes
    DROP COLUMN IF EXISTS access_type,
    DROP COLUMN IF EXISTS suspended_at,
    DROP COLUMN IF EXISTS applied_report,
    DROP COLUMN IF EXISTS applied_at,
    DROP COLUMN IF EXISTS applied_version,
    DROP COLUMN IF EXISTS catalog_version;
ALTER TABLE node_project_grants DROP COLUMN IF EXISTS root_path;

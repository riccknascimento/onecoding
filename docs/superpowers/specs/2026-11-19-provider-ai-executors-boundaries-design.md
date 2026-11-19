# Provider AI, Executors e Orquestrador — desenho

**Intenção:** deixar claro onde cada tipo de configuração vive, sem apagar as conexões de IA existentes nem sugerir capacidades que o OneHub ainda não implementa. Provider AI é o catálogo para habilitar provedores suportados por CLI e API; o catálogo poderá crescer conforme novas integrações reais forem adicionadas.

**Decisão:** Provider AI permanece uma área própria para cadastrar conexões de CLI e API, autenticar contas CLI pelo fluxo oficial em um OneExecutor selecionado e armazenar chaves API cifradas no OneHub. Executors gerencia os workers pareados e seu estado. O Orquestrador será o lugar para motor, modelo, prompts e políticas quando houver suporte real no backend.

**Abordagens consideradas:** (1) mover conexões para Executors, o que esconde a finalidade de catálogo/conexões definida pelo produto; (2) manter Provider AI sem explicação, perpetuando a confusão; (3) manter as áreas separadas e explicitar os limites com descrições e links contextuais. Escolhida a terceira porque separa conexões, capacidade de execução e coordenação sem duplicar formulários.

**Escopo desta entrega:** ajustar descrições, orientação e navegação entre Settings, Provider AI, Executors e Orquestrador. Preservar endpoints, autenticação, permissões e estados atuais. Manter compatível a URL `/configuracoes/provider-ai`.

**Fora do escopo:** implementar instalador remoto de CLIs, adicionar provedores ou modelos sem suporte no backend, testar chaves API, ou criar configuração funcional de motor/prompts para o Orquestrador. A interface deve chamar essas funções indisponíveis pelo nome e não simulá-las.

**Restrições:** API key salva continua identificada como configurada, não conectada, até existir teste real; estado CLI continua vindo da autenticação confirmada pelo OneHub. Somente administradores gerenciam as conexões.

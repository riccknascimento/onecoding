# OneCoding Workspaces e abas — desenho

## Intenção

Transformar o OneCoding Web em um espaço de trabalho com abas: Settings é a aba global fixa e cada aba adicional representa um Workspace persistido no OneHub Central. O usuário pode alternar entre projetos sem misturar conversas nem interromper execuções.

## Objetivos

- Criar, abrir, renomear e fechar abas de Workspaces sem apagar dados.
- Persistir Workspaces, contextos e conversas no PostgreSQL existente, sempre isolados por `owner_id`.
- Fazer cada conversa nova herdar o contexto técnico atual do Workspace sem herdar mensagens anteriores.
- Preservar tarefas por meio da conversa e deixar execuções continuarem enquanto outra aba está ativa.
- Manter Settings global, navegação por módulos, rotas e restrições administrativas existentes.
- Reproduzir o shell visual da referência: barra de abas superior, sidebar à esquerda e painel Settings com cards.
- Manter recursos sem backend claramente indisponíveis; não inserir Workspaces de exemplo.

## Fora de escopo

- Workspaces com execução coordenada de vários projetos ao mesmo tempo.
- Cadastro de repositórios Git, operações Git ou configuração de deploy.
- Alterar VPS/OneNodes de clientes ou instalar ferramentas neles.
- Implementação de VibeAgent, novos motores, módulos de Agent/Skill/Plugins/MCP/Context.
- Excluir Workspaces ou conversas ao fechar uma aba.
- Alterar o backend de produção, publicar ou fazer deploy.

## Abordagens consideradas

1. Guardar Workspaces e conversas no `localStorage`: simples, mas perde os dados entre dispositivos e não aplica autorização no Central; rejeitada.
2. Tratar cada projeto como Workspace: reaproveita projetos, mas acopla identidade e contexto do Workspace a um projeto e dificulta vários projetos no futuro; rejeitada.
3. Criar Workspaces por usuário no PostgreSQL e uma relação própria de contextos Workspace ↔ OneNode/projeto; escolhida por preservar isolamento, dados existentes e expansão futura sem implementar multiprojeto agora.

## Desenho

- `workspaces`: identidade, proprietário, nome e timestamps.
- `workspace_contexts`: relação extensível com contextos OneNode/projeto; apenas um contexto ativo por Workspace na interface atual. Trocar contexto atualiza a seleção, sem apagar conversas.
- `conversations.workspace_id`: associa cada conversa ao Workspace; mensagens, eventos, jobs e tarefas continuam ligados à conversa pelas relações existentes.
- Migration aditiva: associa conversas existentes a um Workspace de compatibilidade por proprietário e inicializa o contexto do Workspace a partir da conversa mais recente. Nenhuma mensagem ou execução é removida.
- API autenticada: listar/criar/renomear Workspaces, selecionar contexto de Workspace, filtrar conversas por Workspace e criar conversa dentro do Workspace. Requisições antigas sem `workspace_id` continuam sendo aceitas pelo Workspace de compatibilidade.
- Interface: Settings fixo; lista de abas de Workspace, botão `+` com opções para abrir ou criar, fechar só a aba; Settings lista Workspaces disponíveis. Em Workspace, a sidebar mostra nome, Nova conversa e histórico desse Workspace.
- Persistência local é limitada à lista de IDs abertos e ao ID da aba ativa. Os IDs são reconciliados com os Workspaces que o servidor retorna após login.
- Contexto de Workspace é retornado na listagem de conversas e copiado para uma nova conversa. Mensagens e contexto da conversa antiga permanecem intactos.
- Rotas atuais de `/configuracoes/...` continuam válidas; query `?workspace=<id>` representa a aba ativa e habilita voltar/avançar entre abas.
- Configurações globais mantêm o papel de administrador; Workspaces são pessoais e podem ser usados por qualquer usuário autenticado.

## Limite de confiança e evolução de projetos/Git

- Um OneNode representa uma VPS/servidor de cliente e pode hospedar vários projetos independentes. Nunca inferir que o servidor inteiro equivale a um projeto ou que o usuário tem acesso a todos os diretórios nele.
- Workspace continua sendo uma entidade separada de Projeto. A evolução deve permitir vários projetos por Workspace, com escopo de cliente/organização explícito para impedir mistura de diretórios, credenciais, histórico Git e permissões entre clientes.
- A associação planejada por projeto contém OneNode responsável, caminho autorizado, repositório remoto Git, branch de trabalho e OneExecutor autorizado. O caminho autorizado continua sendo concedido pelo vínculo de permissão OneNode × projeto (hoje `node_project_grants.root_path`); a configuração do repositório não concede acesso ao diretório.
- GitHub é o remoto; clone/fetch/commit/push usam a cópia Git local no ambiente autorizado do cliente. Não instalar GitHub nas VPS. A existência de um OneNode não implica permissão para executar Git: capacidade e autorização Git são separadas das permissões atuais e precisam de implementação e validação próprias.
- Operações Git e deploy são capacidades distintas. Credenciais/repositórios e operações Git precisam respeitar o escopo e as permissões do projeto; deploy permanece uma ação separada, sujeita à autorização explícita.
- Esse requisito orienta o modelo futuro. A entrega atual de Workspaces não implementa cadastro de repositório, branch, autorização de executor para Git, operações Git nem deploy, e não altera máquinas de clientes.
## Direção visual

Seguir a referência enviada: cabeçalho grafite com marca, Settings e abas delineadas; área lateral escura e compacta; conteúdo principal com cartões alinhados; detalhes e estados usam os tokens já existentes. Responsivo: abas rolam horizontalmente e a sidebar vira gaveta no celular.

## Critérios de aceite

- Workspace criado aparece no backend e pode ser aberto sem duplicar a aba.
- Fechar e reabrir uma aba restaura conversas; alternar de aba não cancela execuções.
- Conversas filtram por Workspace; conversa nova inicia vazia e recebe contexto do Workspace.
- Usuário não lista, abre, altera ou cria conversa em Workspace de outra pessoa.
- Migração mantém conversas históricas e liga-as ao Workspace de compatibilidade.
- Settings e Provider AI continuam acessíveis conforme autorização; páginas sem backend exibem indisponibilidade.
- Typecheck, testes frontend, testes Rust compiláveis e build web passam; navegador real é validado se disponível.

# Integração Provider AI, Orquestrador e Executors — desenho

**Intenção:** fazer Provider AI ser a fonte única de conexões e credenciais, Orquestrador guardar comportamento/seleção de IA e Executors associar um worker a uma conexão CLI compatível, sem duplicar segredos nem conceder acesso implicitamente.

**Estado encontrado:** Provider AI já cadastra contas CLI isoladas por `connection_id` e guarda API keys cifradas, mas ainda não testa nem lista modelos via API. Heartbeat de OneExecutor informa instalação/autenticação/aprovação/versão, hoje só para Codex. O runtime de análise executa somente Codex em snapshot somente leitura. OrchestratorProvider é `UnavailableProvider`; não há configuração persistida.

**Decisão:** estender os contratos existentes de forma aditiva. Provider AI testa e lista modelos por endpoints do OneHub sem devolver a chave; autenticação CLI só pode iniciar quando executor reporta motor instalado e aprovado. Uma configuração global do Orquestrador referencia uma conexão API e modelo real, salva prompts e permanece desabilitada enquanto o runtime não estiver disponível. Uma configuração por OneExecutor referencia uma conta CLI autenticada no mesmo executor; nesta entrega, a execução continua limitada ao Codex, snapshot somente leitura, concessões OneNode/projeto existentes e aprovação atual do binário. A conta selecionada é enviada ao OneExecutor pelo job e seu perfil isolado, sem fallback silencioso.

**Abordagens consideradas:** (1) manter apenas o frontend e documentar as capacidades ausentes — não atende persistência e validação reais; (2) habilitar múltiplos motores, shell/escrita e VibeAgent ao mesmo tempo — não há runtime seguro existente e aumentaria o risco; (3) conectar a persistência e a verificação reais, integrar o perfil Codex já isolado ao job e manter motores/runtime não suportados claramente bloqueados. Escolhida a terceira porque entrega fluxo real sem ultrapassar as permissões atuais.

**Não metas:** instalador remoto de CLIs, execução Claude/Cursor/API por Executor, escrita de código, Git/Worktrees, execução VibeAgent, novos acessos a VPS de clientes, credenciais frontend ou rotação de contas.

**Persistência:** migration aditiva `0012` somente para configuração efetiva do Orquestrador e dos OneExecutors. Não alterar OneNode grants, paths, banco de produção, nem reutilizar uma API key como CLI.

**Operação:** após testes completos em banco isolado, publicar o Central e frontend autorizados pelo usuário; atualizar o binário OneExecutor somente se o protocolo da conta selecionada exigir e sem tocar VPS de clientes. Migration aditiva com backup e rollback de serviço.

import { inTauri } from "../api";
import { Badge, PageHeader } from "./common";

export function SettingsPage() {
  return (
    <section>
      <PageHeader title="Configurações" subtitle="Orquestrador, segurança e armazenamento." />
      <article className="card">
        <h2>Orquestrador</h2>
        <p>
          Provedor: <Badge tone="muted">nenhum (desacoplado)</Badge>
        </p>
        <p className="muted small">
          Nenhuma chamada paga é feita nesta fase: não há orçamento de API aprovado nem autenticação validada. O editor
          de System Prompt e as políticas por Gateway chegam na Fase 6.
        </p>
      </article>
      <article className="card">
        <h2>Segurança</h2>
        <p className="muted small">
          Somente leitura: o app apenas consulta versão, login e modelos das CLIs. Execução de tarefas, SSH e
          instalação de Gateways não existem nesta versão.
        </p>
      </article>
      <article className="card">
        <h2>Ambiente</h2>
        <p>
          Execução: <Badge tone={inTauri() ? "ok" : "muted"}>{inTauri() ? "App nativo" : "Navegador / testes"}</Badge>
        </p>
      </article>
    </section>
  );
}

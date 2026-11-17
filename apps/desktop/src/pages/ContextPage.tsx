import { useEffect, useState } from "react";
import { storeStatus } from "../api";
import type { StoreStatus } from "../contracts";
import { Badge, PageHeader } from "./common";

export function ContextPage() {
  const [status, setStatus] = useState<StoreStatus | null>(null);
  useEffect(() => {
    let alive = true;
    storeStatus().then((s) => alive && setStatus(s)).catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  return (
    <section>
      <PageHeader title="OneContext" subtitle="Conhecimento persistente por projeto, com origem e nível de validação." />
      <article className="card">
        <h2>Armazenamento local</h2>
        {status ? (
          <p>
            <Badge tone="ok">SQLite v{status.schema_version}</Badge>{" "}
            <Badge tone={status.fts5 ? "ok" : "bad"}>{status.fts5 ? "Busca FTS5 ativa" : "FTS5 indisponível"}</Badge>{" "}
            <Badge tone={status.integrity_ok ? "ok" : "bad"}>{status.integrity_ok ? "Integridade ok" : "Integridade com falha"}</Badge>
          </p>
        ) : (
          <p className="muted">Status disponível apenas no app.</p>
        )}
      </article>
      <article className="card">
        <h2>Contexto dos projetos</h2>
        <p className="muted">Nenhum projeto indexado. A ingestão e a recuperação chegam na Fase 5.</p>
      </article>
    </section>
  );
}

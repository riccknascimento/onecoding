import type { ReactNode } from "react";

export function PageHeader({ title, subtitle }: { title: string; subtitle?: string }) {
  return (
    <header className="page-header">
      <h1>{title}</h1>
      {subtitle && <p>{subtitle}</p>}
    </header>
  );
}

/** Selo exibido sempre que a tela mostra dados de demonstração. */
export function DemoBadge({ children = "DEMO" }: { children?: ReactNode }) {
  return (
    <span className="badge demo" title="Dados de demonstração, sem integração real">
      {children}
    </span>
  );
}

export function DemoNotice() {
  return (
    <div className="notice" role="note">
      <DemoBadge /> Dados de demonstração. Ainda não há integração real nesta tela.
    </div>
  );
}

export function Badge({ tone, children }: { tone: "ok" | "warn" | "bad" | "muted"; children: ReactNode }) {
  return <span className={`badge ${tone}`}>{children}</span>;
}

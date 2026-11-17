import { DEMO_GATEWAYS, DEMO_PROJECTS } from "../demo/data";
import { DemoNotice, PageHeader } from "./common";

export function ProjectsPage() {
  const gatewayName = (id: string) => (id === "local" ? "Este Mac" : (DEMO_GATEWAYS.find((g) => g.id === id)?.name ?? id));
  return (
    <section>
      <PageHeader title="Projetos" subtitle="Workspaces autorizados em cada Gateway. Os caminhos ficam no Gateway, não no Mac." />
      <DemoNotice />
      <table className="table">
        <thead>
          <tr><th>Projeto</th><th>Gateway</th><th>Caminho no Gateway</th></tr>
        </thead>
        <tbody>
          {DEMO_PROJECTS.map((p) => (
            <tr key={p.id}>
              <td>{p.name}</td>
              <td>{gatewayName(p.gateway_id)}</td>
              <td><code>{p.path}</code></td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

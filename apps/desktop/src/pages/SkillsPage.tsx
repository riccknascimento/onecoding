import { DEMO_CAPABILITIES } from "../demo/data";
import { DemoNotice, PageHeader } from "./common";

const KIND = { skill: "Skill", agent: "Agent", command: "Command", plugin: "Plugin", mcp: "MCP" } as const;
const SCOPE = { global: "Global", gateway: "Gateway", project: "Projeto", cli: "CLI" } as const;

export function SkillsPage() {
  return (
    <section>
      <PageHeader title="Skills & Agents" subtitle="Catálogo de capacidades por escopo e motor (OneSkills Registry)." />
      <DemoNotice />
      <table className="table">
        <thead>
          <tr><th>Nome</th><th>Tipo</th><th>Escopo</th><th>Motor</th><th>Versão</th><th>Origem</th></tr>
        </thead>
        <tbody>
          {DEMO_CAPABILITIES.map((c) => (
            <tr key={c.id}>
              <td>{c.name}</td>
              <td>{KIND[c.kind]}</td>
              <td>{SCOPE[c.scope]}</td>
              <td>{c.engine ?? "—"}</td>
              <td>{c.version ?? "—"}</td>
              <td>{c.source}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

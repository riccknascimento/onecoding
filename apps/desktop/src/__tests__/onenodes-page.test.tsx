import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { OneNodesPage, nodeStateLabel } from "../central/OneNodesPage";
import { OneHubError } from "../transport";
import type { CentralNode, CentralNodeDetail, OneHubTransport } from "../transport";

const APLIFLOW: CentralNode = { id: "n-ap", name: "apliflow-vps", status: "online", last_seen_at: new Date().toISOString(), catalog_in_sync: true, suspended: false };
const FLOW: CentralNode = { id: "n-fs", name: "FlowSharp", status: "offline", last_seen_at: null, catalog_in_sync: false, suspended: true };
const PENDING: CentralNode = { id: "n-new", name: "Novo servidor", status: "pending", last_seen_at: null, catalog_in_sync: true, suspended: false };

function detail(over: Partial<CentralNodeDetail> = {}): CentralNodeDetail {
  return {
    id: "n-ap", name: "apliflow-vps", status: "online", suspended: false, paired_at: "2026-11-18T20:46:00Z",
    last_seen_at: new Date().toISOString(), access_type: "direct", catalog_version: 3, applied_version: 3, applied_at: "2026-11-18T20:50:00Z",
    catalog_in_sync: true, applied_report: { accepted: [], rejected: [] }, grants: [], ...over,
  };
}

function fakeTransport(over: Partial<Record<keyof OneHubTransport, unknown>> = {}) {
  const base = {
    nodes: vi.fn(async () => [APLIFLOW, FLOW, PENDING]),
    projects: vi.fn(async () => [{ id: "p1", name: "ERP" }]),
    nodeDetail: vi.fn(async (id: string) => (id === "n-ap" ? detail() : detail({ id, name: "FlowSharp", status: "offline", suspended: true, catalog_in_sync: false, applied_version: null }))),
    createNode: vi.fn(async () => ({ id: "n-x", pairing_code: "CODIGO-UNICO-1", pairing_expires_at: new Date(Date.now() + 900000).toISOString() })),
    createProject: vi.fn(async () => ({ id: "p9", name: "Novo" })),
    grantProject: vi.fn(async () => undefined),
    revokeGrant: vi.fn(async () => undefined),
    suspendNode: vi.fn(async () => undefined),
    resumeNode: vi.fn(async () => undefined),
    revokeNode: vi.fn(async () => undefined),
    ...over,
  };
  return base as unknown as OneHubTransport & typeof base;
}

describe("OneNodes: estados reais", () => {
  it("nomeia cada estado a partir dos dados do OneHub", () => {
    expect(nodeStateLabel({ status: "online" })).toEqual({ text: "Online", tone: "ok" });
    expect(nodeStateLabel({ status: "offline" })).toEqual({ text: "Offline", tone: "bad" });
    expect(nodeStateLabel({ status: "online", suspended: true })).toEqual({ text: "Suspenso", tone: "warn" });
    expect(nodeStateLabel({ status: "revoked" })).toEqual({ text: "Revogado", tone: "bad" });
    expect(nodeStateLabel({ status: "pending" })).toEqual({ text: "Aguardando pareamento", tone: "info" });
  });

  it("lista o OneNode existente sem novo pareamento e mostra quando a configuração não foi aplicada", async () => {
    render(<OneNodesPage transport={fakeTransport()} />);
    const list = await screen.findByRole("list", { name: "Lista de OneNodes" });
    expect(within(list).getByRole("button", { name: "apliflow-vps" })).toBeInTheDocument();
    expect(within(list).getByText("Suspenso")).toBeInTheDocument();
    expect(within(list).getByText("Aguardando pareamento")).toBeInTheDocument();
    expect(within(list).getByText(/configuração ainda não aplicada/)).toBeInTheDocument();
  });

  it("mostra zero projetos como estado válido e não cria botão de instalação por SSH", async () => {
    render(<OneNodesPage transport={fakeTransport()} />);
    await userEvent.click(await screen.findByRole("button", { name: "apliflow-vps" }));
    expect(await screen.findByText(/Nenhum projeto\. Um OneNode sem projetos continua Online\./)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /instalar/i })).not.toBeInTheDocument();
    expect(screen.getByText(/instalação automática por SSH ainda não está disponível/i)).toBeInTheDocument();
  });
});

describe("OneNodes: cadastro e código de pareamento", () => {
  it("gera o código, mostra uma vez e não o guarda no formulário", async () => {
    const t = fakeTransport();
    render(<OneNodesPage transport={t} />);
    await userEvent.type(await screen.findByLabelText("Nome do servidor"), "Servidor novo");
    await userEvent.click(screen.getByRole("button", { name: "Gerar código de pareamento" }));
    expect(t.createNode).toHaveBeenCalledWith("Servidor novo");
    expect(await screen.findByText("CODIGO-UNICO-1")).toBeInTheDocument();
    expect(screen.getByText(/exibido uma única vez/)).toBeInTheDocument();
    expect(screen.getByLabelText("Nome do servidor")).toHaveValue("");
  });
});

describe("OneNodes: concessões com caminho autorizado", () => {
  it("não permite conceder sem pasta autorizada e envia a raiz junto com o projeto", async () => {
    const t = fakeTransport();
    render(<OneNodesPage transport={t} />);
    await userEvent.click(await screen.findByRole("button", { name: "apliflow-vps" }));
    const submit = await screen.findByRole("button", { name: "Conceder acesso" });
    expect(submit).toBeDisabled();
    await userEvent.selectOptions(screen.getByLabelText("Projeto existente"), "p1");
    await userEvent.type(screen.getByLabelText("Pasta autorizada neste servidor"), "/opt/onenode/workspace/erp");
    await userEvent.click(submit);
    await waitFor(() => expect(t.grantProject).toHaveBeenCalledWith("p1", "n-ap", "/opt/onenode/workspace/erp"));
  });

  it("mostra o erro real do backend para caminho inválido", async () => {
    const t = fakeTransport({
      grantProject: vi.fn(async () => {
        throw new OneHubError(400, "root_path_invalid");
      }),
    });
    render(<OneNodesPage transport={t} />);
    await userEvent.click(await screen.findByRole("button", { name: "apliflow-vps" }));
    await userEvent.selectOptions(await screen.findByLabelText("Projeto existente"), "p1");
    await userEvent.type(screen.getByLabelText("Pasta autorizada neste servidor"), "/a/../etc");
    await userEvent.click(screen.getByRole("button", { name: "Conceder acesso" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Caminho inválido");
  });

  it("mostra as raízes recusadas pela política local com o motivo", async () => {
    const t = fakeTransport({
      nodeDetail: vi.fn(async () =>
        detail({
          grants: [{ project_id: "p1", project_name: "ERP", root_path: "/root/erp" }],
          applied_report: { accepted: [], rejected: [{ project_id: "p1", reason: "pasta de sistema" }] },
        }),
      ),
    });
    render(<OneNodesPage transport={t} />);
    await userEvent.click(await screen.findByRole("button", { name: "apliflow-vps" }));
    expect(await screen.findByText(/Raízes recusadas pela política local/)).toBeInTheDocument();
    expect(screen.getByText(/ERP: pasta de sistema/)).toBeInTheDocument();
  });
});

describe("OneNodes: suspender, retomar e revogar com confirmação", () => {
  it("suspende o OneNode pelo botão real", async () => {
    const t = fakeTransport();
    render(<OneNodesPage transport={t} />);
    await userEvent.click(await screen.findByRole("button", { name: "apliflow-vps" }));
    await userEvent.click(await screen.findByRole("button", { name: "Suspender OneNode" }));
    await waitFor(() => expect(t.suspendNode).toHaveBeenCalledWith("n-ap"));
  });

  it("retoma um OneNode suspenso", async () => {
    const t = fakeTransport();
    render(<OneNodesPage transport={t} />);
    await userEvent.click(await screen.findByRole("button", { name: "FlowSharp" }));
    await userEvent.click(await screen.findByRole("button", { name: "Retomar OneNode" }));
    await waitFor(() => expect(t.resumeNode).toHaveBeenCalledWith("n-fs"));
  });

  it("revogar OneNode pede confirmação e não chama a API se cancelar", async () => {
    const t = fakeTransport();
    render(<OneNodesPage transport={t} />);
    await userEvent.click(await screen.findByRole("button", { name: "apliflow-vps" }));
    await userEvent.click(await screen.findByRole("button", { name: "Revogar OneNode" }));
    expect(screen.getByText(/Revogar encerra o acesso deste servidor/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Cancelar" }));
    expect(t.revokeNode).not.toHaveBeenCalled();
  });

  it("revogar concessão pede confirmação antes de chamar a API", async () => {
    const t = fakeTransport({
      nodeDetail: vi.fn(async () => detail({ grants: [{ project_id: "p1", project_name: "ERP", root_path: "/opt/onenode/workspace/erp" }] })),
    });
    render(<OneNodesPage transport={t} />);
    await userEvent.click(await screen.findByRole("button", { name: "apliflow-vps" }));
    await userEvent.click(await screen.findByRole("button", { name: "Revogar acesso" }));
    expect(t.revokeGrant).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Confirmar revogação" }));
    await waitFor(() => expect(t.revokeGrant).toHaveBeenCalledWith("p1", "n-ap"));
  });
});

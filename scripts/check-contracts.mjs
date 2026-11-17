// Garante que os tipos TS gerados pelo ts-rs estão em dia com o Rust.
// Regenera (cargo test -p one-protocol) e compara o conteúdo antes/depois, sem depender do git.
import { execSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const dir = "apps/desktop/src/contracts/generated";
const digest = () => {
  const h = createHash("sha256");
  for (const f of readdirSync(dir).sort()) h.update(f).update(readFileSync(join(dir, f)));
  return h.digest("hex");
};

const before = digest();
execSync("cargo test -p one-protocol --quiet", { stdio: "inherit" });
if (digest() !== before) {
  console.error(`Contratos TS desatualizados em ${dir}. Rode "cargo test -p one-protocol" e versione o resultado.`);
  process.exit(1);
}
console.log("Contratos TS em dia com o Rust.");

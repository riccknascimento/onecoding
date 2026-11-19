export { createHttpTransport } from "./http";
export { OneHubError } from "./types";
export type * from "./types";
import { createHttpTransport } from "./http";

/** Transporte padrão da Web. A tela recebe um transporte por parâmetro, então testes podem trocá-lo. */
export const webTransport = createHttpTransport();

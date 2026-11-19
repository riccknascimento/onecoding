// Markdown das respostas. O react-markdown escapa HTML bruto por padrão: nada da resposta vira marcação.
import { useState } from "react";
import type { ReactNode } from "react";
import { isValidElement } from "react";
import ReactMarkdown from "react-markdown";

/** Texto puro de um nó React (para copiar o bloco de código). */
function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  if (isValidElement<{ children?: ReactNode }>(node)) return textOf(node.props.children);
  return "";
}

function CodeBlock({ children }: { children?: ReactNode }) {
  const [copied, setCopied] = useState(false);
  const code = textOf(children).replace(/\n$/, "");
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      setCopied(false);
    }
  };
  return (
    <div className="code-block">
      <div className="code-head">
        <button type="button" className="btn code-copy" onClick={copy} aria-label="Copiar código">
          {copied ? "Copiado" : "Copiar"}
        </button>
      </div>
      <pre>{children}</pre>
    </div>
  );
}

export function Markdown({ text }: { text: string }) {
  return (
    <div className="md">
      <ReactMarkdown
        components={{
          pre: ({ children }) => <CodeBlock>{children}</CodeBlock>,
          a: ({ href, children }) => (
            <a href={href} target="_blank" rel="noopener noreferrer">
              {children}
            </a>
          ),
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
}

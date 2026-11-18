import { useRef } from "react";
import { Icon } from "../icons";

interface Props {
  value: string;
  onChange: (v: string) => void;
  onSubmit: () => void;
  placeholder: string;
  /** Impede digitar (ex.: provedor desconectado). */
  disabled: boolean;
  /** Impede enviar sem impedir digitar. */
  canSend: boolean;
  hint: string;
}

export function Composer({ value, onChange, onSubmit, placeholder, disabled, canSend, hint }: Props) {
  const ref = useRef<HTMLTextAreaElement>(null);
  const grow = () => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  };
  return (
    <form
      className="composer"
      onSubmit={(e) => {
        e.preventDefault();
        if (canSend) onSubmit();
      }}
    >
      <textarea
        ref={ref}
        value={value}
        onChange={(e) => {
          onChange(e.target.value);
          grow();
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
            e.preventDefault();
            e.currentTarget.form?.requestSubmit();
          }
        }}
        placeholder={placeholder}
        rows={1}
        aria-label="Mensagem"
        disabled={disabled}
      />
      <div className="composer-foot">
        <span className="composer-hint">{hint}</span>
        <button type="submit" className="send" disabled={!canSend} aria-label="Enviar">
          <Icon name="send" />
        </button>
      </div>
    </form>
  );
}

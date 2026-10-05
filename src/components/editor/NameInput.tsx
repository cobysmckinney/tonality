import { useEffect, useRef } from "react";

/**
 * A name being changed in place. Enter or clicking away keeps what was
 * typed; Escape keeps the old name.
 */
export function NameInput({ name, label, onDone }: { name: string; label: string; onDone: (name: string) => void }) {
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.select(), []);
  return (
    <input
      ref={input}
      className="branch-name"
      defaultValue={name}
      aria-label={label}
      onBlur={(event) => onDone(event.currentTarget.value)}
      onKeyDown={(event) => {
        if (event.key === "Enter") event.currentTarget.blur();
        if (event.key === "Escape") onDone(name);
        // Typing a name must not trigger the editor's single-key shortcuts.
        event.stopPropagation();
      }}
    />
  );
}

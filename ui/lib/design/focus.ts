/**
 * Svelte action for a dialog: on open, focus moves into it (to the element matching `selector`
 * inside it, or the dialog itself); on close, back to whatever had it.
 *
 * Without it focus stays on the button that opened the dialog, behind the scrim, and Enter or
 * Space presses that button again instead of confirming the dialog.
 */
export function holdFocus(node: HTMLElement, selector?: string) {
  const back = document.activeElement as HTMLElement | null;
  ((selector && node.querySelector<HTMLElement>(selector)) || node).focus();
  return {
    destroy() {
      if (back?.isConnected) back.focus();
    },
  };
}

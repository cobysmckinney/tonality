/**
 * Giving the focus back when something that took it goes away, such as a menu
 * opened from a button.
 */

interface Focusable {
  isConnected: boolean;
}

/**
 * Where the focus should go when a menu closes: back to what had it when the
 * menu opened, if that is still in the page and the focus was simply dropped
 * (it sits on the page itself). If the chosen item moved the focus somewhere
 * on purpose, such as a dialog's button or a name field, it stays there.
 */
export function focusToRestore<T extends Focusable>(opener: T | null, active: unknown, page: unknown): T | null {
  if (!opener || !opener.isConnected || opener === page) return null;
  const dropped = active === null || active === page;
  return dropped ? opener : null;
}

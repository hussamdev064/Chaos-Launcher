/** DOM lookups are internal invariants: a missing selector is a developer error, not user input. */
export function queryRequired<T extends Element>(root: ParentNode, selector: string): T {
  const element = root.querySelector<T>(selector);
  if (!element) {
    throw new Error(`ChaosLauncher UI is missing required element: ${selector}`);
  }
  return element;
}

/** File names are user input, so escape them before interpolating into HTML templates. */
export function escapeHtml(value: string): string {
  return value.replace(
    /[&<>"']/g,
    (character) =>
      ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        '"': "&quot;",
        "'": "&#39;",
      })[character] ?? character,
  );
}

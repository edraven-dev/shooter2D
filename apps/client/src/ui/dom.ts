type Child = Node | string | false | null | undefined;

/** Creates an element with properties and children: a tiny stand-in for a UI framework. */
export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  properties: Partial<HTMLElementTagNameMap[K]> = {},
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const element = Object.assign(document.createElement(tag), properties);
  for (const child of children) {
    if (child !== false && child !== null && child !== undefined) {
      element.append(child);
    }
  }
  return element;
}

/** The message of an unknown thrown value. */
export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

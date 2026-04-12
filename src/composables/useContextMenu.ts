// Global context menu coordination — ensures only one context menu is open at a time.
// Every component that has a context menu calls closeAll() before opening its own.

const listeners: Set<() => void> = new Set();

export function onCloseAll(fn: () => void): void {
  listeners.add(fn);
}

export function offCloseAll(fn: () => void): void {
  listeners.delete(fn);
}

export function closeAll(): void {
  for (const fn of listeners) {
    fn();
  }
}

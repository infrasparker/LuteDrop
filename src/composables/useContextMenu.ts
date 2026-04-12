// Global context menu coordination — ensures only one context menu is open at a time.
// Every component that has a context menu calls closeAll() before opening its own.

import { ref, onMounted, onUnmounted } from 'vue';

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

// Reusable composable that manages context menu state for a component.
// Returns reactive refs for visibility/position and an openMenu helper
// that coordinates with the global close system.
export function useContextMenuState() {
  const menuVisible = ref(false);
  const menuX = ref(0);
  const menuY = ref(0);

  function closeMenu(): void {
    menuVisible.value = false;
  }

  function openMenu(event: MouseEvent): void {
    closeAll();
    menuX.value = event.clientX;
    menuY.value = event.clientY;
    menuVisible.value = true;
  }

  onMounted(() => onCloseAll(closeMenu));
  onUnmounted(() => offCloseAll(closeMenu));

  return { menuVisible, menuX, menuY, openMenu };
}

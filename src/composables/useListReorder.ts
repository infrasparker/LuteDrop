import { ref, watch, type Ref } from 'vue';
import Sortable from 'sortablejs';

/** Handle-only sorting; Vue owns the final DOM order after the command hydrates. */
export function useListReorder(
  container: Ref<HTMLElement | null>,
  options: {
    item: string;
    handle: string;
    context: () => string;
    ids: () => string[];
    move: (id: string, index: number) => Promise<unknown>;
  },
) {
  const saving = ref(false);
  watch(container, (element, _previous, onCleanup) => {
    if (!element) return;
    let startedContext = '';
    let startedIds: string[] = [];
    let destroyed = false;
    const sortable = Sortable.create(element, {
      draggable: options.item,
      handle: options.handle,
      animation: 160,
      forceFallback: true,
      fallbackOnBody: true,
      fallbackClass: 'stage-drag-preview',
      ghostClass: 'stage-drag-placeholder',
      delay: 160,
      delayOnTouchOnly: true,
      onStart: () => {
        startedContext = options.context();
        startedIds = [...options.ids()];
      },
      onEnd: async (event) => {
        // Undo Sortable's move before Vue patches its keyed list, including failures.
        if (event.from.isConnected && event.item.parentNode === event.from) {
          const nextId = startedIds[(event.oldDraggableIndex ?? -1) + 1];
          const next = [...event.from.children].find((child) => child !== event.item
            && (child as HTMLElement).dataset.reorderId === nextId);
          event.from.insertBefore(event.item, next ?? null);
        }
        const index = event.newDraggableIndex;
        const id = event.item.dataset.reorderId;
        if (saving.value || index === undefined || index === event.oldDraggableIndex || !id
          || startedContext !== options.context() || startedIds.join('\0') !== options.ids().join('\0')) return;
        saving.value = true;
        sortable.option('disabled', true);
        try { await options.move(id, index); }
        finally { saving.value = false; if (!destroyed) sortable.option('disabled', false); }
      },
    });
    onCleanup(() => { destroyed = true; sortable.destroy(); });
  });
  return { saving };
}

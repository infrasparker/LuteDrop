import { ref, watch } from 'vue';

/// Component-local slider value mirrored from a store/prop source, for smooth
/// dragging while the engine persists on input. `source` is a getter for the
/// authoritative value; `commit` (optional) is called on every input — omit it
/// for deferred-commit callers (e.g. persist-on-close), which write `local`
/// themselves. Replaces five hand-rolled ref+watch+setter copies.
export function useLocalVolume(source: () => number | undefined, commit?: (v: number) => void) {
  const local = ref(source() ?? 1);
  watch(source, (v) => {
    if (v !== undefined) local.value = v;
  });
  function set(v: number): void {
    local.value = v;
    commit?.(v);
  }
  return { local, set };
}

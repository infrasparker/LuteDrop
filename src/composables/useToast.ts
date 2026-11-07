import { ref } from 'vue';

const message = ref('');
const visible = ref(false);
const color = ref('error');
/// An optional button on the toast (such as Undo), cleared by the next toast.
const action = ref<{ label: string; run: () => void } | null>(null);

export function useToast() {
  function showError(msg: string): void {
    message.value = msg;
    color.value = 'error';
    action.value = null;
    visible.value = true;
  }

  function showSuccess(msg: string, act: { label: string; run: () => void } | null = null): void {
    message.value = msg;
    color.value = 'success';
    action.value = act;
    visible.value = true;
  }

  /// A neutral note, with an optional action button.
  function showInfo(msg: string, act: { label: string; run: () => void } | null = null): void {
    message.value = msg;
    color.value = '';
    action.value = act;
    visible.value = true;
  }

  function runAction(): void {
    const a = action.value;
    visible.value = false;
    action.value = null;
    a?.run();
  }

  return { message, visible, color, action, showError, showSuccess, showInfo, runAction };
}

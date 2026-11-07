import { ref } from 'vue';

const message = ref('');
const visible = ref(false);
const color = ref('error');
/// An optional button on the toast (such as Undo), cleared by the next toast.
const action = ref<{ label: string; run: () => void } | null>(null);
// Errors and toasts with an action stay up longer than plain confirmations;
// any toast can be dismissed early.
const timeout = ref(2500);

type ToastAction = { label: string; run: () => void } | null;

export function useToast() {
  function show(msg: string, tone: string, act: ToastAction, ms: number): void {
    message.value = msg;
    color.value = tone;
    action.value = act;
    timeout.value = act ? Math.max(ms, 6000) : ms;
    visible.value = true;
  }

  function showError(msg: string): void {
    show(msg, 'error', null, 6000);
  }

  function showSuccess(msg: string, act: ToastAction = null): void {
    show(msg, 'success', act, 2500);
  }

  /// A neutral note, with an optional action button.
  function showInfo(msg: string, act: ToastAction = null): void {
    show(msg, '', act, 2500);
  }

  function runAction(): void {
    const a = action.value;
    visible.value = false;
    action.value = null;
    a?.run();
  }

  function hide(): void {
    visible.value = false;
  }

  return { message, visible, color, action, timeout, showError, showSuccess, showInfo, runAction, hide };
}

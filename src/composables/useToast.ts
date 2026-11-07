import { ref } from 'vue';

const message = ref('');
const visible = ref(false);
const color = ref('error');
// Errors stay up longer than confirmations; either can be dismissed early.
const timeout = ref(2500);

export function useToast() {
  function showError(msg: string): void {
    message.value = msg;
    color.value = 'error';
    timeout.value = 6000;
    visible.value = true;
  }

  function showSuccess(msg: string): void {
    message.value = msg;
    color.value = 'success';
    timeout.value = 2500;
    visible.value = true;
  }

  function hide(): void {
    visible.value = false;
  }

  return { message, visible, color, timeout, showError, showSuccess, hide };
}

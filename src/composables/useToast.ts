import { ref } from 'vue';

const message = ref('');
const visible = ref(false);
const color = ref('error');

export function useToast() {
  function showError(msg: string): void {
    message.value = msg;
    color.value = 'error';
    visible.value = true;
  }

  function showSuccess(msg: string): void {
    message.value = msg;
    color.value = 'success';
    visible.value = true;
  }

  return { message, visible, color, showError, showSuccess };
}

import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useToast } from './useToast';
import type { BroadcastStatus } from '@generated/index';

// Module-level state: every component that calls useBroadcast() shares it.
const status = ref<BroadcastStatus | null>(null);
const busy = ref(false);
let started = false;

function start(): void {
  if (started) return;
  started = true;

  // Pushed by the backend when a player connects/disconnects; null when off.
  // It also tells us whether the stream is running, so `enabled` follows it
  // too: if the first status read raced the startup restore, the transport
  // badge still appears once the stream is up.
  listen<number | null>('broadcast-listeners', (event) => {
    if (!status.value) return;
    status.value.enabled = event.payload !== null;
    status.value.listenerCount = event.payload ?? 0;
  });

  void refresh();
}

// Runs one backend command, stores the returned status, and reports a
// rejection (a database failure) both as a toast and inline in the dialog.
// A failed *enable* does not reject; it resolves with `error` set.
async function run(command: string, args?: Record<string, unknown>): Promise<void> {
  busy.value = true;
  try {
    status.value = await invoke<BroadcastStatus>(command, args);
  } catch (e) {
    const message = `Broadcast: ${e}`;
    useToast().showError(message);
    if (status.value) status.value.error = message;
  } finally {
    busy.value = false;
  }
}

function refresh(): Promise<void> {
  return run('get_broadcast_status');
}

function setEnabled(on: boolean): Promise<void> {
  return run('set_broadcast_enabled', { enabled: on });
}

function saveSettings(port: number, publicBase: string): Promise<void> {
  return run('set_broadcast_settings', { port, publicBase });
}

function regenerateKey(): Promise<void> {
  return run('regenerate_broadcast_key');
}

export function useBroadcast() {
  start();
  return { status, busy, refresh, setEnabled, saveSettings, regenerateKey };
}

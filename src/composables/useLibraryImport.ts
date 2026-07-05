import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import type { AppState, BulkImportProgress } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useToast } from './useToast';
import { AUDIO_EXTENSIONS } from '../constants';

// Event listeners are registered once at module scope: they mutate the shared
// Pinia store so the bottom-bar indicator and Import menu stay in sync no
// matter which component happens to trigger the import.
let listenersRegistered = false;

function registerListeners(): void {
  if (listenersRegistered) return;
  listenersRegistered = true;

  const store = useAppStore();
  const toast = useToast();

  listen<BulkImportProgress>('library-import-progress', (event) => {
    store.importProgress = event.payload;
  });

  listen<BulkImportProgress>('library-import-complete', async (event) => {
    const { imported, skipped } = event.payload;
    try {
      const state = await invoke<AppState>('load_data');
      store.hydrate(state);
    } catch (e) {
      toast.showError(`Failed to refresh library: ${e}`);
    }
    store.importProgress = null;

    if (imported > 0) {
      toast.showSuccess(
        `Imported ${imported} file${imported === 1 ? '' : 's'}${
          skipped > 0 ? `, ${skipped} already in library` : ''
        }`,
      );
    } else if (skipped > 0) {
      toast.showSuccess(
        `All ${skipped} file${skipped === 1 ? '' : 's'} already in library`,
      );
    }
  });
}

export function useLibraryImport() {
  registerListeners();
  const store = useAppStore();
  const toast = useToast();

  /**
   * Opens a multi-file picker and resolves with the chosen audio paths.
   * Returns an empty array if the user cancels.
   */
  async function pickFiles(): Promise<string[]> {
    const paths = await open({
      multiple: true,
      filters: [{ name: 'Audio', extensions: AUDIO_EXTENSIONS }],
    });
    if (!paths) return [];
    return Array.isArray(paths) ? paths : [paths];
  }

  /**
   * Opens a folder picker and resolves with every audio file found underneath
   * (recursive, filtered by supported extensions on the backend).
   * Returns an empty array if the user cancels.
   */
  async function pickFolder(): Promise<string[]> {
    const selection = await open({ directory: true, multiple: false });
    if (!selection || Array.isArray(selection)) return [];
    try {
      return await invoke<string[]>('scan_folder_for_audio', { folderPath: selection });
    } catch (e) {
      toast.showError(`Failed to scan folder: ${e}`);
      return [];
    }
  }

  /**
   * Kicks off a background import for `paths` with `tags`. Returns immediately
   * after the backend accepts the task. Progress + completion are delivered via
   * `library-import-progress` / `library-import-complete` events which drive
   * `store.importProgress`.
   */
  async function runImport(paths: string[], tags: string[], entryType: string = 'music'): Promise<void> {
    if (paths.length === 0) return;
    if (store.importProgress) {
      toast.showError('An import is already running');
      return;
    }
    // Seed the indicator before the first event arrives so the UI responds immediately.
    store.importProgress = { processed: 0, imported: 0, skipped: 0, total: paths.length };
    try {
      await invoke<void>('bulk_import_library_entries', { paths, tags, entryType });
    } catch (e) {
      store.importProgress = null;
      toast.showError(`Import failed: ${e}`);
    }
  }

  return { pickFiles, pickFolder, runImport };
}

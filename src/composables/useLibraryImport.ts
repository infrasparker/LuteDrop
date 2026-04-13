import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import type { AppState } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useToast } from './useToast';

const AUDIO_EXTENSIONS = ['mp3', 'wav', 'ogg', 'flac', 'aac', 'm4a', 'wma'];

export function useLibraryImport() {
  const store = useAppStore();
  const toast = useToast();

  async function importFiles(): Promise<void> {
    const paths = await open({
      multiple: true,
      filters: [{ name: 'Audio', extensions: AUDIO_EXTENSIONS }],
    });
    if (!paths) return;
    const pathList = Array.isArray(paths) ? paths : [paths];
    await importPaths(pathList);
  }

  async function importFolder(): Promise<void> {
    const paths = await open({
      multiple: true,
      filters: [{ name: 'Audio', extensions: AUDIO_EXTENSIONS }],
    });
    if (!paths) return;
    const pathList = Array.isArray(paths) ? paths : [paths];
    await importPaths(pathList);
  }

  async function importPaths(paths: string[]): Promise<void> {
    let imported = 0;
    let skipped = 0;

    for (const filePath of paths) {
      const fileName = filePath.split(/[/\\]/).pop()?.replace(/\.[^.]+$/, '') || 'Unknown';
      try {
        const state = await invoke<AppState>('import_library_entry', {
          filePath,
          contentHash: null,
          name: fileName,
          duration: null,
          sampleRate: null,
          fileSize: null,
        });
        store.hydrate(state);
        imported++;
      } catch (e) {
        const msg = String(e);
        if (msg.includes('UNIQUE constraint') || msg.includes('Already in library')) {
          skipped++;
        } else {
          toast.showError(`Failed to import ${fileName}: ${msg}`);
        }
      }
    }

    if (imported > 0) {
      toast.showSuccess(
        `Imported ${imported} file${imported === 1 ? '' : 's'}${skipped > 0 ? `, ${skipped} already in library` : ''}`
      );
    } else if (skipped > 0) {
      toast.showSuccess(
        `All ${skipped} file${skipped === 1 ? '' : 's'} already in library`
      );
    }
  }

  return { importFiles, importFolder };
}

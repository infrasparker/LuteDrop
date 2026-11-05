import { onMounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { SceneConfig } from '@generated/index';

// Used to tell game tags from analyzer tags when the saved vocabulary cannot be read.
export const DEFAULT_VOCABULARY = ['folk', 'electronic', 'playful', 'driving'];

// The analyzer vocabulary, loaded from the scene config on mount unless
// `loadOnMount` is false (then call `reloadVocabulary`). Starts as the default
// and keeps it if the config cannot be read.
export function useVocabulary(loadOnMount = true) {
  const vocabulary = ref(new Set(DEFAULT_VOCABULARY));

  async function load(): Promise<void> {
    try {
      const config = await invoke<SceneConfig>('get_scene_config');
      vocabulary.value = new Set(config.vocabulary);
    } catch {
      // Keep the current vocabulary; it only decides which tags show as game tags.
    }
  }

  if (loadOnMount) onMounted(load);

  return { vocabulary, reloadVocabulary: load };
}

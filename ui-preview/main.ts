// UI preview harness: mounts the real App in a plain browser with Tauri's
// IPC mocked. Commands answer from fixtures exported from a real library
// (ui-preview/export_fixtures.py); state changes (flags, levels, accepted
// lines, the vocabulary) live in memory until the page reloads.
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks';
import type { AppState, FlagBand, FlagBound, FlagKind, MoodDef, MoodTunePrefs, TuneMode, TuneResult } from '@generated/index';
import { stubBars, stubTune } from './stubTune';
import stateFixture from './fixtures/state.json';
import tuneFixture from './fixtures/tune.json';
import settingsFixture from './fixtures/settings.json';
import caseImportFixture from './fixtures/case-import.json';

type Args = Record<string, unknown>;

const state = structuredClone(stateFixture) as unknown as AppState;
const tune = tuneFixture as unknown as Record<string, TuneResult>;

let tunePrefs: Record<string, MoodTunePrefs> = {};
const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));
const entry = (id: unknown) => state.libraryEntries.find((e) => e.id === id);
const moodValue = (entryId: unknown, mood: unknown) => entry(entryId)?.moods.find((m) => m.mood === mood);
const appBound = (band: FlagBand | null): FlagBound =>
  band === 'definitely' || band === 'understandable' ? 'at_least' : 'in';

/// A flat, plausible waveform (the fixtures hold no audio).
function peaks(width: number): number[] {
  return Array.from({ length: width }, (_, i) => 0.35 + 0.25 * Math.abs(Math.sin(i / 17)) + 0.15 * Math.abs(Math.sin(i / 3.1)));
}

async function handle(cmd: string, a: Args): Promise<unknown> {
  switch (cmd) {
    case 'load_data':
      return state;

    // --- Moods ---
    case 'tune_mood': {
      // Compare answers fast for any lines; a search takes a moment.
      await sleep(a.mode === 'compare' ? 120 : 900);
      return stubTune(
        state,
        a.moodId as string,
        a.mode as TuneMode,
        (a.candidates as string[][]) ?? [],
        (a.extraWords as string[]) ?? [],
        (a.bank as string[]) ?? [],
        (a.excludeWords as string[]) ?? [],
        (a.focusEntryId as string | null) ?? null,
        tune[`${a.moodId}:${a.mode}`],
      );
    }
    case 'load_mood_tune_prefs':
      return tunePrefs;

    case 'set_mood_flag': {
      const v = moodValue(a.entryId, a.mood);
      if (v) {
        const kind = a.kind as FlagKind;
        const band = (a.band ?? null) as FlagBand | null;
        v.flag = {
          kind,
          band,
          bound: kind === 'band' ? appBound(band) : kind === 'higher' ? 'at_least' : 'at_most',
          barAtFlag: kind === 'band' ? null : v.auto,
          calibratedAt: null,
          stale: false,
          staleReason: null,
        };
      }
      return state;
    }
    case 'clear_mood_flag': {
      const v = moodValue(a.entryId, a.mood);
      if (v) v.flag = null;
      return state;
    }
    case 'keep_mood_flag': {
      const v = moodValue(a.entryId, a.mood);
      if (v?.flag) v.flag = { ...v.flag, stale: false, staleReason: null };
      return state;
    }
    case 'set_entry_mood_level':
      for (const id of a.entryIds as string[]) {
        const v = moodValue(id, a.mood);
        if (v) v.user = a.level as number | null;
      }
      return state;
    case 'accept_tuned_lines': {
      const m = state.moodVocabulary.moods.find((x) => x.id === a.moodId);
      if (m && JSON.stringify(a.linesTunedFrom) !== JSON.stringify(m.prompts)) {
        throw "This mood's lines have changed since Tune ran; run Tune again";
      }
      if (m) {
        // The bars follow the lines used, as a rescore would.
        const bars = stubBars(state, m.id, a.lines as string[]);
        for (const e of state.libraryEntries) {
          const v = e.moods.find((x) => x.mood === m.id);
          const b = bars.get(e.id);
          if (v && b !== undefined) v.auto = b;
        }
        m.prompts = a.lines as string[];
      }
      return { state, converted: [] };
    }
    case 'restore_mood_flags':
      return state;
    case 'rescore_moods':
      await sleep(300);
      return null;
    case 'add_never_suggest': {
      const list = tunePrefs[a.mood as string]?.neverSuggest ?? [];
      tunePrefs = { ...tunePrefs, [a.mood as string]: { neverSuggest: [...list, ...(a.words as string[])] } };
      return tunePrefs;
    }
    case 'remove_never_suggest': {
      const list = (tunePrefs[a.mood as string]?.neverSuggest ?? []).filter((w) => w.toLowerCase() !== (a.word as string).toLowerCase());
      tunePrefs = { ...tunePrefs, [a.mood as string]: { neverSuggest: list } };
      return tunePrefs;
    }
    case 'accept_tuned_split': {
      const m = state.moodVocabulary.moods.find((x) => x.id === a.moodId);
      if (m) m.prompts = a.keepLines as string[];
      const added: MoodDef = { id: a.newId as string, name: a.newName as string, prompts: a.newLines as string[] };
      state.moodVocabulary.moods.push(added);
      state.moodsNeedRecalibration = true;
      return state;
    }
    case 'set_mood_vocabulary':
      state.moodVocabulary = a.vocabulary as AppState['moodVocabulary'];
      state.moodsNeedRecalibration = true;
      return state;
    case 'recalibrate_moods':
      state.moodsNeedRecalibration = false;
      return state;
    case 'mood_reset_impact':
      return { moods: [], rows: 0, userLevels: 0 };
    case 'import_case_studies':
      await sleep(400);
      return caseImportFixture;
    case 'preview_mood_prompts':
      return [];
    case 'preview_energy_prompts':
      return { most: [], least: [] };

    // --- Library, editor, analyzer ---
    case 'get_waveform_data': {
      const e = state.libraryEntries.find((x) => x.filePath === a.filePath);
      return { peaks: peaks((a.width as number) ?? 1200), duration: e?.duration ?? 120 };
    }
    case 'get_entry_tag_scores':
      return null;
    case 'get_library_entry_usage':
    case 'scry_map':
    case 'scan_folder_for_audio':
      return [];
    case 'get_autotag_settings':
      return settingsFixture.autotag;
    case 'get_scene_config':
      return { vocabulary: state.tags };
    case 'get_autotag_report':
      return { finishedAt: null, analyzed: 0, added: 0, removed: 0, failed: 0, unreachable: 0, gate: null, tags: [], error: null, sampleErrors: [] };
    case 'test_autotag_connection':
      return 'Preview: no analyzer';
    case 'get_server_info':
      return { url: 'http://192.168.0.10:3000' };
    case 'get_broadcast_status':
    case 'set_broadcast_enabled':
    case 'set_broadcast_settings':
    case 'regenerate_broadcast_key':
      return settingsFixture.broadcast;

    // --- Playback: no audio in the preview ---
    case 'toggle_master_mute':
    case 'toggle_track_mute':
    case 'toggle_clip_mute':
    case 'toggle_comp_mute':
      return false;

    // --- Plugins ---
    case 'plugin:dialog|open':
      return 'L:/calibration-case-studies.json';
  }
  if (cmd.startsWith('plugin:')) return null;
  if (/^(play|pause|stop|resume|set_(master|track|clip|library)_volume|activate_comp|save_|analyze_)/.test(cmd)) return null;
  if (cmd.startsWith('scry')) return [];
  // Everything else is a state command: a no-op that returns the state.
  return state;
}

mockWindows('main');
// Answers go through JSON like real IPC, so the store never shares objects
// with the mock's state (an in-place change there would bypass reactivity).
mockIPC(
  async (cmd, payload) => {
    const result = await handle(cmd, (payload ?? {}) as Args);
    return result === undefined ? null : JSON.parse(JSON.stringify(result));
  },
  { shouldMockEvents: true },
);

// Loaded after the mocks are in place, so nothing reaches for Tauri first.
const [{ createApp }, { createPinia }, { default: App }, { vuetify }, { default: router }] = await Promise.all([
  import('vue'),
  import('pinia'),
  import('../src/App.vue'),
  import('../src/plugins/vuetify'),
  import('../src/router'),
]);
const app = createApp(App);
app.use(createPinia());
app.use(vuetify);
app.use(router);
app.mount('#app');

// For driving the preview from devtools or an automated browser.
(window as unknown as { __preview: unknown }).__preview = { state, router };

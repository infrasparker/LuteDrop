import { ref, type Ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';

interface WaveformData {
  peaks: number[];
  duration: number;
}

export function useWaveformData(filePath: Ref<string>) {
  const duration = ref(0);
  const loading = ref(false);
  const error = ref('');

  const basePeaks = ref<number[]>([]);
  const basePeaksRangeStart = ref(0);
  const basePeaksRangeEnd = ref(0);

  const zoomedPeaks = ref<number[]>([]);
  const zoomedPeaksRangeStart = ref(0);
  const zoomedPeaksRangeEnd = ref(0);

  let zoomDebounce: ReturnType<typeof setTimeout> | null = null;

  async function loadBasePeaks(): Promise<void> {
    if (!filePath.value) return;

    loading.value = true;
    error.value = '';

    try {
      const data = await invoke<WaveformData>('get_waveform_data', {
        filePath: filePath.value,
        width: 1200,
      });
      basePeaks.value = data.peaks;
      basePeaksRangeStart.value = 0;
      basePeaksRangeEnd.value = data.duration;
      zoomedPeaks.value = [];
      duration.value = data.duration;
    } catch (e: unknown) {
      error.value = String(e);
      basePeaks.value = [];
      zoomedPeaks.value = [];
      duration.value = 0;
    } finally {
      loading.value = false;
    }
  }

  /**
   * Request high-res peaks for a visible range. Debounced.
   * Pass `isZoomed`, `viewStart`, `viewEnd`, and `canvasPixelWidth`
   * from the zoom state and canvas.
   */
  function requestZoomedPeaks(
    isZoomed: boolean,
    viewStart: number,
    viewEnd: number,
    canvasPixelWidth: number,
  ): void {
    if (zoomDebounce) clearTimeout(zoomDebounce);
    zoomDebounce = setTimeout(async () => {
      if (duration.value <= 0 || !filePath.value) return;

      if (!isZoomed) {
        zoomedPeaks.value = [];
        return;
      }

      try {
        const data = await invoke<WaveformData>('get_waveform_data', {
          filePath: filePath.value,
          width: canvasPixelWidth,
          start: viewStart,
          end: viewEnd,
        });
        zoomedPeaks.value = data.peaks;
        zoomedPeaksRangeStart.value = viewStart;
        zoomedPeaksRangeEnd.value = viewEnd;
      } catch {
        // keep existing peaks on error
      }
    }, 150);
  }

  function cleanup(): void {
    if (zoomDebounce) clearTimeout(zoomDebounce);
  }

  return {
    duration,
    loading,
    error,
    basePeaks,
    basePeaksRangeStart,
    basePeaksRangeEnd,
    zoomedPeaks,
    zoomedPeaksRangeStart,
    zoomedPeaksRangeEnd,
    loadBasePeaks,
    requestZoomedPeaks,
    cleanup,
  };
}

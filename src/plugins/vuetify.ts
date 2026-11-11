import 'vuetify/styles';
import '@mdi/font/css/materialdesignicons.css';
import '../shared/mixer.css';
import { createVuetify } from 'vuetify';

export const vuetify = createVuetify({
  defaults: {
    // Desktop app: the browser's "Saved info" autofill list (previously typed titles,
    // search terms) is not meaningful here, so every text-like input opts out. The
    // Rust side also disables WebView2 general autofill; this is the per-input fallback.
    VTextField: { autocomplete: 'off' },
    VTextarea: { autocomplete: 'off' },
    VCombobox: { autocomplete: 'off' },
    VAutocomplete: { autocomplete: 'off' },
    VNumberInput: { autocomplete: 'off' },
    VSlider: { color: 'primary', trackColor: 'surface-light' },
  },
  theme: {
    defaultTheme: 'lutedrop',
    themes: {
      lutedrop: {
        dark: true,
        colors: {
          background: '#0A0A0C',
          surface: '#141418',
          'surface-light': '#1E1E24',
          'surface-variant': '#28282F',
          primary: '#F0A830',
          secondary: '#FF6D00',
          error: '#D32F2F',
          success: '#FF6D00',
          warning: '#BF360C',
          info: '#F0A830',
          'on-background': '#E8E4DC',
          'on-surface': '#E8E4DC',
          'on-primary': '#0A0A0C',
          'on-secondary': '#0A0A0C',
        },
      },
    },
  },
});

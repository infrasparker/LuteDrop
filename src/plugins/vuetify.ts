import 'vuetify/styles';
import '@mdi/font/css/materialdesignicons.css';
import { createVuetify } from 'vuetify';

export const vuetify = createVuetify({
  defaults: {
    VCard: { variant: 'flat', rounded: 'lg' },
    VBtn: { rounded: 'pill' },
    VTextField: { variant: 'solo-filled', rounded: 'lg' },
    VSelect: { variant: 'solo-filled', rounded: 'lg' },
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

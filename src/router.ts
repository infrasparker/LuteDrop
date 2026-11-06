import { createRouter, createWebHistory } from 'vue-router';
import SoundstageView from './views/SoundstageView.vue';
import LibraryView from './views/LibraryView.vue';
import ScryView from './views/ScryView.vue';
import ReviewView from './views/ReviewView.vue';

const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', name: 'soundstage', component: SoundstageView },
    { path: '/library', name: 'library', component: LibraryView },
    { path: '/scry', name: 'scry', component: ScryView },
    { path: '/review', name: 'review', component: ReviewView },
  ],
});

export default router;

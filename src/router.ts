import { createRouter, createWebHistory } from 'vue-router';

const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: '/',
      name: 'soundstage',
      component: () => import('./views/SoundstageView.vue'),
    },
    {
      path: '/library',
      name: 'library',
      component: () => import('./views/LibraryView.vue'),
    },
  ],
});

export default router;

import js from '@eslint/js';
import tseslint from 'typescript-eslint';
import pluginVue from 'eslint-plugin-vue';
import globals from 'globals';

export default tseslint.config(
  // .claude/** is gitignored tooling/worktree scratch space (not app source) —
  // excluded so stray agent worktrees under it never leak into lint results.
  { ignores: ['dist/**', 'dist-remote/**', 'generated/**', 'src-tauri/**', 'node_modules/**', '.claude/**', 'tagger/**'] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  ...pluginVue.configs['flat/recommended'],
  {
    files: ['**/*.vue'],
    languageOptions: {
      parserOptions: { parser: tseslint.parser },
    },
  },
  {
    languageOptions: {
      ecmaVersion: 2021,
      sourceType: 'module',
      globals: { ...globals.browser },
    },
    rules: {
      // Enforced already by tsconfig; keep ESLint focused on style/correctness.
      '@typescript-eslint/no-unused-vars': ['error', { argsIgnorePattern: '^_', varsIgnorePattern: '^_' }],

      // The codebase's established template style keeps a component's full
      // attribute list (and short inline content) on one line rather than
      // one-attribute-per-line; these vue3-strongly-recommended stylistic
      // rules fight that intentional, consistently-applied convention across
      // every .vue file. Off rather than reformatting ~250 pre-existing call
      // sites for a pure style preference with no correctness benefit.
      'vue/max-attributes-per-line': 'off',
      'vue/attributes-order': 'off',
      'vue/singleline-html-element-content-newline': 'off',
      'vue/multiline-html-element-content-newline': 'off',

      // False positive on Vuetify's dotted data-table slot names (e.g.
      // `#item.status`, `#item.actions`). Vue's own compiler special-cases
      // `v-slot`: a dot after the slot name is folded back into the slot
      // name rather than parsed as a modifier (see
      // @vue/compiler-core's `ondirmodifier`, which appends the segment to
      // `arg` when `currentProp.name === "slot"`). vue-eslint-parser does not
      // replicate that special case, so it misreads the dot as an
      // (unsupported) v-slot modifier. The templates are correct as written.
      'vue/valid-v-slot': 'off',
    },
  },
  {
    // Node-executed dev tooling (not app source, not bundled by Vite).
    files: ['scripts/**/*.mjs'],
    languageOptions: {
      globals: { ...globals.node },
    },
  },
  {
    // Foundry VTT module (plain browser JS; Foundry supplies these globals).
    files: ['foundry-module/**/*.js'],
    languageOptions: {
      globals: {
        ...globals.browser,
        game: 'readonly',
        Hooks: 'readonly',
        ui: 'readonly',
        foundry: 'readonly',
        CONFIG: 'readonly',
      },
    },
  },
);

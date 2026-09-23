import js from '@eslint/js';
import globals from 'globals';
import svelte from 'eslint-plugin-svelte';
import tsParser from '@typescript-eslint/parser';

export default [
	js.configs.recommended,
	...svelte.configs['flat/recommended'],
	{
		files: ['**/*.svelte'],
		languageOptions: {
			globals: { ...globals.browser },
			parserOptions: {
				parser: tsParser,
				extraFileExtensions: ['.svelte'],
				sourceType: 'module'
			}
		},
		rules: {
			// Type-position params (Props documentation) and destructures are
			// svelte-check's (tsc's) authority, not the base rule's.
			'no-unused-vars': 'off'
		}
	},
		{
			languageOptions: {
				globals: { ...globals.browser, ...globals.node }
			}
		},
		{
			rules: {
				// The shell renders NO remote content and NO raw HTML — ever
				// (the security posture; the sanitizer helper is the only text
				// path and it STRIPS, never injects).
				'no-restricted-syntax': [
					'error',
					{
						selector: "SvelteDirective[name='html']",
						message: '{@html} is banned in the shell: no raw HTML is rendered, ever.'
					}
				],
				'no-console': 'error'
			}
		},
		{
			// Build-time tooling under scripts/ is a Node CLI: its console IS
			// the output channel (the shell's no-console law targets app code).
			files: ['scripts/**/*.mjs'],
			rules: {
				'no-console': 'off'
			}
		},
	{
		ignores: ['build/', '.svelte-kit/', 'src-tauri/target/', 'playwright-report/', 'node_modules/']
	}
];

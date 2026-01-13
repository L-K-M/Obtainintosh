import { writable } from 'svelte/store';

// Window focus state - reactive across all components
export const windowFocused = writable(true);

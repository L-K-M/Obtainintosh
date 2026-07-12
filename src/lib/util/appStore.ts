import { writable } from 'svelte/store';
import type { App } from '$lib/types';
import { TauriService } from '$lib/tauri';
import { getErrorMessage } from '$lib/util/errors';
import { notifications } from './notifications';

function createAppStore() {
    const { subscribe, set, update } = writable<{
        apps: App[];
        loading: boolean;
        error: string | null;
    }>({
        apps: [],
        loading: false,
        error: null
    });

    return {
        subscribe,
        loadApps: async () => {
            update(s => ({ ...s, loading: true, error: null }));
            try {
                const apps = await TauriService.getAllApps();
                update(s => ({ ...s, apps, loading: false }));
            } catch (e) {
                const error = getErrorMessage(e, 'Failed to load apps');
                update(s => ({ ...s, error, loading: false }));
                notifications.add(error, 'error');
            }
        },

        addApp: async (url: string, name: string) => {
            update(s => ({ ...s, loading: true, error: null }));
            try {
                await TauriService.addApp(url, name);
                const apps = await TauriService.getAllApps();
                update(s => ({ ...s, apps, loading: false }));
                notifications.add(`App "${name}" added successfully`, 'success');
                return true;
            } catch (e) {
                const error = getErrorMessage(e, 'Failed to add program');
                update(s => ({ ...s, error, loading: false }));
                notifications.add(error, 'error');
                return false;
            }
        },

        removeApp: async (id: string, name: string) => {
            update(s => ({ ...s, loading: true, error: null }));
            try {
                await TauriService.removeApp(id);
                const apps = await TauriService.getAllApps();
                update(s => ({ ...s, apps, loading: false }));
                notifications.add(name ? `"${name}" removed` : 'App removed successfully', 'success');
                return true;
            } catch (e) {
                const error = getErrorMessage(e, 'Failed to remove app');
                update(s => ({ ...s, error, loading: false }));
                notifications.add(error, 'error');
                return false;
            }
        },

        // `quiet` suppresses the success notification (used for the automatic
        // check on launch); errors are always surfaced.
        checkForUpdates: async (quiet = false) => {
            update(s => ({ ...s, loading: true, error: null }));
            try {
                const apps = await TauriService.checkForUpdates();
                update(s => ({ ...s, apps, loading: false }));
                if (!quiet) {
                    notifications.add('Update check completed', 'success');
                }
            } catch (e) {
                const error = getErrorMessage(e, 'Failed to check updates');
                update(s => ({ ...s, error, loading: false }));
                notifications.add(error, 'error');
            }
        },

        downloadAndInstall: async (appId: string) => {
            update(s => ({ ...s, loading: true, error: null }));
            try {
                const result = await TauriService.downloadAndInstall(appId);
                notifications.add(result, 'success');
                // Reload apps to update version status
                const apps = await TauriService.getAllApps();
                update(s => ({ ...s, apps, loading: false }));
            } catch (e) {
                const error = getErrorMessage(e, 'Failed to install app');
                update(s => ({ ...s, error, loading: false }));
                notifications.add(error, 'error');
            }
        },

        clearError: () => {
            update(s => ({ ...s, error: null }));
        }
    };
}

export const appStore = createAppStore();

import { writable } from 'svelte/store';
import type { App, CheckOutcome } from '$lib/types';
import { TauriService } from '$lib/tauri';
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
                const error = e instanceof Error ? e.message : 'Failed to load apps';
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
                const error = e instanceof Error ? e.message : 'Failed to add program';
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
                const error = e instanceof Error ? e.message : 'Failed to remove app';
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
                const outcomes = await TauriService.checkForUpdates();
                const apps = await TauriService.getAllApps();
                update(s => ({ ...s, apps, loading: false }));
                const incomplete = outcomes.filter(outcome => outcome.state !== 'succeeded');
                if (incomplete.length > 0) {
                    notifications.add(checkFailureSummary(outcomes, incomplete), 'error');
                } else if (!quiet) {
                    notifications.add('Update check completed', 'success');
                }
            } catch (e) {
                const error = e instanceof Error
                    ? e.message
                    : typeof e === 'string'
                        ? e
                        : 'Failed to check updates';
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
                const error = e instanceof Error ? e.message : 'Failed to install app';
                update(s => ({ ...s, error, loading: false }));
                notifications.add(error, 'error');
            }
        },

        clearError: () => {
            update(s => ({ ...s, error: null }));
        }
    };
}

function checkFailureSummary(outcomes: CheckOutcome[], incomplete: CheckOutcome[]): string {
    if (incomplete.length === 1) {
        const outcome = incomplete[0];
        const detail = outcome.message || 'The update check did not complete';
        return `${outcome.app_name}: ${detail}`;
    }

    if (incomplete.length === outcomes.length) {
        return `Update checks failed or were skipped for all ${outcomes.length} apps`;
    }

    return `${incomplete.length} of ${outcomes.length} update checks failed or were skipped`;
}

export const appStore = createAppStore();

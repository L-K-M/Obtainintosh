import { writable } from 'svelte/store';
import type { App, CheckOutcome, SourceInput } from '$lib/types';
import { TauriService } from '$lib/tauri';
import { getErrorMessage } from '$lib/util/errors';
import { notifications } from './notifications';

function createAppStore() {
    const { subscribe, set, update } = writable<{
        apps: App[];
        loading: boolean;
        // True while an update check runs; drives the modal check-progress
        // dialog. Distinct from `loading`, which also covers add/remove/
        // download operations that must not open that dialog.
        checking: boolean;
        error: string | null;
    }>({
        apps: [],
        loading: false,
        checking: false,
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

        addApp: async (input: SourceInput) => {
            update(s => ({ ...s, loading: true, error: null }));
            try {
                await TauriService.addApp(input);
                const apps = await TauriService.getAllApps();
                update(s => ({ ...s, apps, loading: false }));
                notifications.add(`App "${input.name}" added successfully`, 'success');
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
            update(s => ({ ...s, loading: true, checking: true, error: null }));
            try {
                const outcomes = await TauriService.checkForUpdates();
                // The check now reports per-app outcomes rather than the apps
                // themselves, and it only writes the fields it owns, so the
                // list is re-read rather than reconstructed here.
                const apps = await TauriService.getAllApps();
                update(s => ({ ...s, apps, loading: false, checking: false }));
                const incomplete = outcomes.filter(outcome => outcome.state !== 'succeeded');
                if (incomplete.length > 0) {
                    // Surfaced even when quiet: a check that silently did
                    // nothing is exactly what this is meant to stop.
                    notifications.add(checkFailureSummary(outcomes, incomplete), 'error');
                } else if (!quiet) {
                    notifications.add('Update check completed', 'success');
                }
            } catch (e) {
                const error = getErrorMessage(e, 'Failed to check updates');
                update(s => ({ ...s, error, loading: false, checking: false }));
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

/** One line summarising what a check run did not manage to do. */
function checkFailureSummary(outcomes: CheckOutcome[], incomplete: CheckOutcome[]): string {
    if (incomplete.length === 1) {
        const outcome = incomplete[0];
        const detail = outcome.message || 'The update check did not complete';
        return `${outcome.appName}: ${detail}`;
    }

    if (incomplete.length === outcomes.length) {
        return `Update checks failed or were skipped for all ${outcomes.length} apps`;
    }

    return `${incomplete.length} of ${outcomes.length} update checks failed or were skipped`;
}

export const appStore = createAppStore();

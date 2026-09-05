import { writable } from 'svelte/store';
import type { App, CheckOutcome, ImportSummary, SourceInput } from '$lib/types';
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

        // Both of these open a native dialog on the Rust side; `loading`
        // stays set until it closes so the toolbar cannot start a second
        // operation underneath it.
        exportApps: async () => {
            update(s => ({ ...s, loading: true, error: null }));
            try {
                const summary = await TauriService.exportAppList();
                update(s => ({ ...s, loading: false }));
                if (summary) {
                    notifications.add(
                        `Exported ${plural(summary.count, 'program')} to ${summary.fileName}`,
                        'success'
                    );
                }
            } catch (e) {
                const error = getErrorMessage(e, 'Failed to export the program list');
                update(s => ({ ...s, error, loading: false }));
                notifications.add(error, 'error');
            }
        },

        // Resolves to whether any program was added, so the caller can decide
        // to check the list afterwards. An import only ever adds programs;
        // the list is re-read rather than patched because storage assigns
        // the ids and the backend detects installed versions.
        importApps: async (): Promise<boolean> => {
            update(s => ({ ...s, loading: true, error: null }));
            try {
                const summary = await TauriService.importAppList();
                if (!summary) {
                    update(s => ({ ...s, loading: false }));
                    return false;
                }
                const apps = await TauriService.getAllApps();
                update(s => ({ ...s, apps, loading: false }));
                notifications.add(importSummaryMessage(summary), summary.added > 0 ? 'success' : 'info');
                if (summary.missingKeys > 0) {
                    notifications.add(missingKeysMessage(summary.missingKeys), 'info');
                }
                if (summary.rejected.length > 0) {
                    notifications.add(rejectedEntriesMessage(summary), 'error');
                }
                return summary.added > 0;
            } catch (e) {
                const error = getErrorMessage(e, 'Failed to import the program list');
                update(s => ({ ...s, error, loading: false }));
                notifications.add(error, 'error');
                return false;
            }
        },

        clearError: () => {
            update(s => ({ ...s, error: null }));
        },

        revealDownload: async (appId: string) => {
            try {
                const result = await TauriService.revealDownloadedFile(appId);
                notifications.add(result, 'success');
            } catch (e) {
                const error = getErrorMessage(e, 'Failed to show the downloaded file');
                notifications.add(error, 'error');
                // The backend forgets a record whose file is gone (the download
                // folder is temporary); reload so the button reflects that.
                const apps = await TauriService.getAllApps();
                update(s => ({ ...s, apps }));
            }
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

function plural(count: number, singular: string, pluralForm = `${singular}s`): string {
    return `${count} ${count === 1 ? singular : pluralForm}`;
}

/** One line saying what an import added, and what it left alone. */
function importSummaryMessage(summary: ImportSummary): string {
    const added = `Imported ${plural(summary.added, 'program')} from ${summary.fileName}`;
    if (summary.duplicates === 0) return added;
    const duplicates =
        summary.duplicates === 1
            ? '1 was already tracked'
            : `${summary.duplicates} were already tracked`;
    return `${added} (${duplicates})`;
}

function missingKeysMessage(count: number): string {
    const programs = plural(count, 'imported Forgejo program');
    const keys = count === 1 ? 'needs its application key' : 'need their application keys';
    return `${programs} ${keys} entered again — keys are never written to an export. Edit the program to add it.`;
}

/**
 * Names the entries an import turned away, bounded so a file full of them
 * does not produce a notification the height of the window.
 */
function rejectedEntriesMessage(summary: ImportSummary): string {
    const shown = 3;
    const details = summary.rejected
        .slice(0, shown)
        .map(entry => `${entry.label}: ${entry.reason}`)
        .join('; ');
    const remaining = summary.rejected.length - shown;
    const more = remaining > 0 ? `; and ${plural(remaining, 'more entry', 'more entries')}` : '';
    const skipped = plural(summary.rejected.length, 'entry', 'entries');
    return `${skipped} of ${summary.fileName} could not be imported — ${details}${more}`;
}

export const appStore = createAppStore();

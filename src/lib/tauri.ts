import { invoke } from '@tauri-apps/api/core';
import type {
    App,
    CheckOutcome,
    ExportSummary,
    ImportSummary,
    Settings,
    SourceInput,
    SystemColors
} from './types';

export class TauriService {
    static async getAllApps(): Promise<App[]> {
        return await invoke('get_all_apps');
    }

    static async addApp(input: SourceInput): Promise<App> {
        return await invoke('add_app', { ...input });
    }

    static async updateApp(id: string, input: SourceInput): Promise<App> {
        return await invoke('update_app', { id, ...input });
    }

    static async removeApp(id: string): Promise<void> {
        await invoke('remove_app', { id });
    }

    static async checkForUpdates(appId?: string): Promise<CheckOutcome[]> {
        return await invoke('check_for_updates', { appId: appId || null });
    }

    static async downloadAndInstall(appId: string): Promise<string> {
        return await invoke('download_and_install', { appId });
    }

    static async revealDownloadedFile(appId: string): Promise<string> {
        return await invoke('reveal_downloaded_file', { appId });
    }

    static async getSettings(): Promise<Settings> {
        return await invoke('get_settings');
    }

    static async updateSettings(settings: Settings): Promise<void> {
        await invoke('update_settings', { settings });
    }

    static async getSystemColors(): Promise<SystemColors> {
        return await invoke('get_system_colors');
    }

    /**
     * Opens a native save dialog and writes the program list there. The
     * dialog lives on the Rust side, so no file path passes through here;
     * resolves to null when the user cancels.
     */
    static async exportAppList(): Promise<ExportSummary | null> {
        return await invoke('export_app_list');
    }

    /**
     * Opens a native open dialog and merges the chosen program list into the
     * tracked one. Resolves to null when the user cancels.
     */
    static async importAppList(): Promise<ImportSummary | null> {
        return await invoke('import_app_list');
    }
}

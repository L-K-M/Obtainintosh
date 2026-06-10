import { invoke } from '@tauri-apps/api/core';
import type { App, Settings, SystemColors } from './types';

export class TauriService {
    static async getAllApps(): Promise<App[]> {
        return await invoke('get_all_apps');
    }

    static async addApp(url: string, name: string): Promise<App> {
        return await invoke('add_app', { url, name });
    }

    static async updateApp(id: string, url: string, name: string): Promise<App> {
        return await invoke('update_app', { id, url, name });
    }

    static async removeApp(id: string): Promise<void> {
        await invoke('remove_app', { id });
    }

    static async checkForUpdates(appId?: string): Promise<App[]> {
        return await invoke('check_for_updates', { appId: appId || null });
    }

    static async downloadAndInstall(appId: string): Promise<string> {
        return await invoke('download_and_install', { appId });
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
}

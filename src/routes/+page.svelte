<script lang="ts">
    import {onMount, tick} from 'svelte';

    import type {App, SystemColors} from '$lib/types';
    import AddAppDialog from '$lib/components/dialogs/AddAppDialog.svelte';
    import SettingsPanel from '$lib/components/dialogs/SettingsPanel.svelte';
    import {
        ConfirmDialog,
        ErrorBanner,
        Notification,
        TitleBar,
        getSystem7WindowStyle
    } from '@lkmc/system7-ui';
    import Toolbar from '$lib/components/Toolbar.svelte';
    import AppTable from '$lib/components/AppTable.svelte';

    import {getCurrentWindow} from '@tauri-apps/api/window';
    import {listen} from '@tauri-apps/api/event';
    import {openUrl} from '@tauri-apps/plugin-opener';
    import DownloadProgressDialog from '$lib/components/dialogs/DownloadProgressDialog.svelte';
    import {windowFocused} from '$lib/util/windowState';
    import AboutDialog from '$lib/components/dialogs/AboutDialog.svelte';
    import { WindowManager } from '$lib/windowManager';

    import { appStore } from '$lib/util/appStore';
    import { getErrorMessage } from '$lib/util/errors';
    import { notifications } from '$lib/util/notifications';
    import { TauriService } from '$lib/tauri';

    type AddModal = {type: 'add'; app: App | null};
    type ConfirmRemoveModal = {type: 'confirm-remove'; appId: string; appName: string};
    type ModalState = AddModal | {type: 'settings'} | ConfirmRemoveModal | {type: 'about'} | null;

    let activeModal: ModalState = null;
    let isWindowShaded = false;
    let systemColors: SystemColors | null = null;

    interface DownloadProgress {
        appId: string;
        fileName: string;
        downloaded: number;
        total: number | null;
        done: boolean;
    }
    let downloadProgress: DownloadProgress | null = null;

    $: ({ apps, loading, error } = $appStore);

    $: windowStyle = systemColors ? getSystem7WindowStyle(systemColors) : '';

    const appWindow = getCurrentWindow();
    const windowManager = new WindowManager();

    onMount(() => {
        // Load the list, then quietly check for updates in the background
        appStore.loadApps().then(() => appStore.checkForUpdates(true));
        void loadSystemColors();

        const unlistenFocus = appWindow.onFocusChanged(({payload: focused}) => {
            windowFocused.set(focused);
        });

        const unlistenProgress = listen<DownloadProgress>('download-progress', (event) => {
            downloadProgress = event.payload.done ? null : event.payload;
        });

        const unlistenAddApp = appWindow.listen('menu-add-app', () => {
            activeModal = {type: 'add', app: null};
        });

        const unlistenCheckAll = appWindow.listen('menu-check-all', () => {
            appStore.checkForUpdates();
        });

        const unlistenSettings = appWindow.listen('menu-settings', () => {
            activeModal = {type: 'settings'};
        });

        const unlistenAbout = appWindow.listen('menu-about', () => {
            activeModal = {type: 'about'};
        });

        // Cleanup on unmount
        return () => {
            unlistenFocus.then(fn => fn());
            unlistenAddApp.then(fn => fn());
            unlistenCheckAll.then(fn => fn());
            unlistenSettings.then(fn => fn());
            unlistenAbout.then(fn => fn());
            unlistenProgress.then(fn => fn());
        };
    });

    async function loadSystemColors() {
        try {
            systemColors = await TauriService.getSystemColors();
        } catch {
            systemColors = null;
        }
    }

    function closeModal(modal: Exclude<ModalState, null>) {
        if (activeModal === modal) activeModal = null;
    }

    async function handleAddApp(modal: AddModal, url: string, name: string) {
        const success = await appStore.addApp(url, name);
        if (success) closeModal(modal);
    }

    function handleEdit(app: App) {
        activeModal = {type: 'add', app};
    }

    async function handleUpdateApp(modal: AddModal) {
        // Dialog handles the actual API call, we just need to refresh
        closeModal(modal);
        await appStore.loadApps();
    }

    async function handleRemove(id: string) {
        activeModal = {
            type: 'confirm-remove',
            appId: id,
            appName: apps.find(a => a.id === id)?.name || ''
        };
    }

    async function confirmRemove(modal: ConfirmRemoveModal) {
        if (!modal.appId) return;

        // Close the dialog regardless of outcome; the store surfaces errors
        // via notifications.
        await appStore.removeApp(modal.appId, modal.appName);
        closeModal(modal);
    }

    function cancelRemove(modal: ConfirmRemoveModal) {
        closeModal(modal);
    }



    function handleInstall(appId: string) {
        appStore.downloadAndInstall(appId);
    }

    async function handleOpenUrl(url: string) {
        try {
            await openUrl(url);
        } catch (e) {
            notifications.add(getErrorMessage(e, 'Failed to open URL'), 'error');
        }
    }

    function handleWindowClose() {
        windowManager.close();
    }

    function getAppTableHeightMetrics(): { viewportHeight: number; contentHeight: number } | null {
        const tableBodyContainer = document.querySelector<HTMLDivElement>('.s7-data-table-body-container');
        const tableBody = tableBodyContainer?.querySelector<HTMLTableElement>('table');

        if (!tableBodyContainer || !tableBody) {
            return null;
        }

        return {
            viewportHeight: tableBodyContainer.clientHeight,
            contentHeight: tableBody.getBoundingClientRect().height
        };
    }

    async function handleWindowResizeToFit() {
        // "Fit to Content" (Zoom): grow or shrink the window by the difference
        // between the table's content height and its visible viewport.
        if (isWindowShaded) {
            return;
        }

        await tick();

        const tableHeightMetrics = getAppTableHeightMetrics();
        if (!tableHeightMetrics) {
            return;
        }

        const heightDelta = Math.round(tableHeightMetrics.contentHeight - tableHeightMetrics.viewportHeight);
        await windowManager.resizeHeightBy(heightDelta);
    }

    async function handleWindowShade() {
        isWindowShaded = await windowManager.toggleShade();
    }

    function handleWindowDrag() {
        windowManager.startDragging();
    }
</script>

<div class="window-frame s7-root" class:window-unfocused={!$windowFocused} style={windowStyle}>
    <TitleBar
            title="Obtainintosh"
            focused={$windowFocused}
            closable
            collapsible
            shadeable
            draggable
            onclose={handleWindowClose}
            oncollapse={handleWindowResizeToFit}
            onshade={handleWindowShade}
            ondragstart={handleWindowDrag}
    />

    {#if !isWindowShaded}
        <!-- Notifications -->
        <Notification notifications={$notifications} ondismiss={(id) => notifications.remove(id)} />

        <!-- Main Content -->
        <main class="app-content">
            <Toolbar
                {loading}
                onaddApp={() => activeModal = {type: 'add', app: null}}
                oncheckAll={() => appStore.checkForUpdates()}
                onsettings={() => activeModal = {type: 'settings'}}
            />

            {#if error}
                <ErrorBanner message={error} onclose={() => appStore.clearError()} />
            {/if}

            <AppTable
                {apps}
                {loading}
                onopenurl={handleOpenUrl}
                oninstall={handleInstall}
                onedit={handleEdit}
                onremove={handleRemove}
            />
        </main>
    {/if}

    <!-- Dialogs live inside the .s7-root frame so they inherit the System 7
         font scope and the OS accent variables applied to the frame. -->
    {#if activeModal?.type === 'add'}
        {@const modal = activeModal}
        <AddAppDialog
                app={modal.app}
                onclose={() => closeModal(modal)}
                onadd={(e) => handleAddApp(modal, e.url, e.name)}
                onupdate={() => handleUpdateApp(modal)}
        />
    {:else if activeModal?.type === 'settings'}
        {@const modal = activeModal}
        <SettingsPanel onclose={() => closeModal(modal)}/>
    {:else if activeModal?.type === 'confirm-remove'}
        {@const modal = activeModal}
        <ConfirmDialog
            message="Are you sure you want to remove {modal.appName}?"
            okText="Remove"
            onconfirm={() => confirmRemove(modal)}
            oncancel={() => cancelRemove(modal)}
        />
    {:else if activeModal?.type === 'about'}
        {@const modal = activeModal}
        <AboutDialog onClose={() => closeModal(modal)} />
    {/if}

    {#if downloadProgress}
        <DownloadProgressDialog
            fileName={downloadProgress.fileName}
            downloaded={downloadProgress.downloaded}
            total={downloadProgress.total}
        />
    {/if}
</div>

<style>
    .window-frame {
        /* Monochrome System 7 defaults; the OS accent colors fetched at
           runtime override these via the inline style. */
        --system7-color-accent: #000;
        --system7-color-accent-text: #fff;
        --system7-color-highlight: #000;
        --system7-color-highlight-text: #fff;
        --system7-color-success: #000;
        --system7-color-error: #000;
        --system7-color-info: #000;
        /* Height must be explicit here for the main window */
        width: 100vw;
        height: 100vh;
        background: #fff;
        border: 1px solid #000;
        box-shadow: 2px 2px 0 rgba(0, 0, 0, 0.2);
        display: flex;
        flex-direction: column;
    }

    .window-frame :global(.notification.success),
    .window-frame :global(.notification.error),
    .window-frame :global(.notification.info) {
        border-left: 2px solid var(--system7-color-ink, #000);
    }

    .app-content {
        flex: 1;
        display: flex;
        flex-direction: column;
        background: #fff;
        overflow: hidden;
        min-height: 0; /* Critical for nested flex scrolling */
    }
</style>

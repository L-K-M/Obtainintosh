<script lang="ts">
    import {onMount} from 'svelte';

    import type {App} from '$lib/types';
    import AddAppDialog from '$lib/components/dialogs/AddAppDialog.svelte';
    import SettingsPanel from '$lib/components/dialogs/SettingsPanel.svelte';
    import TitleBar from '$lib/components/widgets/TitleBar.svelte';
    import ConfirmDialog from '$lib/components/widgets/ConfirmDialog.svelte';
    import Notification from '$lib/components/widgets/Notification.svelte';
    import Toolbar from '$lib/components/Toolbar.svelte';
    import ErrorBanner from '$lib/components/widgets/ErrorBanner.svelte';
    import AppTable from '$lib/components/AppTable.svelte';

    import {getCurrentWindow} from '@tauri-apps/api/window';
    import {openUrl} from '@tauri-apps/plugin-opener';
    import {windowFocused} from '$lib/util/windowState';
    import AboutDialog from '$lib/components/dialogs/AboutDialog.svelte';
    import { WindowManager } from '$lib/windowManager';

    import { appStore } from '$lib/util/appStore';
    import { notifications } from '$lib/util/notifications';

    let showAddDialog = false;
    let showSettings = false;
    let editingApp: App | null = null;
    let isWindowShaded = false;
    let clickedApplicationName = '';
    let showConfirm = false;
    let confirmAppId: string | null = null;
    let showAboutDialog = false;

    $: ({ apps, loading, error } = $appStore);


    const appWindow = getCurrentWindow();
    const windowManager = new WindowManager();

    onMount(() => {
        appStore.loadApps();

        appWindow.onFocusChanged(({payload: focused}) => {
            windowFocused.set(focused);
        });

        const unlistenAddApp = appWindow.listen('menu-add-app', () => {
            editingApp = null;
            showAddDialog = true;
        });

        const unlistenCheckAll = appWindow.listen('menu-check-all', () => {
            appStore.checkForUpdates();
        });

        const unlistenSettings = appWindow.listen('menu-settings', () => {
            showSettings = true;
        });

        const unlistenAbout = appWindow.listen('menu-about', () => {
            showAboutDialog = true;
        });

        // Cleanup on unmount
        return () => {
            unlistenAddApp.then(fn => fn());
            unlistenCheckAll.then(fn => fn());
            unlistenSettings.then(fn => fn());
            unlistenAbout.then(fn => fn());
        };
    });

    async function handleAddApp(url: string, name: string) {
        const success = await appStore.addApp(url, name);
        if (success) {
            showAddDialog = false;
            editingApp = null;
        }
    }

    function handleEdit(app: App) {
        editingApp = app;
        showAddDialog = true;
    }

    async function handleUpdateApp() {
        // Dialog handles the actual API call, we just need to refresh
        showAddDialog = false;
        editingApp = null;
        await appStore.loadApps();
    }

    async function handleRemove(id: string) {
        confirmAppId = id;
        clickedApplicationName = apps.find(a => a.id === id)?.name || '';
        showConfirm = true;
    }

    async function confirmRemove() {
        if (!confirmAppId) return;

        const success = await appStore.removeApp(confirmAppId, clickedApplicationName);
        if (success || true) {
            showConfirm = false;
            confirmAppId = null;
        }
    }

    function cancelRemove() {
        showConfirm = false;
        confirmAppId = null;
    }



    function handleInstall(appId: string) {
        appStore.downloadAndInstall(appId);
    }

    async function handleOpenUrl(url: string) {
        try {
            await openUrl(url);
        } catch (e) {
            // console.error('Failed to open URL:', e);
            notifications.add('Failed to open URL', 'error');
        }
    }

    function handleWindowClose() {
        windowManager.close();
    }

    async function handleWindowMinimize() {
        // "Fit to Content" (Zoom)
        try {
            const currentSize = await appWindow.innerSize();
            const scaleFactor = await appWindow.scaleFactor();
            const currentWidth = currentSize.width / scaleFactor;

            // Measure visible components
            const titleBarH = document.querySelector('.title-bar')?.getBoundingClientRect().height || 35;
            const toolbarH = document.querySelector('.toolbar')?.getBoundingClientRect().height || 0;
            const errorH = document.querySelector('.error-banner')?.getBoundingClientRect().height || 0;
            
            // AppTable internals
            const tableHeaderH = document.querySelector('.table-header-container')?.getBoundingClientRect().height || 0;
            const tableBodyH = document.querySelector('.table-body-container table')?.getBoundingClientRect().height || 0;
            
            // Add some padding/borders safety (window frame border top/bottom + internal spacing)
            // Table body container has 1px border top. Window frame has 1px border.
            // Let's add a small buffer or just sum exacts.
            const totalHeight = titleBarH + toolbarH + errorH + tableHeaderH + tableBodyH + 5; // +5 for borders/padding safety

            await windowManager.setSize(currentWidth, totalHeight);
        } catch (e) {
            console.error('Failed to resize window:', e);
        }
    }

    async function handleWindowShade() {
        isWindowShaded = await windowManager.toggleShade();
    }

    function handleWindowDrag() {
        windowManager.startDragging();
    }
</script>

<div class="window-frame" class:window-unfocused={!$windowFocused}>
    <TitleBar
            title="Obtainintosh"
            closable
            collapsible
            shadeable
            draggable
            onclose={handleWindowClose}
            oncollapse={handleWindowMinimize}
            onshade={handleWindowShade}
            ondragstart={handleWindowDrag}
    />

    {#if !isWindowShaded}
        <!-- Notifications -->
        <Notification notifications={$notifications} />

        <!-- Main Content -->
        <main class="app-content">
            <Toolbar
                {loading}
                onaddApp={() => { editingApp = null; showAddDialog = true; }}
                oncheckAll={() => appStore.checkForUpdates()}
                onsettings={() => showSettings = true}
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
</div>

{#if showAddDialog}
    <AddAppDialog
            app={editingApp}
            onclose={() => { showAddDialog = false; editingApp = null; }}
            onadd={(e) => handleAddApp(e.url, e.name)}
            onupdate={handleUpdateApp}
    />
{/if}

{#if showSettings}
    <SettingsPanel onclose={() => showSettings = false}/>
{/if}

{#if showConfirm}
    <ConfirmDialog
        message="Are you sure you want to remove {clickedApplicationName}?"
        okText="Remove"
        onconfirm={confirmRemove}
        oncancel={cancelRemove}
    />
{/if}

{#if showAboutDialog}
    <AboutDialog onClose={() => showAboutDialog = false} />
{/if}

<style>
    .window-frame {
        /* Height must be explicit here for the main window */
        width: 100vw;
        height: 100vh;
        background: #fff;
        border: 1px solid #000;
        box-shadow: 2px 2px 0 rgba(0, 0, 0, 0.2);
        display: flex;
        flex-direction: column;
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

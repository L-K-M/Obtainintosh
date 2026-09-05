<script lang="ts">
    import { Button } from '@lkmc/system7-ui';


    export let loading = false;
    /** Whether there is anything to export; the button is disabled otherwise. */
    export let hasApps = false;
    export let onaddApp: (() => void) | undefined = undefined;
    export let oncheckAll: (() => void) | undefined = undefined;
    export let onimport: (() => void) | undefined = undefined;
    export let onexport: (() => void) | undefined = undefined;
    export let onsettings: (() => void) | undefined = undefined;
    export let onabout: (() => void) | undefined = undefined;
</script>

<div class="toolbar">
    <div class="toolbar-group">
        <Button onclick={onaddApp} disabled={loading}>
            Add Program...
        </Button>
        <Button onclick={oncheckAll} disabled={loading}>
            {#if loading}
                Checking...
            {:else}
                Check All
            {/if}
        </Button>
        <Button onclick={onimport} disabled={loading}>
            Import...
        </Button>
        <Button onclick={onexport} disabled={loading || !hasApps}>
            Export...
        </Button>
    </div>
    <div class="toolbar-group">
        <Button onclick={onsettings}>
            Settings
        </Button>
        <Button onclick={onabout}>
            About
        </Button>
    </div>
</div>

<style>
    .toolbar {
        display: flex;
        justify-content: space-between;
        /* Six buttons fit one row at the default window width but not at
           the minimum, where the Settings/About group wraps onto a second
           row instead of overflowing. */
        flex-wrap: wrap;
        gap: 8px;
        padding: 8px 12px;
        border-bottom: 1px solid #000;
        flex-shrink: 0;
    }

    .toolbar-group {
        display: flex;
        flex-wrap: wrap;
        gap: 8px;
    }

    /* Keeps the right-hand group at the right edge when it wraps; with
       space-between alone a lone group on the second row would sit left. */
    .toolbar-group:last-child {
        margin-left: auto;
    }
</style>

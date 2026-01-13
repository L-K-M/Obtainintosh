<script lang="ts">
    import Button from '$lib/components/widgets/Button.svelte';


    export let loading = false;
    export let onaddApp: (() => void) | undefined = undefined;
    export let oncheckAll: (() => void) | undefined = undefined;
    export let onsettings: (() => void) | undefined = undefined;
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
    </div>
    <div class="toolbar-group">
        <Button onclick={onsettings}>
            Settings
        </Button>
    </div>
</div>

<style>
    .toolbar {
        display: flex;
        justify-content: space-between;
        padding: 8px;
        border-bottom: 1px solid #000;
        flex-shrink: 0;
    }

    .toolbar-group {
        display: flex;
        gap: 8px;
        padding: 0 10px;
    }
</style>


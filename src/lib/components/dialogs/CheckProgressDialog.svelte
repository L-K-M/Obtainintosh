<script lang="ts">
    import { ModalDialog, ProgressBar } from '@lkmc/system7-ui';

    // Modal progress for a batch update check, driven by 'check-progress'
    // events from the Rust backend. All props are null until the first event
    // arrives; the bar shows the barber pole in that window. Deliberately not
    // dismissable (no onclose): it closes when the check finishes.
    export let position: number | null = null;
    export let total: number | null = null;
    export let appName: string | null = null;

    // Events fire before each app is checked, so `position - 1` apps are
    // actually finished — the bar must not count the in-flight one.
    $: completed = position === null ? 0 : position - 1;
</script>

<ModalDialog width="380px">
    <div class="check-dialog">
        <div class="check-line">
            <span class="check-label">Checking for updates:</span>
            <span class="check-value">{appName ?? '…'}</span>
        </div>
        <ProgressBar
            value={completed}
            max={total ?? 1}
            indeterminate={position === null || total === null}
            height={12}
            ariaLabel="Update check progress"
        />
        <div class="check-line count">
            {#if position !== null && total !== null}
                {position} of {total}
            {:else}
                &nbsp;
            {/if}
        </div>
    </div>
</ModalDialog>

<style>
    .check-dialog {
        padding: 10px 8px 6px;
    }

    .check-line {
        display: flex;
        gap: 6px;
        margin-bottom: 10px;
        white-space: nowrap;
        overflow: hidden;
    }

    .check-label {
        flex-shrink: 0;
    }

    .check-value {
        overflow: hidden;
        text-overflow: ellipsis;
        font-weight: bold;
    }

    .check-line.count {
        margin-bottom: 0;
        margin-top: 8px;
        justify-content: flex-end;
    }
</style>

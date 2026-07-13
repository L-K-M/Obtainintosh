<script lang="ts">
    import { ModalDialog, ProgressBar } from '@lkmc/system7-ui';

    // A System 7 Finder-style file copy progress dialog.
    // Driven by 'download-progress' events emitted from the Rust backend.
    // Deliberately not dismissable (no onclose): downloads are not
    // cancellable yet, and the dialog closes on the final done event.
    export let fileName: string;
    export let downloaded: number;
    export let total: number | null = null;

    function formatBytes(bytes: number): string {
        if (bytes < 1024) return `${bytes} B`;
        if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
        return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    }
</script>

<ModalDialog width="420px">
    <div class="copy-dialog">
        <div class="copy-line">
            <span class="copy-label">Downloading:</span>
            <span class="copy-value">{fileName}</span>
        </div>
        <ProgressBar
            value={downloaded}
            max={total || 1}
            indeterminate={!total}
            height={12}
            ariaLabel="Download progress for {fileName}"
        />
        <div class="copy-line bytes">
            {#if total}
                {formatBytes(downloaded)} of {formatBytes(total)}
            {:else}
                {formatBytes(downloaded)} so far
            {/if}
        </div>
    </div>
</ModalDialog>

<style>
    .copy-dialog {
        padding: 10px 8px 6px;
    }

    .copy-line {
        display: flex;
        gap: 6px;
        margin-bottom: 10px;
        white-space: nowrap;
        overflow: hidden;
    }

    .copy-label {
        flex-shrink: 0;
    }

    .copy-value {
        overflow: hidden;
        text-overflow: ellipsis;
        font-weight: bold;
    }

    .copy-line.bytes {
        margin-bottom: 0;
        margin-top: 8px;
        justify-content: flex-end;
    }
</style>

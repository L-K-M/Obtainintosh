<script lang="ts">
    import { ProgressBar } from '@lkmc/system7-ui';

    // A System 7 Finder-style file copy progress dialog.
    // Driven by 'download-progress' events emitted from the Rust backend.
    export let fileName: string;
    export let downloaded: number;
    export let total: number | null = null;

    function formatBytes(bytes: number): string {
        if (bytes < 1024) return `${bytes} B`;
        if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
        return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    }
</script>

<div class="overlay">
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
</div>

<style>
    .overlay {
        position: fixed;
        inset: 0;
        display: flex;
        align-items: flex-start;
        justify-content: center;
        padding: 90px 16px 16px;
        box-sizing: border-box;
        z-index: 1000;
        pointer-events: none;
    }

    .copy-dialog {
        pointer-events: auto;
        background: #fff;
        border: 2px solid #000;
        outline: 1px solid #fff;
        box-shadow: 2px 2px 0 rgba(0, 0, 0, 0.5);
        padding: 14px 18px 12px;
        box-sizing: border-box;
        width: 100%;
        max-width: 460px;
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

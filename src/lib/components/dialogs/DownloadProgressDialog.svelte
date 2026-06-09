<script lang="ts">
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

    $: percent = total ? Math.min(100, (downloaded / total) * 100) : null;
</script>

<div class="overlay">
    <div class="copy-dialog">
        <div class="copy-line">
            <span class="copy-label">Downloading:</span>
            <span class="copy-value">{fileName}</span>
        </div>
        <div class="progress-track">
            {#if percent !== null}
                <div class="progress-fill" style="width: {percent}%"></div>
            {:else}
                <div class="progress-fill indeterminate"></div>
            {/if}
        </div>
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
        padding-top: 90px;
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
        min-width: 340px;
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

    .progress-track {
        border: 1px solid #000;
        height: 12px;
        background: #fff;
        overflow: hidden;
    }

    .progress-fill {
        height: 100%;
        background: #000;
        transition: width 0.15s linear;
    }

    /* Barber-pole stripes while the total size is unknown */
    .progress-fill.indeterminate {
        width: 100%;
        background: repeating-linear-gradient(
            -45deg,
            #000 0,
            #000 6px,
            #fff 6px,
            #fff 12px
        );
        animation: barber 0.6s linear infinite;
    }

    @keyframes barber {
        from { background-position: 0 0; }
        to { background-position: 17px 0; }
    }

    .copy-line.bytes {
        margin-bottom: 0;
        margin-top: 8px;
        justify-content: flex-end;
    }
</style>

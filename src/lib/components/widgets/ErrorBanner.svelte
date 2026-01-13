<script lang="ts">
    export let message: string;
    export let onclose: (() => void) | undefined = undefined;
</script>

<div class="error-banner">
    <span class="error-icon">⚠️</span>
    {message}
    <button class="close-error" onclick={onclose}>×</button>
</div>

<style>
    .error-banner {
        background: #000;
        color: #fff;
        padding: 8px;
        margin: 8px 8px 0;
        display: flex;
        align-items: center;
        gap: 8px;
    }

    .close-error {
        margin-left: auto;
        background: none;
        border: 1px solid #fff;
        color: #fff;
        cursor: pointer;
    }
</style>


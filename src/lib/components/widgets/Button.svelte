<script lang="ts">
    export let variant = 'default'; // 'default', 'primary', 'small', 'icon'
    export let disabled = false;
    export let type: 'button' | 'submit' | 'reset' = 'button';
    export let title = '';
    export let onclick: ((e: MouseEvent) => void) | undefined = undefined;
</script>


{#if variant === 'primary'}
    <div
            class="primary-border"
            class:disabled={disabled}
    >
        <button
                class="sys7-btn"
                {disabled}
                {title}
                {type}
                {onclick}
        >
            <slot/>
        </button>
    </div>
{:else}
    <button
            class="sys7-btn"
            class:icon-btn={variant === 'icon'}
            {disabled}
            {title}
            {type}
            {onclick}
    >
        <slot/>
    </button>
{/if}

<style>
    .primary-border {
        display: inline-block;
        border: 3px solid #000;
        border-radius: 10px;
        padding: 2px;
        margin-top: -4px;
        margin-bottom: -4px;
    }

    .primary-border.disabled {
        border-color: #808080;
    }

    .sys7-btn {
        background: #fff;
        border: 1.5px solid #000;
        padding: 4px 20px 3px;
        font-family: inherit;
        cursor: pointer;
        border-radius: 6px;
        transition: none;
        position: relative;
    }

    .sys7-btn:active:not(:disabled) {
        transform: translate(1px, 1px);
        background: #000;
        color: #fff;
    }

    .sys7-btn:disabled {
        color: #808080;
        cursor: default;
    }

    /* Icon buttons - no border, just the icon */
    .icon-btn {
        border: none !important;
        background: transparent !important;
        padding: 2px !important;
        min-width: 20px;
        cursor: pointer;
    }

    .icon-btn:hover {
        opacity: 0.7;
    }

    .icon-btn:active {
        opacity: 0.5;
        transform: none !important;
    }

    .icon-btn :global(img) {
        width: 16px;
        height: 16px;
        display: block;
    }
</style>

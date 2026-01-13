<script lang="ts">

    import bombIcon from '$lib/assets/bomb-icon.png';
    import Button from '$lib/components/widgets/Button.svelte';
    import ModalDialog from '$lib/components/widgets/ModalDialog.svelte';

    export let message = '';
    export let okText = 'OK';
    export let cancelText = 'Cancel';
    export let onconfirm: (() => void) | undefined = undefined;
    export let oncancel: (() => void) | undefined = undefined;

    function handleOk() {
        if (onconfirm) onconfirm();
    }

    function handleCancel() {
        if (oncancel) oncancel();
    }
</script>

<ModalDialog width="400px" onclose={handleCancel}>
    <div class="confirm-content">
        <img src={bombIcon} alt="Bomb" class="bomb-icon"/>
        <p class="message dialog-text ">{message}</p>
    </div>
    <div class="button-row">
        <Button onclick={handleCancel}>{cancelText}</Button>
        <Button variant="primary" onclick={handleOk}>{okText}</Button>
    </div>
</ModalDialog>

<style>
    .confirm-content {
        display: flex;
        gap: 16px;
        margin-bottom: 32px;
        align-items: flex-start;
    }

    .bomb-icon {
        flex-shrink: 0;
        width: 48px;
        height: 48px;
        margin-top: -7px;
    }

    .message {
        flex: 1;
        min-height: 32px;
        margin: 0;
        line-height: 1.3;
    }

    .button-row {
        display: flex;
        gap: 8px;
        justify-content: flex-end;
        margin: -10px;
        margin-right: -14px;
    }
</style>

<script lang="ts">
    import {onMount} from 'svelte';
    import {TauriService} from '$lib/tauri';
    import type {Settings} from '$lib/types';
    import {getErrorMessage} from '$lib/util/errors';
    import {BalloonHelp, Button, MovableDialog, TextInput} from '@lkmc/system7-ui';
    import {openUrl} from '@tauri-apps/plugin-opener';

    export let onclose: (() => void) | undefined = undefined;

    let settings: Settings = {github_token: null, gitlab_token: null};
    let initialLoading = true;
    let loading = false;
    let error: string | null = null;
    let success = false;
    let destroyed = false;
    let closeTimer: ReturnType<typeof setTimeout> | undefined;

    onMount(() => {
        void (async () => {
            try {
                const loadedSettings = await TauriService.getSettings();
                if (!destroyed) settings = loadedSettings;
            } catch (e) {
                if (!destroyed) error = getErrorMessage(e, 'Failed to load settings');
            } finally {
                if (!destroyed) initialLoading = false;
            }
        })();

        return () => {
            destroyed = true;
            if (closeTimer !== undefined) clearTimeout(closeTimer);
        };
    });

    function close() {
        if (onclose) onclose();
    }

    async function handleSave(event: Event) {
        event.preventDefault();
        if (initialLoading || loading || success) return;

        try {
            loading = true;
            error = null;
            success = false;
            await TauriService.updateSettings(settings);
            if (destroyed) return;
            success = true;
            closeTimer = setTimeout(() => {
                close();
            }, 1000);
        } catch (e) {
            if (!destroyed) error = getErrorMessage(e, 'Failed to save settings');
        } finally {
            if (!destroyed) loading = false;
        }
    }
</script>

<MovableDialog title="Settings" onclose={close}>
    <form on:submit={handleSave} aria-busy={initialLoading || loading}>
        <div class="s7-form-group">
            <label for="github-token">GitHub Personal Access Token</label>
            <TextInput
                    id="github-token"
                    type="password"
                    clearable
                    disabled={initialLoading || loading || success}
                    placeholder="ghp_xxxxxxxxxxxx (optional)"
                    value={settings.github_token ?? ''}
                    oninput={(value) => (settings.github_token = value || null)}
                    onclear={() => (settings.github_token = null)}
            />
            <div class="hint">
                <BalloonHelp message="Generate a fine-grained token with read-only access to public repositories">
                    Optional. Increases API rate limits.
                </BalloonHelp>
                <button type="button" class="link" on:click={() => openUrl('https://github.com/settings/tokens')}>Generate token →</button>
            </div>
        </div>
        <!--
        <div class="form-group">
            <label for="gitlab-token">GitLab Personal Access Token</label>
            <input
                    id="gitlab-token"
                    type="password"
                    placeholder="glpat-xxxxxxxxxxxx (optional)"
                    bind:value={settings.gitlab_token}
            />
            <div class="hint">
                Optional. For GitLab repositories.
                <button type="button" class="link" on:click={() => openUrl('https://gitlab.com/-/user_settings/personal_access_tokens')}>Generate token →</button>
            </div>
        </div>
        -->
        {#if error}
            <div class="error">{error}</div>
        {/if}

        {#if success}
            <div class="success">✓ Settings saved successfully!</div>
        {/if}

        <div class="actions">
            <Button type="button" onclick={close} disabled={loading}>
                Cancel
            </Button>
            <Button type="submit" variant="primary" disabled={initialLoading || loading || success}>
                Save
            </Button>
        </div>
    </form>
</MovableDialog>

<style>
    /* Local overrides only - most styles are now global */
    .s7-form-group :global(.sys7-text-input) {
        flex: 1;
        width: 100%;
    }

    .hint {
        color: #000;
        display: flex;
        align-items: baseline;
        gap: 4px;
    }

    .link {
        background: none;
        border: none;
        padding: 0;
        font: inherit;
        color: #000;
        text-decoration: underline;
        cursor: pointer;
        display: inline;
        float: left;
        font-size: 18px;
    }

    .link:hover {
        text-decoration: none;
    }

    .error {
        color: #000;
        font-style: italic;
        margin-bottom: 16px;
    }

    .success {
        color: #000;
        font-weight: bold;
        margin-bottom: 16px;
    }
</style>

<script lang="ts">
    import {onMount} from 'svelte';
    import {TauriService} from '$lib/tauri';
    import type {Settings} from '$lib/types';
    import {BalloonHelp, Button, MovableDialog} from '@lkmc/system7-ui';
    import {openUrl} from '@tauri-apps/plugin-opener';

    export let onclose: (() => void) | undefined = undefined;

    let settings: Settings = {github_token: null, gitlab_token: null};
    let loading = false;
    let error: string | null = null;
    let success = false;

    onMount(async () => {
        try {
            settings = await TauriService.getSettings();
        } catch (e) {
            error = 'Failed to load settings';
        }
    });

    function close() {
        if (onclose) onclose();
    }

    async function handleSave(event: Event) {
        event.preventDefault();

        try {
            loading = true;
            error = null;
            success = false;
            await TauriService.updateSettings(settings);
            success = true;
            setTimeout(() => {
                close();
            }, 1000);
        } catch (e) {
            error = e instanceof Error ? e.message : 'Failed to save settings';
        } finally {
            loading = false;
        }
    }
</script>

<MovableDialog title="Settings" onclose={close}>
    <form on:submit={handleSave}>
        <div class="form-group">
            <label for="github-token">GitHub Personal Access Token</label>
            <input
                    id="github-token"
                    type="password"
                    placeholder="ghp_xxxxxxxxxxxx (optional)"
                    bind:value={settings.github_token}
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
            <Button type="submit" variant="primary" disabled={loading}>
                Save
            </Button>
        </div>
    </form>
</MovableDialog>

<style>
    /* Local overrides only - most styles are now global */
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

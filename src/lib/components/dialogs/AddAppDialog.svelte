<script lang="ts">
  import { TauriService } from '$lib/tauri';
  import type { App } from '$lib/types';
  import { getErrorMessage } from '$lib/util/errors';
  import { Button, MovableDialog, TextInput } from '@lkmc/system7-ui';
  

  export let app: App | null = null;
  export let onclose: (() => void) | undefined = undefined;
  export let onadd: ((e: {url: string, name: string}) => void | Promise<void>) | undefined = undefined;
  export let onupdate: (() => void) | undefined = undefined;

  let url = app ? app.source_url : '';
  let name = app ? app.name : '';
  let loading = false;
  let error: string | null = null;

  // Tracks the last auto-derived name so we keep updating it as the URL is
  // typed, but stop as soon as the user edits the name themselves.
  let autoFilledName = '';

  $: isEdit = !!app;

  function deriveNameFromUrl(url: string): string | null {
    const match = url.trim().match(/(?:github|gitlab)\.com\/[^/]+\/([^/?#]+)/i);
    if (!match) return null;
    const repoName = match[1].replace(/\.git$/, '');
    return repoName || null;
  }

  function handleUrlChange() {
    if (isEdit) return;
    if (name && name !== autoFilledName) return;

    const derived = deriveNameFromUrl(url);
    if (derived) {
      name = derived;
      autoFilledName = derived;
    }
  }
  
  function close() {
    if (onclose) onclose();
  }

  function handleInputKeydown(event: KeyboardEvent) {
    if (event.key === 'Enter' && url && name && !loading) {
      void handleSubmit(event);
    }
  }

  async function handleSubmit(event: Event) {
    event.preventDefault();

    if (!url.trim()) {
      error = 'Please enter a GitHub or GitLab URL';
      return;
    }

    if (!name.trim()) {
      error = 'Please enter a program name';
      return;
    }

    try {
      loading = true;
      error = null;

      if (isEdit && app) {
        // Update app
        await TauriService.updateApp(app.id, url.trim(), name.trim());
        if (onupdate) onupdate();
      } else {
        // Add app — await it, otherwise a failed add leaves the dialog
        // open with the button stuck in the disabled/loading state
        if (onadd) await onadd({ url: url.trim(), name: name.trim() });
      }
    } catch (e) {
      error = getErrorMessage(e, 'Failed to save program');
    } finally {
      loading = false;
    }
  }
</script>

<MovableDialog title={app ? 'Edit Program' : 'Add Program'} onclose={close}>
  <div class="s7-form-group" class:has-error={!!error}>
    <label for="url">GitHub URL</label>
    <TextInput
      id="url"
      bind:value={url}
      clearable
      placeholder="https://github.com/owner/repo"
      oninput={handleUrlChange}
      onkeydown={handleInputKeydown}
    />
    {#if error}
      <span class="s7-error-msg">{error}</span>
    {/if}
  </div>

  <div class="s7-form-group">
    <label for="name">Program Name</label>
    <TextInput
      id="name"
      bind:value={name}
      clearable
      placeholder="Program Name"
      onkeydown={handleInputKeydown}
    />
  </div>

  <div class="actions">
    <Button onclick={close}>Cancel</Button>
    <Button variant="primary" onclick={handleSubmit} disabled={!url || !name || loading}>
      {app ? 'Update' : 'Add'}
    </Button>
  </div>
</MovableDialog>

<style>
  .s7-form-group :global(.sys7-text-input) {
    flex: 1;
    width: 100%;
  }

  .s7-form-group.has-error :global(.sys7-text-input) {
    border-width: 2px;
  }

  .actions {
    display: flex;
    gap: 12px;
    justify-content: flex-end;
  }
</style>

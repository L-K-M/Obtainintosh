<script lang="ts">
  import { TauriService } from '$lib/tauri';
  import type { App } from '$lib/types';
  import { Button, MovableDialog } from '@lkmc/system7-ui';
  

  export let app: App | null = null;
  export let onclose: (() => void) | undefined = undefined;
  export let onadd: ((e: {url: string, name: string}) => void | Promise<void>) | undefined = undefined;
  export let onupdate: (() => void) | undefined = undefined;

  let url = app ? app.source_url : '';
  let name = app ? app.name : '';
  let loading = false;
  let error: string | null = null;

  $: isEdit = !!app;

  function detectAppName(url: string) {
    try {
      const parts = url.trim().split('/');
      const repoName = parts[parts.length - 1] || parts[parts.length - 2];
      name = repoName.replace(/\.git$/, '');
    } catch (e) {
      // Ignore errors
    }
  }

  function handleUrlChange() {
    if (url && !name && !isEdit) {
      detectAppName(url);
    }
  }
  
  function close() {
    if (onclose) onclose();
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
      error = e instanceof Error ? e.message : 'Failed to save program';
    } finally {
      loading = false;
    }
  }
</script>

<MovableDialog title={app ? 'Edit Program' : 'Add Program'} onclose={close}>
  <div class="form-group">
    <label for="url">GitHub URL</label>
    <input 
      type="text" 
      id="url" 
      bind:value={url} 
      placeholder="https://github.com/owner/repo"
      on:input={handleUrlChange}
      class:error={!!error}
    />
    {#if error}
      <span class="error-msg">{error}</span>
    {/if}
  </div>

  <div class="form-group">
    <label for="name">Program Name</label>
    <input 
      type="text" 
      id="name" 
      bind:value={name} 
      placeholder="Program Name"
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
  input {
    display: flex;
    gap: 12px;
    justify-content: flex-end;
  }
</style>

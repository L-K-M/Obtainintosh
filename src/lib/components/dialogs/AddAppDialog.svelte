<script lang="ts">
  import { TauriService } from '$lib/tauri';
  import type { App, SourceInput, SourceType } from '$lib/types';
  import { getErrorMessage } from '$lib/util/errors';
  import { BalloonHelp, Button, Dropdown, MovableDialog, TextInput } from '@lkmc/system7-ui';


  export let app: App | null = null;
  export let onclose: (() => void) | undefined = undefined;
  export let onadd: ((input: SourceInput) => void | Promise<void>) | undefined = undefined;
  export let onupdate: (() => void) | undefined = undefined;

  /** `auto` leaves the forge for the backend to detect from the URL. */
  type SourceChoice = 'auto' | SourceType;

  let url = app ? app.source_url : '';
  let name = app ? app.name : '';
  let sourceChoice: SourceChoice = app ? app.source_type : 'auto';
  let username = app?.username ?? '';
  let accessToken = app?.access_token ?? '';
  let loading = false;
  let error: string | null = null;

  // Tracks the last auto-derived name so we keep updating it as the URL is
  // typed, but stop as soon as the user edits the name themselves.
  let autoFilledName = '';
  let appIdentity = app?.id ?? null;

  $: if ((app?.id ?? null) !== appIdentity) {
    appIdentity = app?.id ?? null;
    url = app?.source_url ?? '';
    name = app?.name ?? '';
    sourceChoice = app ? app.source_type : 'auto';
    username = app?.username ?? '';
    accessToken = app?.access_token ?? '';
    autoFilledName = '';
    error = null;
  }

  $: isEdit = !!app;

  // GitLab is not offered as a choice — it has no adapter yet — but an app
  // already stored as GitLab keeps the option so editing its name can't
  // silently retype it.
  $: sourceOptions = [
    { value: 'auto', label: 'Detect automatically' },
    { value: 'github', label: 'GitHub' },
    { value: 'forgejo', label: 'Forgejo' },
    ...(app?.source_type === 'gitlab' ? [{ value: 'gitlab', label: 'GitLab' }] : [])
  ];

  // Forgejo is self-hosted, so a private instance needs credentials that no
  // other source type uses.
  $: needsCredentials = sourceChoice === 'forgejo';

  function deriveNameFromUrl(url: string): string | null {
    // Forgejo lives on arbitrary hosts, so match the <owner>/<repo> shape
    // rather than a known forge domain.
    const match = url
      .trim()
      .match(/^(?:https?:\/\/)?[^/\s?#]+\/[^/\s?#]+\/([^/\s?#]+)/i);
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

  function collectInput(): SourceInput {
    return {
      url: url.trim(),
      name: name.trim(),
      sourceType: sourceChoice === 'auto' ? null : sourceChoice,
      // Credentials belong to Forgejo only: switching the source type away
      // from it drops them rather than storing keys nothing will send.
      username: needsCredentials ? username.trim() || null : null,
      accessToken: needsCredentials ? accessToken.trim() || null : null
    };
  }

  async function handleSubmit(event: Event) {
    event.preventDefault();

    if (!url.trim()) {
      error = 'Please enter a repository URL';
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
        await TauriService.updateApp(app.id, collectInput());
        if (onupdate) onupdate();
      } else {
        // Add app — await it, otherwise a failed add leaves the dialog
        // open with the button stuck in the disabled/loading state
        if (onadd) await onadd(collectInput());
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
    <label for="url">Repository URL</label>
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

  <div class="s7-form-group source-group">
    <label for="source-type">Source</label>
    <BalloonHelp message="A Forgejo instance can be on any domain, so pick Forgejo when its address doesn't say so">
      <Dropdown
        id="source-type"
        options={sourceOptions}
        value={sourceChoice}
        disabled={loading}
        onchange={(value) => (sourceChoice = value as SourceChoice)}
      />
    </BalloonHelp>
  </div>

  {#if needsCredentials}
    <div class="s7-form-group">
      <label for="forgejo-username">Forgejo Username</label>
      <TextInput
        id="forgejo-username"
        bind:value={username}
        clearable
        placeholder="Username (private instances)"
        onkeydown={handleInputKeydown}
      />
    </div>

    <div class="s7-form-group">
      <label for="forgejo-key">Application Key</label>
      <TextInput
        id="forgejo-key"
        type="password"
        bind:value={accessToken}
        clearable
        placeholder="Application key (private instances)"
        onkeydown={handleInputKeydown}
      />
      <div class="hint">
        <BalloonHelp message="On your Forgejo instance: Settings → Applications → Generate New Token, with read access to the repository">
          Leave both blank for public repositories.
        </BalloonHelp>
      </div>
    </div>
  {/if}

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

  /* The balloon wrapper is inline-flex by default, which would keep the
     dropdown at its own minimum width instead of matching the text inputs. */
  .source-group :global(.balloon-container) {
    display: flex;
    width: 100%;
  }

  .source-group :global(.sys7-dropdown) {
    width: 100%;
  }

  .hint {
    color: #000;
    display: flex;
    align-items: baseline;
    gap: 4px;
  }

  .actions {
    display: flex;
    gap: 12px;
    justify-content: flex-end;
  }
</style>

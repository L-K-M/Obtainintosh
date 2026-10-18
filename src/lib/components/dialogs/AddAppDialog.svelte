<script lang="ts">
  import { onDestroy } from 'svelte';
  import { TauriService } from '$lib/tauri';
  import type { App, ReleasePrograms, SourceInput, SourceType } from '$lib/types';
  import { getErrorMessage } from '$lib/util/errors';
  import { programDisplayName } from '$lib/util/programNames';
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
  let assetFilter = app?.asset_filter ?? '';
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
    assetFilter = app?.asset_filter ?? '';
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

  // The latest release's programs, looked up once the URL names a
  // repository, so that a release with several can offer them as choices.
  // `lookupKey` names the inputs the shown programs belong to: a reply for
  // inputs that have changed since is dropped.
  // Long enough that typing a URL by hand does not spend a forge API
  // request on every partial repository name.
  const PROGRAM_LOOKUP_DELAY_MS = 1000;
  let releasePrograms: ReleasePrograms | null = null;
  let lookupKey = '';
  let lookupTimer: ReturnType<typeof setTimeout> | undefined;

  $: scheduleProgramLookup(
    url,
    sourceChoice,
    needsCredentials ? username : '',
    needsCredentials ? accessToken : ''
  );

  // Offered only when there is a choice to make.
  $: offeredPrograms =
    releasePrograms && releasePrograms.programs.length > 1 ? releasePrograms.programs : [];
  $: chosenProgram = offeredPrograms.includes(assetFilter.trim().toLowerCase())
    ? assetFilter.trim().toLowerCase()
    : '';
  $: programOptions = [
    { value: '', label: 'Choose…' },
    ...offeredPrograms.map(program => ({ value: program, label: program }))
  ];

  function scheduleProgramLookup(...inputs: string[]) {
    const key = JSON.stringify(inputs.map(input => input.trim()));
    if (key === lookupKey) return;
    lookupKey = key;
    releasePrograms = null;
    clearTimeout(lookupTimer);
    if (!deriveNameFromUrl(url)) return;

    lookupTimer = setTimeout(() => void lookUpPrograms(key), PROGRAM_LOOKUP_DELAY_MS);
  }

  async function lookUpPrograms(key: string) {
    try {
      const programs = await TauriService.listReleasePrograms(collectInput());
      if (key === lookupKey) releasePrograms = programs;
    } catch {
      // Offering programs is a convenience. A repository the lookup cannot
      // read is reported by the update check, with its reason.
    }
  }

  onDestroy(() => clearTimeout(lookupTimer));

  function chooseProgram(program: string) {
    assetFilter = program;
    if (isEdit || !program) return;

    // Installed programs are found by name, so a name still derived from the
    // repository follows the chosen program.
    if (!name || name === autoFilledName) {
      name = programDisplayName(program);
      autoFilledName = name;
    }
  }

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
      accessToken: needsCredentials ? accessToken.trim() || null : null,
      assetFilter: assetFilter.trim() || null
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

  <div class="s7-form-group">
    <label for="asset-filter">Program in Release</label>
    <TextInput
      id="asset-filter"
      bind:value={assetFilter}
      clearable
      placeholder="Optional"
      onkeydown={handleInputKeydown}
    />
    {#if releasePrograms && offeredPrograms.length > 0}
      <div class="hint program-choice">
        <label for="release-program">Version {releasePrograms.version} has several programs:</label>
        <Dropdown
          id="release-program"
          options={programOptions}
          value={chosenProgram}
          disabled={loading}
          onchange={(program) => chooseProgram(program)}
        />
      </div>
      {#if !assetFilter.trim() && releasePrograms.defaultProgram}
        <div class="hint">Without a choice, Obtainintosh downloads {releasePrograms.defaultProgram}.</div>
      {/if}
    {:else}
      <div class="hint">
        <BalloonHelp message="Some repositories publish several programs in each release. Enter the one to track as its file names begin (seance for seance-macos-universal.zip), or a pattern such as *-gtk4-*">
          Only for releases with several programs.
        </BalloonHelp>
      </div>
    {/if}
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

  .program-choice {
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
  }

  .actions {
    display: flex;
    gap: 12px;
    justify-content: flex-end;
  }
</style>

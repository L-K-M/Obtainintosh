<script lang="ts">
  import { Button, ModalDialog } from '@lkmc/system7-ui';
  import aboutImg from '$lib/assets/about.png';
  import { TauriService } from '$lib/tauri';
  import { getErrorMessage } from '$lib/util/errors';
  export let onClose: () => void;

  // The bundled code's license texts, loaded when the user asks for them.
  let notices: string | null = null;
  let noticesError: string | null = null;

  async function showLicenses() {
    noticesError = null;
    try {
      notices = await TauriService.getThirdPartyNotices();
    } catch (error) {
      noticesError = getErrorMessage(error, 'The third-party notices could not be loaded.');
    }
  }
</script>

<ModalDialog onclose={onClose} width={notices === null ? '400px' : '560px'}>
  {#if notices === null}
    <img src={aboutImg} alt="About Obtainintosh" class="about-img" />
    <p class="license">
      Obtainintosh's own code is released under the MIT License. It bundles third-party
      components that keep their own licenses.
    </p>
    {#if noticesError}
      <p class="error" role="alert">{noticesError}</p>
    {/if}
    <div class="actions">
      <Button onclick={showLicenses}>Licenses</Button>
    </div>
  {:else}
    <pre class="notices" aria-label="Third-party notices">{notices}</pre>
    <div class="actions">
      <Button onclick={() => (notices = null)}>Back</Button>
    </div>
  {/if}
</ModalDialog>

<style>
  .about-img {
    width:100%;
  }

  .license {
    margin: 8px 0;
  }

  .error {
    font-style: italic;
  }

  .notices {
    height: 360px;
    margin: 0 0 8px;
    overflow: auto;
    font-size: 11px;
    white-space: pre-wrap;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
  }
</style>

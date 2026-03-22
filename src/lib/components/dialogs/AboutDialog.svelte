<script lang="ts">
  import { ModalDialog } from '@lkmc/system7-ui';
  import aboutImg from '$lib/assets/about.png';
  export let onClose: () => void;
</script>

<ModalDialog onclose={onClose}>
  <img src={aboutImg} alt="About Obtainintosh" class="about-img" />
</ModalDialog>

<style>
  .about-img {
    width:100%;
  }
</style>

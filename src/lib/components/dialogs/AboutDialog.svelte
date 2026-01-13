<script lang="ts">
  import ModalDialog from '$lib/components/widgets/ModalDialog.svelte';
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

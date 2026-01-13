<script lang="ts">
  import { onDestroy } from 'svelte';
  import TitleBar from './TitleBar.svelte';


  export let title: string;
  export let width = '460px';
  export let onclose: (() => void) | undefined = undefined;

  let dialogElement: HTMLDivElement;
  let isDragging = false;
  let isCollapsed = false;
  let dragOffset = { x: 0, y: 0 };
  let position = { x: 0, y: 0 };
  let initialized = false;

  function close() {
    if (onclose) onclose();
  }

  function toggleCollapse() {
    // Initialize position before collapsing to prevent jumping
    if (!initialized && dialogElement) {
      const rect = dialogElement.getBoundingClientRect();
      position.x = rect.left;
      position.y = rect.top;
      initialized = true;
    }
    isCollapsed = !isCollapsed;
  }

  function handleTitleBarDragStart(e: MouseEvent) {
    handleDragStart(e);
  }

  function handleDragStart(event: MouseEvent) {
    if (!dialogElement) return;

    isDragging = true;
    const rect = dialogElement.getBoundingClientRect();
    dragOffset.x = event.clientX - rect.left;
    dragOffset.y = event.clientY - rect.top;

    // Initialize position if not yet set
    if (!initialized) {
      position.x = rect.left;
      position.y = rect.top;
      initialized = true;
    }

    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', handleMouseUp);
  }

  function handleMouseMove(event: MouseEvent) {
    if (!isDragging || !dialogElement) return;

    const rect = dialogElement.getBoundingClientRect();
    const dialogWidth = rect.width;
    const dialogHeight = rect.height;

    // Calculate new position
    let newX = event.clientX - dragOffset.x;
    let newY = event.clientY - dragOffset.y;

    // Constrain to viewport boundaries
    const minX = 0;
    const minY = 0;
    const maxX = window.innerWidth - dialogWidth;
    const maxY = window.innerHeight - dialogHeight;

    position.x = Math.max(minX, Math.min(newX, maxX));
    position.y = Math.max(minY, Math.min(newY, maxY));
  }

  function handleMouseUp() {
    isDragging = false;
    document.removeEventListener('mousemove', handleMouseMove);
    document.removeEventListener('mouseup', handleMouseUp);
  }

  onDestroy(() => {
    document.removeEventListener('mousemove', handleMouseMove);
    document.removeEventListener('mouseup', handleMouseUp);
  });
</script>

<!-- svelte-ignore a11y-click-events-have-key-events, a11y-no-static-element-interactions -->
<div class="backdrop" on:click={close}>
  <div 
    bind:this={dialogElement}
    class="dialog"
    class:dragging={isDragging}
    style="width: {width}; {initialized ? `position: fixed; left: ${position.x}px; top: ${position.y}px; transform: none;` : ''}"
    on:click|stopPropagation
  >
    <TitleBar {title} closable shadeable draggable onclose={close} onshade={toggleCollapse} ondragstart={handleTitleBarDragStart} />

    {#if !isCollapsed}
      <div class="modal-content">
        <slot />
      </div>
    {/if}
  </div>
</div>

<style>
  .modal-content {
    padding: 16px;
    display: flex;
    flex-direction: column;
  }

  .dialog.dragging {
    user-select: none;
  }
</style>

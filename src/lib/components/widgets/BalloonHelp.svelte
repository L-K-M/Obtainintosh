<script lang="ts">
    export let message = '';
    export let position = 'bottom'; // bottom by default, can be top, left, right
    export let delay = 1000; // delay in milliseconds before showing balloon

    let showBalloon = false;
    let timeoutId: ReturnType<typeof setTimeout> | null = null;
    let containerElement: HTMLDivElement;
    let balloonElement: HTMLDivElement;
    let adjustedPosition = position;
    let horizontalOffset = 0;

    // Pointer SVG path
    let pointerFillPath = '';
    let pointerLeftPath = '';
    let pointerRightPath = '';

    function handleMouseEnter() {
        timeoutId = setTimeout(() => {
            showBalloon = true;
            // Wait for next tick to measure balloon position
            requestAnimationFrame(() => {
                adjustBalloonPosition();
                updatePointerPath();
            });
        }, delay);
    }

    function handleMouseLeave() {
        if (timeoutId) {
            clearTimeout(timeoutId);
            timeoutId = null;
        }
        showBalloon = false;
        horizontalOffset = 0;
        adjustedPosition = position;
    }

    function adjustBalloonPosition() {
        if (!balloonElement || !containerElement) return;

        const balloonRect = balloonElement.getBoundingClientRect();
        const padding = 25; // Padding from window edges

        // Reset adjustments
        adjustedPosition = position;
        horizontalOffset = 0;

        // Check vertical overflow
        if (position === 'bottom' && balloonRect.bottom > window.innerHeight - padding) {
            adjustedPosition = 'top';
        } else if (position === 'top' && balloonRect.top < padding) {
            adjustedPosition = 'bottom';
        }

        // Recalculate balloon rect after position change
        requestAnimationFrame(() => {
            if (!balloonElement) return;
            const newBalloonRect = balloonElement.getBoundingClientRect();

            // Check horizontal overflow (left)
            if (newBalloonRect.left < padding) {
                horizontalOffset = padding - newBalloonRect.left;
            }
            // Check horizontal overflow (right)
            else if (newBalloonRect.right > window.innerWidth - padding) {
                horizontalOffset = (window.innerWidth - padding) - newBalloonRect.right;
            }

            updatePointerPath();
        });
    }

    function updatePointerPath() {
        if (!balloonElement || !containerElement) return;

        const containerRect = containerElement.getBoundingClientRect();
        const balloonRect = balloonElement.getBoundingClientRect();

        // Anchor point: center of container element
        const anchorX = containerRect.left + containerRect.width / 2 - containerRect.left;
        let anchorY: number;
        if (adjustedPosition === 'bottom') {
            anchorY = containerRect.height; // bottom center
        } else {
            anchorY = 0; // top center
        }

        // Balloon connection point: center of balloon edge
        let connectionX = balloonRect.left + balloonRect.width / 2 - containerRect.left;
        let connectionY: number;
        if (adjustedPosition === 'bottom') {
            connectionY = balloonRect.top - containerRect.top + 2; // top edge of balloon
        } else {
            connectionY = balloonRect.top - containerRect.top + balloonRect.height - 2; // bottom edge of balloon
        }

        // Pointer width at balloon connection
        const pointerWidth = 8;
        let horizontalOffset = -30;
        if(balloonRect.left < 200) {
            horizontalOffset = 30;
        }
        const leftX = connectionX - pointerWidth + horizontalOffset;
        const rightX = connectionX + pointerWidth + horizontalOffset;

        // Filled pointer path (closed, for fill)
        pointerFillPath = `
            M ${anchorX} ${anchorY}
            L ${leftX} ${connectionY}
            L ${rightX} ${connectionY}
            L ${anchorX} ${anchorY}
            Z
        `;
        // Left side path
        pointerLeftPath = `M ${anchorX} ${anchorY} L ${leftX} ${connectionY}`;
        // Right side path
        pointerRightPath = `M ${anchorX} ${anchorY} L ${rightX} ${connectionY}`;
    }
</script>

<!-- svelte-ignore a11y-no-static-element-interactions -->
<div
        class="balloon-container"
        bind:this={containerElement}
        on:mouseenter={handleMouseEnter}
        on:mouseleave={handleMouseLeave}
>
    <slot/>
    {#if showBalloon && message}
        <svg class="pointer-svg" aria-hidden="true">
            <path d={pointerFillPath} fill="white" stroke="none"/>
            <path d={pointerLeftPath} fill="none" stroke="black" stroke-width="2"/>
            <path d={pointerRightPath} fill="none" stroke="black" stroke-width="2"/>
        </svg>
        <div
                class="balloon {adjustedPosition}"
                bind:this={balloonElement}
                style="transform: translateX(calc(-50% + {horizontalOffset}px));"
        >
            <div class="balloon-content">{message}</div>
        </div>
    {/if}
</div>

<style>
    .balloon-container {
        position: relative;
        display: inline-flex;
        align-items: center;
    }

    .pointer-svg {
        position: absolute;
        top: 0;
        left: 0;
        width: 100%;
        height: 100%;
        overflow: visible;
        z-index: 10001;
        pointer-events: none;
    }

    .balloon {
        position: absolute;
        left: 50%;
        transform: translateX(-50%);
        white-space: nowrap;

        z-index: 10000;
        pointer-events: none;
        animation: fadeIn 0.2s ease-in;

        border-radius: 10px;
        border: 2px solid black;

        padding: 15px;
        background-color: white;
        box-shadow: 2px 2px 0 rgba(0, 0, 0, 1);
    }

    .balloon.bottom {
        top: calc(100% + 25px);
    }

    .balloon.top {
        bottom: calc(100% + 25px);
    }

    @keyframes fadeIn {
        from {
            opacity: 0;
        }
        to {
            opacity: 1;
        }
    }
</style>

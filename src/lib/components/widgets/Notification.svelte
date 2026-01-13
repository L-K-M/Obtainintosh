<script lang="ts">
    export let notifications: {id: number, message: string, type: 'success' | 'error' | 'info'}[] = [];
    // Helper to convert newlines to <br>
    function formatMessage(message: string) {
        return message.replace(/\n/g, '<br>');
    }
</script>

{#each notifications as notification (notification.id)}
    <div class="notification notification-{notification.type}" style="bottom: {20 + notifications.indexOf(notification) * 70}px;">
        <div class="notification-content">
            {@html formatMessage(notification.message)}
        </div>
    </div>
{/each}

<style>
    .notification {
        position: fixed;
        right: 20px;
        display: flex;
        align-items: center;
        gap: 8px;
        max-width: 300px;

        z-index: 10000;
        pointer-events: none;
        animation: fadeIn 0.2s ease-in;

        border-radius: 10px;
        border: 2px solid black;

        padding: 15px;
        background-color: white;
        box-shadow: 2px 2px 0 rgba(0, 0, 0, 1);
    }
    
    .notification.success {
        border-left: 4px solid #4CAF50;
    }
    
    .notification.error {
        border-left: 4px solid #F44336;
    }
    
    .notification.info {
        border-left: 4px solid #2196F3;
    }
    
    .notification-content {
        flex: 1;
        overflow-wrap: break-word;
        hyphens: auto;
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
import { writable } from 'svelte/store';

export type NotificationType = 'success' | 'error' | 'info';

export interface Notification {
    id: number;
    message: string;
    type: NotificationType;
}

function createNotificationStore() {
    const { subscribe, update } = writable<Notification[]>([]);
    let counter = 0;

    return {
        subscribe,
        add: (message: string, type: NotificationType = 'info') => {
            const id = counter++;
            update(n => [...n, { id, message, type }]);

            // Auto-remove after 5 seconds
            setTimeout(() => {
                update(n => n.filter(item => item.id !== id));
            }, 5000);
        },
        remove: (id: number) => {
            update(n => n.filter(item => item.id !== id));
        }
    };
}

export const notifications = createNotificationStore();

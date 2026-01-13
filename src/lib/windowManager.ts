import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';

export class WindowManager {
    private static readonly TITLE_BAR_HEIGHT = 36;
    private savedWindowSize: { width: number; height: number } | null = null;
    private isShaded = false;
    private appWindow = getCurrentWindow();

    async close(): Promise<void> {
        try {
            await this.appWindow.close();
        } catch (e) {
            console.error('Failed to close window:', e);
        }
    }

    async minimize(): Promise<void> {
        try {
            await this.appWindow.minimize();
        } catch (e) {
            console.error('Failed to minimize window:', e);
        }
    }

    async setSize(width: number, height: number): Promise<void> {
        try {
            const size = new LogicalSize(width, height);
            await this.appWindow.setSize(size);
        } catch (e) {
            console.error('Failed to set window size:', e);
        }
    }

    async toggleShade(): Promise<boolean> {
        try {
            const scaleFactor = await this.appWindow.scaleFactor();

            if (!this.isShaded) {
                // Shade
                const size = await this.appWindow.innerSize();
                const logicalWidth = size.width / scaleFactor;
                const logicalHeight = size.height / scaleFactor;

                this.savedWindowSize = { width: logicalWidth, height: logicalHeight };

                await this.appWindow.setSize(new LogicalSize(logicalWidth, WindowManager.TITLE_BAR_HEIGHT));
                this.isShaded = true;
            } else {
                // Unshade
                if (this.savedWindowSize) {
                    await this.appWindow.setSize(
                        new LogicalSize(this.savedWindowSize.width, this.savedWindowSize.height)
                    );
                }
                this.isShaded = false;
            }
            return this.isShaded;
        } catch (e) {
            console.error('Failed to toggle window shade:', e);
            return this.isShaded; // Return current state on error
        }
    }

    async startDragging(): Promise<void> {
        try {
            await this.appWindow.startDragging();
        } catch (e) {
            console.error('Failed to start dragging:', e);
        }
    }

    get isWindowShaded(): boolean {
        return this.isShaded;
    }
}

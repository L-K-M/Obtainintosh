import { currentMonitor, getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';

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

    async resizeHeightBy(deltaHeight: number): Promise<void> {
        if (!Number.isFinite(deltaHeight)) {
            return;
        }

        try {
            const [scaleFactor, innerSize, outerSize, outerPosition, monitor] = await Promise.all([
                this.appWindow.scaleFactor(),
                this.appWindow.innerSize(),
                this.appWindow.outerSize(),
                this.appWindow.outerPosition(),
                currentMonitor()
            ]);

            const logicalInnerWidth = innerSize.width / scaleFactor;
            const logicalInnerHeight = innerSize.height / scaleFactor;
            const logicalOuterHeight = outerSize.height / scaleFactor;
            const chromeHeight = Math.max(0, logicalOuterHeight - logicalInnerHeight);

            const requestedInnerHeight = logicalInnerHeight + deltaHeight;
            let maxInnerHeight = Number.POSITIVE_INFINITY;

            if (monitor) {
                const monitorBottomPx = monitor.workArea.position.y + monitor.workArea.size.height;
                const availableOuterHeightPx = monitorBottomPx - outerPosition.y;
                if (availableOuterHeightPx > 0) {
                    maxInnerHeight = Math.max(
                        WindowManager.TITLE_BAR_HEIGHT,
                        availableOuterHeightPx / scaleFactor - chromeHeight
                    );
                }
            }

            const targetInnerHeight = Math.max(
                WindowManager.TITLE_BAR_HEIGHT,
                Math.min(requestedInnerHeight, maxInnerHeight)
            );

            if (Math.abs(targetInnerHeight - logicalInnerHeight) < 0.5) {
                return;
            }

            await this.appWindow.setSize(new LogicalSize(logicalInnerWidth, targetInnerHeight));
        } catch (e) {
            console.error('Failed to resize window:', e);
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

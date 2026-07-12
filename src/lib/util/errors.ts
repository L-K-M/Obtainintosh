export function getErrorMessage(error: unknown, fallback: string): string {
    if (typeof error === 'string') {
        return error;
    }

    return error instanceof Error ? error.message : fallback;
}

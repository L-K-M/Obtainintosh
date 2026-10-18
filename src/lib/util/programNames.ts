/**
 * A program name from a release, as a Program Name: `seance-sync` becomes
 * `Seance Sync`. Installed programs are looked up by that name, so it should
 * read like the program rather than like a file name.
 */
export function programDisplayName(program: string): string {
    return program
        .split('-')
        .filter(Boolean)
        .map(word => word.charAt(0).toUpperCase() + word.slice(1))
        .join(' ');
}

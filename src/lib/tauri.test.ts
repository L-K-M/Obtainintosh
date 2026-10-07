import { expect, it, vi } from 'vitest';
import { TauriService } from './tauri';
import type { SourceInput } from './types';

const native = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: native.invoke }));

it('sends the dialog input as the one argument the program commands take', async () => {
    const input: SourceInput = {
        url: 'https://github.com/L-K-M/Hauntware',
        name: 'Seance',
        sourceType: null,
        username: null,
        accessToken: null,
        assetFilter: 'seance'
    };

    await TauriService.addApp(input);
    await TauriService.updateApp('id-1', input);
    await TauriService.listReleasePrograms(input);

    // The Rust commands name these parameters `input` and `id`.
    expect(native.invoke.mock.calls).toEqual([
        ['add_app', { input }],
        ['update_app', { id: 'id-1', input }],
        ['list_release_programs', { input }]
    ]);
});

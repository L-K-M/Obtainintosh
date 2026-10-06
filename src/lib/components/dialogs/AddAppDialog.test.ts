import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { beforeEach, expect, it, vi } from 'vitest';
import AddAppDialog from './AddAppDialog.svelte';
import type { ReleasePrograms, SourceInput } from '$lib/types';

const native = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: native.invoke }));

const HAUNTWARE = 'https://github.com/L-K-M/Hauntware';
const HAUNTWARE_PROGRAMS: ReleasePrograms = {
    version: '1.9.0',
    programs: ['planchette', 'poltergeist', 'seance', 'seance-sync'],
    defaultProgram: 'planchette'
};

beforeEach(() => {
    native.invoke.mockReset();
});

function field(container: HTMLElement, id: string): HTMLInputElement {
    return container.querySelector(`#${id}`) as HTMLInputElement;
}

async function enterUrl(container: HTMLElement, url: string) {
    await fireEvent.input(field(container, 'url'), { target: { value: url } });
}

function addButton(container: HTMLElement): HTMLButtonElement {
    const buttons = Array.from(container.querySelectorAll('button'));
    return buttons.find(button => button.textContent?.trim() === 'Add') as HTMLButtonElement;
}

it('offers the programs of a multi-program release and names the entry after the choice', async () => {
    native.invoke.mockImplementation(async (command: string) =>
        command === 'list_release_programs' ? HAUNTWARE_PROGRAMS : null
    );
    const onadd = vi.fn<(input: SourceInput) => void>();
    const { container, getByText } = render(AddAppDialog, { props: { onadd } });

    await enterUrl(container, HAUNTWARE);
    const choice = await waitFor(() => {
        const select = container.querySelector('#release-program') as HTMLSelectElement;
        expect(select).not.toBeNull();
        return select;
    });
    expect(getByText('Without a choice, Obtainintosh downloads planchette.')).toBeTruthy();
    expect(native.invoke).toHaveBeenCalledWith('list_release_programs', {
        input: expect.objectContaining({ url: HAUNTWARE, sourceType: null })
    });

    await fireEvent.change(choice, { target: { value: 'seance-sync' } });
    expect(field(container, 'asset-filter').value).toBe('seance-sync');
    expect(field(container, 'name').value).toBe('Seance Sync');

    await fireEvent.click(addButton(container));
    expect(onadd).toHaveBeenCalledWith({
        url: HAUNTWARE,
        name: 'Seance Sync',
        sourceType: null,
        username: null,
        accessToken: null,
        assetFilter: 'seance-sync'
    });
});

it('keeps a typed program when the release lookup fails', async () => {
    native.invoke.mockImplementation(async (command: string) => {
        if (command === 'list_release_programs') throw new Error('rate limited');
        return null;
    });
    const onadd = vi.fn<(input: SourceInput) => void>();
    const { container } = render(AddAppDialog, { props: { onadd } });

    await enterUrl(container, HAUNTWARE);
    await waitFor(() =>
        expect(native.invoke).toHaveBeenCalledWith('list_release_programs', expect.anything())
    );
    await fireEvent.input(field(container, 'asset-filter'), { target: { value: ' Seance ' } });
    await fireEvent.click(addButton(container));

    expect(container.querySelector('#release-program')).toBeNull();
    expect(onadd).toHaveBeenCalledWith(
        expect.objectContaining({ name: 'Hauntware', assetFilter: 'Seance' })
    );
});

it('offers no choice for a release with a single program', async () => {
    native.invoke.mockImplementation(async (command: string) =>
        command === 'list_release_programs'
            ? { version: '2.0.0', programs: ['tool'], defaultProgram: 'tool' }
            : null
    );
    const { container } = render(AddAppDialog, { props: {} });

    await enterUrl(container, 'https://github.com/owner/tool');
    await waitFor(() =>
        expect(native.invoke).toHaveBeenCalledWith('list_release_programs', expect.anything())
    );

    expect(container.querySelector('#release-program')).toBeNull();
    expect(field(container, 'name').value).toBe('tool');
});

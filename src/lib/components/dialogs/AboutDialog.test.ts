import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, expect, it, vi } from 'vitest';
import AboutDialog from './AboutDialog.svelte';

const native = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: native.invoke }));

const NOTICES = 'Third-party notices for Obtainintosh\n\nsvelte 5.57.1\nLicense: MIT\n';

beforeEach(() => {
    native.invoke.mockReset();
});

it('states the own license and shows the bundled notices on request', async () => {
    native.invoke.mockResolvedValue(NOTICES);
    const { container, getByText } = render(AboutDialog, { props: { onClose: vi.fn() } });

    expect(getByText(/own code is released under the MIT License/)).toBeTruthy();
    expect(native.invoke).not.toHaveBeenCalled();

    await fireEvent.click(getByText('Licenses'));
    await tick();

    expect(native.invoke).toHaveBeenCalledWith('get_third_party_notices');
    expect(container.querySelector('pre.notices')?.textContent).toBe(NOTICES);

    await fireEvent.click(getByText('Back'));
    expect(container.querySelector('pre.notices')).toBeNull();
});

it('reports notices that cannot be loaded', async () => {
    native.invoke.mockRejectedValue('This build does not include its third-party notices.');
    const { container, getByText } = render(AboutDialog, { props: { onClose: vi.fn() } });

    await fireEvent.click(getByText('Licenses'));
    await tick();

    expect(getByText('This build does not include its third-party notices.')).toBeTruthy();
    expect(container.querySelector('pre.notices')).toBeNull();
});

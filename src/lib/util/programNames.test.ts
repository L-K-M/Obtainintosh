import { expect, it } from 'vitest';
import { programDisplayName } from './programNames';

it('turns a release program name into a program name', () => {
    expect(programDisplayName('seance')).toBe('Seance');
    expect(programDisplayName('seance-sync')).toBe('Seance Sync');
    expect(programDisplayName('mac-mouse-fix')).toBe('Mac Mouse Fix');
    expect(programDisplayName('7zip')).toBe('7zip');
    expect(programDisplayName('séance')).toBe('Séance');
});

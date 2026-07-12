<script lang="ts">
    import type { App } from '$lib/types';
    import AppRow from './AppRow.svelte';
    import semver from 'semver';
    import { DataTable } from '@lkmc/system7-ui';


    export let apps: App[] = [];
    export let loading = false;
    export let onopenurl: ((url: string) => void) | undefined = undefined;
    export let oninstall: ((id: string) => void) | undefined = undefined;
    export let onedit: ((app: App) => void) | undefined = undefined;
    export let onremove: ((id: string) => void) | undefined = undefined;

    type SortField = 'name' | 'current' | 'latest' | 'status';
    type SortDirection = 'asc' | 'desc';

    const columns = [
        { key: 'name', label: 'Name', width: '35%', sortable: true, className: 'col-name' },
        { key: 'current', label: 'Current', width: '15%', sortable: true, className: 'col-version' },
        { key: 'latest', label: 'Latest', width: '15%', sortable: true, className: 'col-version' },
        { key: 'status', label: 'Status', width: '20%', sortable: true, className: 'col-status' },
        { key: 'actions', label: 'Actions', width: '15%', className: 'col-actions' }
    ];

    let sortField: SortField = 'name';
    let sortDirection: SortDirection = 'asc';

    const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });

    function cleanVersion(v: string): string {
        return v.replace(/^v/, '').trim();
    }

    function hasUpdate(app: App): boolean {
        if (!app.current_version || !app.latest_version) return false;

        const current = cleanVersion(app.current_version);
        const latest = cleanVersion(app.latest_version);

        // Try semver first
        if (semver.valid(current) && semver.valid(latest)) {
            return semver.gt(latest, current);
        }

        // Try coercing to semver
        try {
            const currentCoerced = semver.coerce(current);
            const latestCoerced = semver.coerce(latest);
            if (currentCoerced && latestCoerced) {
                return semver.gt(latestCoerced, currentCoerced);
            }
        } catch (e) {
            // Fall through to string comparison
        }

        // Fallback for non-semver versions: numeric-aware compare so "10.0"
        // is newer than "9.0" (plain string compare would say otherwise)
        return collator.compare(latest, current) > 0;
    }

    $: sortedApps = [...apps].sort((a, b) => {
        const multiplier = sortDirection === 'asc' ? 1 : -1;
        const result = compareApps(a, b, sortField) * multiplier;
        if (result !== 0) {
            return result;
        }
        return collator.compare(a.name, b.name);
    });

    function compareApps(a: App, b: App, field: SortField): number {
        if (field === 'name') {
            return collator.compare(a.name, b.name);
        }

        if (field === 'current') {
            return compareVersionStrings(a.current_version || '', b.current_version || '');
        }

        if (field === 'latest') {
            return compareVersionStrings(a.latest_version || '', b.latest_version || '');
        }

        return compareStatus(a, b);
    }

    function compareStatus(a: App, b: App): number {
        return statusRank(a) - statusRank(b);
    }

    function statusRank(app: App): number {
        if (hasUpdate(app)) {
            return 0;
        }
        if (!app.latest_version) {
            return 1;
        }
        if (app.current_version) {
            return 2;
        }
        return 3;
    }

    function compareVersionStrings(leftRaw: string, rightRaw: string): number {
        const left = cleanVersion(leftRaw);
        const right = cleanVersion(rightRaw);

        if (!left && !right) {
            return 0;
        }
        if (!left) {
            return -1;
        }
        if (!right) {
            return 1;
        }

        if (semver.valid(left) && semver.valid(right)) {
            return semver.compare(left, right);
        }

        const leftCoerced = semver.coerce(left);
        const rightCoerced = semver.coerce(right);
        if (leftCoerced && rightCoerced) {
            return semver.compare(leftCoerced, rightCoerced);
        }

        return collator.compare(left, right);
    }

    function setSort(field: string) {
        if (field !== 'name' && field !== 'current' && field !== 'latest' && field !== 'status') {
            return;
        }

        if (sortField === field) {
            sortDirection = sortDirection === 'asc' ? 'desc' : 'asc';
            return;
        }

        sortField = field;
        sortDirection = 'asc';
    }
</script>

<div class="app-table">
    <DataTable
        class="data-view"
        columns={columns}
        sortKey={sortField}
        sortDirection={sortDirection}
        onSort={setSort}
        loading={loading && apps.length === 0}
        empty={!loading && apps.length === 0}
        emptyText="No apps tracked. Click 'Add Program...' to start."
        emptyColspan={5}
    >
        {#each sortedApps as app (app.id)}
            <AppRow
                {app}
                hasUpdate={hasUpdate(app)}
                onopenurl={onopenurl}
                oninstall={oninstall}
                onedit={onedit}
                onremove={onremove}
            />
        {/each}
    </DataTable>
</div>

<style>
    .app-table {
        flex: 1;
        min-height: 0;
        display: flex;
    }

    .app-table :global(.data-view) {
        background: #fff;
        width: 100%;
    }

    .app-table :global(.data-view th.col-name) {
        padding-left: 49px;
    }
</style>

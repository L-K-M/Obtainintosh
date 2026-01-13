<script lang="ts">
    import type { App } from '$lib/types';
    import AppRow from './AppRow.svelte';
    import semver from 'semver';


    export let apps: App[] = [];
    export let loading = false;
    export let onopenurl: ((url: string) => void) | undefined = undefined;
    export let oninstall: ((id: string) => void) | undefined = undefined;
    export let onedit: ((app: App) => void) | undefined = undefined;
    export let onremove: ((id: string) => void) | undefined = undefined;

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

        // Fallback for non-semver versions
        return latest > current;
    }
</script>

<div class="data-view">
    <div class="table-header-container">
        <table>
            <thead>
            <tr>
                <th class="col-name">Name</th>
                <th class="col-version">Current</th>
                <th class="col-version">Latest</th>
                <th class="col-status">Status</th>
                <th class="col-actions">Actions</th>
            </tr>
            </thead>
        </table>
    </div>
    <div class="table-body-container">
        <table>
            <tbody>
            {#if loading && apps.length === 0}
                <tr>
                    <td colspan="5" class="loading-state">Loading...</td>
                </tr>
            {:else if apps.length === 0}
                <tr>
                    <td colspan="5" class="empty-state">No apps tracked. Click "Add Program..." to start.</td>
                </tr>
            {:else}
                {#each apps as app}
                    <AppRow
                        {app}
                        hasUpdate={hasUpdate(app)}
                        onopenurl={onopenurl}
                        oninstall={oninstall}
                        onedit={onedit}
                        onremove={onremove}
                    />
                {/each}
            {/if}
            </tbody>
        </table>
    </div>
</div>

<style>
    .data-view {
        flex: 1;
        display: flex;
        flex-direction: column;
        overflow: hidden;
        background: #fff;
        min-height: 0;
    }

    .table-header-container {
        padding-right: 16px;
        border-bottom: 1px solid #000;
    }

    .table-body-container {
        flex: 1;
        overflow-y: scroll;
        overflow-x: hidden;
        min-height: 0;
        margin-top: 2px;
        border-top: 1px solid #000;
    }

    table {
        width: 100%;
        border-collapse: collapse;
        table-layout: fixed;
    }

    th {
        padding: 4px 8px;
        text-align: left;
        font-weight: normal;
        background: #fff;
    }

    th.col-name {
        text-decoration: underline;
        padding-left: 49px;
    }

    th:last-child {
        border-right: none;
    }

    .col-name {
        width: 35%;
    }

    .col-version {
        width: 15%;
    }

    .col-status {
        width: 20%;
    }

    .col-actions {
        width: 15%;
        text-align: right;
    }

    .loading-state, .empty-state {
        padding: 20px;
        text-align: center;
        color: #666;
        font-style: italic;
    }

    :global(tr:last-child td) {
        border-bottom: none;
    }
</style>


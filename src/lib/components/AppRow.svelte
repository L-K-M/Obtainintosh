<script lang="ts">
    import type { App } from '$lib/types';
    import { BalloonHelp, Button, DownloadIcon, EditIcon, TrashIcon } from '@lkmc/system7-ui';
    import alertIcon from '$lib/assets/alert-icon.png';



    export let app: App;
    export let hasUpdate: boolean;
    export let onopenurl: ((url: string) => void) | undefined = undefined;
    export let oninstall: ((id: string) => void) | undefined = undefined;
    export let onedit: ((app: App) => void) | undefined = undefined;
    export let onremove: ((id: string) => void) | undefined = undefined;

    $: iconUrl = getIconUrl(app);
    $: failedCheckHelp = getFailedCheckHelp(app);

    function getFailedCheckHelp(app: App): string {
        const message = app.last_check_attempt?.message || 'The update check did not complete.';
        if (app.latest_version) {
            return `Last update check failed: ${message} The latest version shown is from an earlier successful check.`;
        }
        return `Last update check failed: ${message}`;
    }

    function getIconUrl(app: App): string | null {
        if (app.source_type === 'github') {
            try {
                const url = new URL(app.source_url);
                const parts = url.pathname.split('/').filter(Boolean);
                if (parts.length >= 1) {
                    const owner = parts[0];
                    return `https://avatars.githubusercontent.com/${owner}?size=40`;
                }
            } catch (e) {
                console.error('Invalid URL for icon:', e);
            }
        }
        return null;
    }
</script>

<!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
<tr class:has-update={hasUpdate}>
    <td class="col-name clickable" on:dblclick={() => { if(onopenurl) onopenurl(app.source_url); }}>
        <BalloonHelp message="Double-click to open the GitHub page">
            <div class="name-cell">
                {#if iconUrl}
                    <img
                        src={iconUrl}
                        alt=""
                        class="app-icon-img"
                        on:error={(e) => {
                            const img = e.target;
                            if (img instanceof HTMLImageElement) {
                                img.style.display = 'none';
                                img.nextElementSibling?.classList.remove('hidden');
                            }
                        }}
                    />
                    <span class="app-icon fallback-icon hidden">📄</span>
                {:else}
                    <span class="app-icon">📄</span>
                {/if}
                <span class="app-name">{app.name}</span>
            </div>
        </BalloonHelp>
    </td>
    <td class="col-version">
        {app.current_version || '-'}
    </td>
    <td class="col-version">
        {app.latest_version || '-'}
    </td>
    <td class="col-status">
        {#if app.last_check_attempt?.state === 'unsupported'}
            <BalloonHelp message={app.last_check_attempt.message || 'This source cannot be checked for updates.'}>
                <span class="status-badge missing">
                    <img src={alertIcon} alt="Alert" class="alert-icon"/>
                    Unsupported
                </span>
            </BalloonHelp>
        {:else if app.last_check_attempt?.state === 'failed'}
            <BalloonHelp message={failedCheckHelp}>
                <span class="status-badge missing">
                    <img src={alertIcon} alt="Alert" class="alert-icon"/>
                    {app.latest_version ? 'Check Failed (Stale)' : 'Check Failed'}
                </span>
            </BalloonHelp>
        {:else if hasUpdate}
            <span class="status-badge update">Update Available</span>
        {:else if !app.last_checked}
            <BalloonHelp message="This program hasn't been checked for updates yet.">
                <span class="status-badge not-checked">Not Checked</span>
            </BalloonHelp>
        {:else if !app.latest_version}
            <BalloonHelp message="No Mac version for this application was found.">
                <span class="status-badge missing">
                    <img src={alertIcon} alt="Alert" class="alert-icon"/>
                    Not Found
                </span>
            </BalloonHelp>
        {:else if app.current_version}
            <span class="status-badge ok">Up to Date</span>
        {:else}
            <span class="status-badge not-installed">Not Installed</span>
        {/if}
    </td>
    <td class="col-actions">
        <div class="action-buttons">
            {#if hasUpdate}
                <BalloonHelp message="Download and save the update for this program">
                    <Button variant="icon" onclick={() => { if(oninstall) oninstall(app.id); }}>
                        <DownloadIcon alt="Update"/>
                    </Button>
                </BalloonHelp>
            {:else if !app.current_version}
                <BalloonHelp message="Download this program">
                    <Button variant="icon" onclick={() => { if(oninstall) oninstall(app.id); }}>
                        <DownloadIcon alt="Download"/>
                    </Button>
                </BalloonHelp>
            {/if}
            <BalloonHelp message="Edit this program's name and URL">
                <Button variant="icon" onclick={() => { if(onedit) onedit(app); }}>
                    <EditIcon alt="Edit"/>
                </Button>
            </BalloonHelp>
            <BalloonHelp message="Delete program from list - if it's installed, it will remain on your system">
                <Button
                    variant="icon"
                    onclick={() => { if(onremove) onremove(app.id); }}
                >
                    <TrashIcon alt="Remove"/>
                </Button>
            </BalloonHelp>
        </div>
    </td>
</tr>

<style>
    td {
        padding: 4px 8px;
        vertical-align: middle;
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

    .name-cell {
        display: flex;
        align-items: center;
        gap: 8px;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        padding-left: 13px;
    }

    .name-cell span.app-name {
        padding: 2px 4px;
    }

    .action-buttons {
        display: flex;
        justify-content: flex-end;
        gap: 4px;
    }

    .app-icon {
        display: flex;
        align-items: center;
    }

    .fallback-icon {
        margin-left: -20px;
    }

    .hidden {
        display: none;
    }

    .app-icon-img {
        width: 16px;
        height: 16px;
        border: 1px solid #000;
        object-fit: cover;
        image-rendering: auto;
    }

    .status-badge {
        cursor: default;
        display: flex;
        align-items: center;
        gap: 4px;
    }

    .alert-icon {
        width: 16px;
        height: 16px;
        margin-right: 4px;
        vertical-align: middle;
    }

    .clickable {
        cursor: pointer;
    }

    /* Row Hover Styles */
    tr:hover td .name-cell span.app-name {
        background: #000;
        color: #fff;
    }

    tr:hover :global(.sys7-btn) {
        border-color: #fff;
        box-shadow: 1px 1px 0 #fff;
        background: #000;
        color: #fff;
    }

    tr:hover :global(.sys7-btn:active) {
        background: #fff;
        color: #000;
    }
</style>

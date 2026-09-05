// TypeScript types matching Rust models

export type SourceType = 'github' | 'gitlab' | 'forgejo';
export type CheckAttemptState = 'succeeded' | 'failed' | 'unsupported';
export type CheckOutcomeState = CheckAttemptState | 'skipped';

/** How the last update check for an app turned out. */
export interface CheckAttempt {
  attempted_at: string;
  state: CheckAttemptState;
  message: string | null;
}

/** One app's result from a check run, reported back to the caller. */
export interface CheckOutcome {
  appId: string;
  appName: string;
  state: CheckOutcomeState;
  message: string | null;
}

/** A release file a completed download left in the cache. */
export interface DownloadedRelease {
  version: string;
  path: string;
}

export interface App {
  id: string;
  name: string;
  source_type: SourceType;
  source_url: string;
  current_version: string | null;
  latest_version: string | null;
  install_path: string | null;
  last_checked: string | null;
  /** The most recent attempt, successful or not; null before the first check. */
  last_check_attempt: CheckAttempt | null;
  /** The cached release file, when its version is downloaded already. */
  downloaded: DownloadedRelease | null;
  /** Forgejo instance credentials; null for sources that need none. */
  username: string | null;
  access_token: string | null;
}

/** What the Add/Edit Program dialog collects about a tracked program. */
export interface SourceInput {
  url: string;
  name: string;
  /** `null` leaves the forge for the backend to detect from the URL. */
  sourceType: SourceType | null;
  username: string | null;
  accessToken: string | null;
}

export interface Settings {
  github_token: string | null;
  gitlab_token: string | null;
}

/** What an export of the program list wrote. */
export interface ExportSummary {
  fileName: string;
  count: number;
}

/** An entry of an imported program list that could not be added, and why. */
export interface RejectedEntry {
  /** The entry's name, or its position in the file when it has none. */
  label: string;
  reason: string;
}

/** What an import of a program list did with the file's entries. */
export interface ImportSummary {
  fileName: string;
  added: number;
  /** Entries whose repository was tracked already, left as they were. */
  duplicates: number;
  rejected: RejectedEntry[];
  /**
   * Added Forgejo programs that came with a username but no application key:
   * the export never writes keys, so these need theirs entered again.
   */
  missingKeys: number;
}

export interface Release {
  version: string;
  download_url: string;
  file_name: string;
  file_size: number | null;
  checksum: string | null;
  release_notes: string | null;
}

export interface SystemColors {
  accent_color: string | null;
  accent_text_color: string | null;
  highlight_color: string | null;
  highlight_text_color: string | null;
}

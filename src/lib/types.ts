// TypeScript types matching Rust models

export type SourceType = 'github' | 'gitlab';
export type CheckAttemptState = 'succeeded' | 'failed' | 'unsupported';
export type CheckOutcomeState = CheckAttemptState | 'skipped';

export interface CheckAttempt {
  attempted_at: string;
  state: CheckAttemptState;
  message: string | null;
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
  last_check_attempt: CheckAttempt | null;
}

export interface CheckOutcome {
  app_id: string;
  app_name: string;
  state: CheckOutcomeState;
  message: string | null;
}

export interface Settings {
  github_token: string | null;
  gitlab_token: string | null;
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

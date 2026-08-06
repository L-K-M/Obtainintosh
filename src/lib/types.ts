// TypeScript types matching Rust models

export type SourceType = 'github' | 'gitlab' | 'forgejo';

export interface App {
  id: string;
  name: string;
  source_type: SourceType;
  source_url: string;
  current_version: string | null;
  latest_version: string | null;
  install_path: string | null;
  last_checked: string | null;
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

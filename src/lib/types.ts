// TypeScript types matching Rust models

export type SourceType = 'github' | 'gitlab';

export interface App {
  id: string;
  name: string;
  source_type: SourceType;
  source_url: string;
  current_version: string | null;
  latest_version: string | null;
  install_path: string | null;
  last_checked: string | null;
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

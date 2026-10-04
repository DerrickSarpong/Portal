// Types mirroring the Rust `Snapshot` structs (serde camelCase).

export interface Framework {
  id: string;
  label: string;
  color: string;
}

export type ServerKind = "web" | "adb" | "docker" | "system" | "other";

export interface ServerEntry {
  port: number;
  pid: number;
  processName: string;
  project: string;
  framework: Framework;
  cwd: string | null;
  kind: ServerKind;
  restartable: boolean;
}

export interface AvdEntry {
  name: string;
  running: boolean;
  serial: string | null;
  /** "stopped" | "offline" | "booting" | "ready" | "unauthorized" */
  status: string;
}

export interface EmulatorsState {
  sdkPath: string | null;
  sdkError: string | null;
  avds: AvdEntry[];
}

export interface Snapshot {
  servers: ServerEntry[];
  emulators: EmulatorsState;
  updatedAt: number;
  error: string | null;
}

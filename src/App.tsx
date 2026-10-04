import { useCallback, useEffect, useState } from "react";
import type { ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AvdEntry, ServerEntry, Snapshot } from "./types";

const EMPTY: Snapshot = {
  servers: [],
  emulators: { sdkPath: null, sdkError: null, avds: [] },
  updatedAt: 0,
  error: null,
};

type Tab = "servers" | "android";

const AVD_STATUS_STYLES: Record<string, string> = {
  ready: "bg-emerald-500/15 text-emerald-300",
  booting: "bg-amber-500/15 text-amber-300 animate-pulse",
  offline: "bg-red-500/15 text-red-300",
  unauthorized: "bg-orange-500/15 text-orange-300",
  stopped: "bg-zinc-700/40 text-zinc-400",
};

export default function App() {
  const [snap, setSnap] = useState<Snapshot>(EMPTY);
  const [tab, setTab] = useState<Tab>("servers");
  // Hide/show system-noise rows in the ports list (persisted in localStorage).
  const [showSystem, setShowSystem] = useState<boolean>(() => {
    try {
      return localStorage.getItem("portal.showSystem") === "1";
    } catch {
      return false;
    }
  });
  // Show only dev-server rows (Next.js, React, FastAPI, …) — persisted.
  const [serversOnly, setServersOnly] = useState<boolean>(() => {
    try {
      return localStorage.getItem("portal.serversOnly") === "1";
    } catch {
      return false;
    }
  });
  const [busy, setBusy] = useState<string | null>(null);
  const [autostart, setAutostart] = useState(false);
  const [toast, setToast] = useState<string | null>(null);

  // Initial snapshot + live updates from the Rust poller (every 4 s).
  useEffect(() => {
    let disposed = false;
    let un: (() => void) | undefined;

    invoke<Snapshot>("get_snapshot")
      .then((s) => {
        if (!disposed) setSnap(s);
      })
      .catch(() => {});
    listen<Snapshot>("snapshot", (e) => {
      if (!disposed) setSnap(e.payload);
    }).then((u) => {
      un = u;
    });
    invoke<boolean>("get_start_at_login")
      .then((v) => {
        if (!disposed) setAutostart(v);
      })
      .catch(() => {});

    return () => {
      disposed = true;
      un?.();
    };
  }, []);

  // Auto-clear the toast.
  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 2800);
    return () => clearTimeout(t);
  }, [toast]);

  const run = useCallback(
    async (key: string, okMsg: string, action: () => Promise<unknown>) => {
      setBusy(key);
      try {
        await action();
        setToast(okMsg);
      } catch (err) {
        setToast(String(err));
      } finally {
        setBusy(null);
      }
    },
    [],
  );

  const openServer = (s: ServerEntry) =>
    run(`open:${s.port}`, `Opened :${s.port} in browser`, () =>
      invoke("open_url", { url: `http://localhost:${s.port}` }),
    );

  const restartServer = (s: ServerEntry) =>
    run(`restart:${s.pid}`, `Restarted ${s.project}`, () =>
      invoke("restart_server", { pid: s.pid }),
    );

  const launchAvd = (a: AvdEntry, cold: boolean) =>
    run(`avd:${a.name}:${cold}`, cold ? `Cold-booting ${a.name}` : `Booting ${a.name}`, () =>
      invoke("launch_avd", { name: a.name, cold }),
    );

  const killAvd = (a: AvdEntry) =>
    run(`kill:${a.name}`, `Stopped ${a.name}`, () =>
      invoke("kill_avd", { serial: a.serial ?? "" }),
    );

  const toggleSystem = () =>
    setShowSystem((v) => {
      const next = !v;
      try {
        localStorage.setItem("portal.showSystem", next ? "1" : "0");
      } catch {
        // localStorage unavailable — the state still toggles for this session.
      }
      return next;
    });

  const toggleServersOnly = () =>
    setServersOnly((v) => {
      const next = !v;
      try {
        localStorage.setItem("portal.serversOnly", next ? "1" : "0");
      } catch {
        // localStorage unavailable — the state still toggles for this session.
      }
      return next;
    });

  const toggleAutostart = () =>
    run("autostart", autostart ? "Start at Login off" : "Start at Login on", async () => {
      const next = await invoke<boolean>("set_start_at_login", { enabled: !autostart });
      setAutostart(next);
    });

  const allServers = snap.servers;
  const systemCount = allServers.filter((s) => s.kind === "system").length;
  const servers = serversOnly
    ? allServers.filter((s) => s.kind === "web")
    : showSystem
      ? allServers
      : allServers.filter((s) => s.kind !== "system");
  const avds = snap.emulators.avds;
  const runningAvds = avds.filter((a) => a.running).length;

  return (
    <div className="relative flex h-screen select-none flex-col overflow-hidden bg-zinc-950 font-sans text-zinc-100">
      {/* Frameless widget header — doubles as the drag handle */}
      <header
        data-tauri-drag-region
        className="flex items-center gap-2 border-b border-zinc-900 px-3 py-2.5"
      >
        <span
          data-tauri-drag-region
          className="h-2 w-2 animate-pulse rounded-full bg-emerald-400"
        />
        <span
          data-tauri-drag-region
          className="flex-1 text-[13px] font-semibold tracking-wide"
        >
          Portal
        </span>
        <span data-tauri-drag-region className="text-[11px] text-zinc-500">
          {allServers.length} {allServers.length === 1 ? "port" : "ports"}
        </span>
        <button
          onClick={() => invoke("hide_window").catch(() => {})}
          title="Hide to tray"
          className="rounded px-1.5 text-zinc-500 hover:bg-zinc-800 hover:text-zinc-200"
        >
          &#10005;
        </button>
      </header>

      {/* Tabs */}
      <nav className="flex items-center gap-1 px-2 pt-2">
        <TabButton
          active={tab === "servers"}
          onClick={() => setTab("servers")}
          label="Dev Servers"
          badge={servers.length}
        />
        <TabButton
          active={tab === "android"}
          onClick={() => setTab("android")}
          label="Android"
          badge={runningAvds}
        />
        <div className="flex-1" />
        {tab === "servers" && (
          <>
            <button
              onClick={toggleSystem}
              disabled={serversOnly}
              title={
                serversOnly
                  ? "Not applicable while 'Servers only' is active"
                  : showSystem
                    ? "System noise is shown — click to hide"
                    : "System noise is hidden — click to show"
              }
              className={`flex items-center gap-1.5 rounded-md px-2 py-1.5 text-[11px] transition-colors ${
                serversOnly
                  ? "cursor-not-allowed text-zinc-600 opacity-50"
                  : showSystem
                    ? "text-zinc-500 hover:text-zinc-300"
                    : "text-zinc-200 hover:text-zinc-50"
              }`}
            >
              {showSystem ? <EyeIcon /> : <EyeOffIcon />}
              <span>
                System
                {!serversOnly && !showSystem && systemCount > 0 ? ` (${systemCount} hidden)` : ""}
              </span>
            </button>
            <button
              onClick={toggleServersOnly}
              title={
                serversOnly
                  ? "Showing only dev servers — click to show everything"
                  : "Show only dev servers"
              }
              className={`flex items-center gap-1.5 rounded-md px-2 py-1.5 text-[11px] transition-colors ${
                serversOnly
                  ? "bg-zinc-800 text-zinc-100"
                  : "text-zinc-500 hover:text-zinc-300"
              }`}
            >
              <ServerIcon />
              Servers only
            </button>
          </>
        )}
      </nav>

      <main className="flex-1 overflow-y-auto">
        {snap.error && (
          <div className="m-2 rounded-md bg-red-500/10 px-3 py-2 text-[11px] text-red-300">
            {snap.error}
          </div>
        )}

        {tab === "servers" ? (
          servers.length === 0 ? (
            <EmptyState
              text={
                allServers.length === 0
                  ? "No listening servers. Start a dev server and it will show up here."
                  : serversOnly
                    ? "No dev servers are listening right now — 'Servers only' is on."
                    : "Only system services are listening — they are hidden. Toggle 'System' to show them."
              }
            />
          ) : (
            <ul>
              {servers.map((s) => (
                <li
                  key={`${s.port}-${s.pid}`}
                  className="flex items-center gap-2.5 border-b border-zinc-900/70 px-3 py-2 hover:bg-zinc-900/50"
                >
                  <span
                    className="h-2.5 w-2.5 shrink-0 rounded-full ring-1 ring-white/10"
                    style={{ backgroundColor: s.framework.color }}
                  />
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-[13px] font-medium" title={s.cwd ?? ""}>
                      {s.project}
                    </div>
                    <div className="truncate text-[11px] text-zinc-500">
                      {s.framework.label} &middot; :{s.port} &middot; pid {s.pid}
                    </div>
                  </div>
                  {s.kind === "web" && (
                    <IconButton
                      title={`Open http://localhost:${s.port}`}
                      busy={busy === `open:${s.port}`}
                      onClick={() => openServer(s)}
                    >
                      <GlobeIcon />
                    </IconButton>
                  )}
                  {s.restartable && (
                    <IconButton
                      title={`Restart ${s.project}`}
                      busy={busy === `restart:${s.pid}`}
                      onClick={() => restartServer(s)}
                    >
                      <RestartIcon />
                    </IconButton>
                  )}
                </li>
              ))}
            </ul>
          )
        ) : (
          <div>
            {snap.emulators.sdkError ? (
              <div className="m-2 rounded-md bg-amber-500/10 px-3 py-2 text-[11px] leading-relaxed text-amber-300">
                {snap.emulators.sdkError}
              </div>
            ) : (
              <div
                className="truncate px-3 pb-1 pt-2 text-[11px] text-zinc-500"
                title={snap.emulators.sdkPath ?? ""}
              >
                SDK &middot; {snap.emulators.sdkPath}
              </div>
            )}
            {avds.length === 0 && !snap.emulators.sdkError ? (
              <EmptyState text="No Android Virtual Devices found. Create one in Android Studio → Device Manager." />
            ) : (
              <ul>
                {avds.map((a) => (
                  <li
                    key={a.name}
                    className="flex items-center gap-2.5 border-b border-zinc-900/70 px-3 py-2 hover:bg-zinc-900/50"
                  >
                    <AndroidIcon />
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-[13px] font-medium">{a.name}</div>
                      <div className="truncate text-[11px] text-zinc-500">
                        {a.serial ?? "not running"}
                      </div>
                    </div>
                    <span
                      className={`shrink-0 rounded-full px-2 py-0.5 text-[10px] ${
                        AVD_STATUS_STYLES[a.status] ?? AVD_STATUS_STYLES.stopped
                      }`}
                    >
                      {a.status}
                    </span>
                    {a.running ? (
                      <IconButton
                        title={`Stop ${a.name}`}
                        busy={busy === `kill:${a.name}`}
                        onClick={() => killAvd(a)}
                      >
                        <StopIcon />
                      </IconButton>
                    ) : (
                      <>
                        <IconButton
                          title={`Boot ${a.name}`}
                          busy={busy === `avd:${a.name}:false`}
                          onClick={() => launchAvd(a, false)}
                        >
                          <PlayIcon />
                        </IconButton>
                        <IconButton
                          title={`Cold boot ${a.name}`}
                          busy={busy === `avd:${a.name}:true`}
                          onClick={() => launchAvd(a, true)}
                        >
                          <ColdBootIcon />
                        </IconButton>
                      </>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </div>
        )}
      </main>

      {toast && (
        <div className="pointer-events-none absolute bottom-12 left-1/2 -translate-x-1/2 rounded-md bg-zinc-800 px-3 py-1.5 text-[11px] text-zinc-100 shadow-lg">
          {toast}
        </div>
      )}

      <footer className="flex items-center justify-between border-t border-zinc-900 px-3 py-2 text-[11px] text-zinc-500">
        <label className="flex cursor-pointer items-center gap-1.5">
          <input
            type="checkbox"
            checked={autostart}
            onChange={toggleAutostart}
            className="accent-emerald-500"
          />
          Start at Login
        </label>
        <span>
          {snap.error
            ? "scan error"
            : snap.updatedAt
              ? new Date(snap.updatedAt).toLocaleTimeString()
              : "\u2026"}
        </span>
      </footer>
    </div>
  );
}

function TabButton({
  active,
  onClick,
  label,
  badge,
}: {
  active: boolean;
  onClick: () => void;
  label: string;
  badge: number;
}) {
  return (
    <button
      onClick={onClick}
      className={`flex items-center gap-1.5 rounded-md px-3 py-1.5 text-[12px] font-medium transition-colors ${
        active ? "bg-zinc-800 text-zinc-100" : "text-zinc-500 hover:text-zinc-300"
      }`}
    >
      {label}
      {badge > 0 && (
        <span className="rounded-full bg-zinc-700 px-1.5 text-[10px] text-zinc-300">
          {badge}
        </span>
      )}
    </button>
  );
}

function IconButton({
  title,
  busy,
  onClick,
  children,
}: {
  title: string;
  busy: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      title={title}
      onClick={onClick}
      disabled={busy}
      className={`shrink-0 rounded-md p-1.5 text-zinc-500 transition-colors hover:bg-zinc-800 hover:text-zinc-100 ${
        busy ? "animate-pulse text-emerald-400" : ""
      }`}
    >
      {children}
    </button>
  );
}

function EmptyState({ text }: { text: string }) {
  return (
    <div className="px-6 py-10 text-center text-[12px] leading-relaxed text-zinc-600">
      {text}
    </div>
  );
}

function ServerIcon() {
  return (
    <svg
      width="13"
      height="13"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <rect x="2" y="2" width="20" height="8" rx="2" />
      <rect x="2" y="14" width="20" height="8" rx="2" />
      <line x1="6" y1="6" x2="6.01" y2="6" />
      <line x1="6" y1="18" x2="6.01" y2="18" />
    </svg>
  );
}

function EyeIcon() {
  return (
    <svg
      width="13"
      height="13"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7z" />
      <circle cx="12" cy="12" r="3" />
    </svg>
  );
}

function EyeOffIcon() {
  return (
    <svg
      width="13"
      height="13"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94" />
      <path d="M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19" />
      <path d="M14.12 14.12a3 3 0 1 1-4.24-4.24" />
      <line x1="1" y1="1" x2="23" y2="23" />
    </svg>
  );
}

function GlobeIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <circle cx="12" cy="12" r="10" />
      <path d="M2 12h20" />
      <path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z" />
    </svg>
  );
}

function RestartIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8" />
      <path d="M21 3v5h-5" />
    </svg>
  );
}

function PlayIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
      <path d="M8 5v14l11-7z" />
    </svg>
  );
}

function StopIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
      <rect x="6" y="6" width="12" height="12" rx="1" />
    </svg>
  );
}

function ColdBootIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" />
    </svg>
  );
}

function AndroidIcon() {
  return (
    <svg width="15" height="15" viewBox="0 0 24 24" fill="currentColor" className="shrink-0 text-emerald-400">
      <path d="M17.6 9.48l1.84-3.18c.16-.31.04-.7-.26-.85-.29-.16-.7-.05-.85.26l-1.88 3.24a11.46 11.46 0 0 0-8.9 0L5.67 5.71c-.16-.32-.56-.42-.85-.26-.3.16-.42.54-.26.85l1.84 3.18C3.3 11.25 1.28 14.44 1 18h22c-.28-3.56-2.3-6.75-5.4-8.52zM7 15.25a1.25 1.25 0 1 1 0-2.5 1.25 1.25 0 0 1 0 2.5zm10 0a1.25 1.25 0 1 1 0-2.5 1.25 1.25 0 0 1 0 2.5z" />
    </svg>
  );
}

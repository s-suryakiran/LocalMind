import { useEffect, useState } from "react";
import { api, listen, remoteStatus } from "./lib/api";
import { useApp } from "./lib/store";
import { isTauri } from "./lib/util";
import {
  runReachabilityPoller,
  ONLINE_POLL_MS,
  OFFLINE_POLL_MS,
} from "./lib/online";
import { Sidebar } from "./components/Sidebar";
import { OfflineBanner } from "./components/OfflineBanner";
import { UpdateBanner } from "./components/UpdateBanner";
import { Chat } from "./pages/Chat";
import { Marketplace } from "./pages/Marketplace";
import { Models } from "./pages/Models";
import { Knowledge } from "./pages/Knowledge";
import { ImageGen } from "./pages/ImageGen";
import { Synapse } from "./pages/Synapse";
import { Settings } from "./pages/Settings";
import { Connect } from "./pages/Connect";

function App() {
  const { view, setHardware, setInstalled, setLanUrl, setLlama, connection, setOnline } = useApp();
  const remote = !isTauri();
  const [sidebarOpen, setSidebarOpen] = useState(false);

  // Reset hatch (?reset / ?logout)
  useEffect(() => {
    if (typeof window === "undefined") return;
    const params = new URL(window.location.href).searchParams;
    if (params.has("reset") || params.has("logout")) {
      try { localStorage.removeItem("localmind-store"); } catch { /* private mode etc. */ }
      window.location.replace(window.location.origin + "/");
    }
  }, []);

  // PWA reachability poller. Only on the phone PWA path — desktop Tauri
  // talks to its own backend over IPC and doesn't need network probes.
  useEffect(() => {
    if (!remote || !connection) return;
    const base = connection.url.replace(/\/+$/, "");
    let currentOnline = useApp.getState().online;
    const stop = runReachabilityPoller({
      // /api/health is auth-public and ALWAYS returns 200 when the LAN
      // server is reachable, regardless of whether a model is loaded.
      // Don't use /health here — it proxies to llama-server and fails
      // any time no model is running, which would falsely flip the
      // banner on while the desktop is perfectly fine.
      probe: () => fetch(`${base}/api/health`, { method: "GET", cache: "no-store" }),
      intervalMs: () => (currentOnline ? ONLINE_POLL_MS : OFFLINE_POLL_MS),
      onSuccess: () => {
        currentOnline = true;
        setOnline(true, Date.now());
      },
      onFailure: () => {
        currentOnline = false;
        setOnline(false, useApp.getState().lastOnlineAt);
      },
    });

    const onWindowOnline = () => setOnline(true, Date.now());
    const onWindowOffline = () => setOnline(false, useApp.getState().lastOnlineAt);
    window.addEventListener("online", onWindowOnline);
    window.addEventListener("offline", onWindowOffline);

    return () => {
      stop();
      window.removeEventListener("online", onWindowOnline);
      window.removeEventListener("offline", onWindowOffline);
    };
  }, [remote, connection, setOnline]);

  useEffect(() => {
    if (!remote) {
      api.detectHardware().then(setHardware).catch(console.error);
      api.listInstalledModels().then(setInstalled).catch(() => {});
      api.llamaStatus().then(setLlama).catch(() => {});
      api.getLanUrl().then((u) => u && setLanUrl(u)).catch(() => {});

      const unlistenLan = listen<string>("lan:ready", (url) => setLanUrl(url));
      const unlistenReady = listen<{ port: number; modelId: string }>("llama:ready", () => {
        api.llamaStatus().then(setLlama).catch(() => {});
      });

      return () => {
        Promise.all([unlistenLan, unlistenReady]).then((fns) => fns.forEach((fn) => fn()));
      };
    }
    if (connection) {
      // The /api/status poll doubles as the reachability signal — when
      // it succeeds we're online, when it fails the host is unreachable.
      // This runs alongside the dedicated /health poller; either one
      // updating `online` is enough to surface the banner.
      const ok = (s: Parameters<typeof setLlama>[0]) => {
        setLlama(s);
        setOnline(true, Date.now());
      };
      const fail = () => setOnline(false, useApp.getState().lastOnlineAt);
      remoteStatus().then(ok).catch(fail);
      const t = setInterval(() => remoteStatus().then(ok).catch(fail), 5000);
      return () => clearInterval(t);
    }
  }, [remote, connection, setHardware, setInstalled, setLanUrl, setLlama, setOnline]);

  if (remote && !connection) {
    return <Connect />;
  }

  return (
    <div className="h-screen w-screen flex flex-col overflow-hidden">
      <UpdateBanner />
      <OfflineBanner />
      <div className="flex-1 min-h-0 flex">
        <Sidebar remote={remote} open={sidebarOpen} onClose={() => setSidebarOpen(false)} />
        <main className="flex-1 min-w-0 bg-[var(--color-bg)]">
          {view === "chat" && <Chat onOpenMenu={() => setSidebarOpen(true)} />}
          {!remote && view === "marketplace" && <Marketplace />}
          {!remote && view === "models" && <Models />}
          {!remote && view === "knowledge" && <Knowledge />}
          {!remote && view === "image" && <ImageGen />}
          {view === "synapse" && <Synapse />}
          {view === "settings" && <Settings />}
        </main>
      </div>
    </div>
  );
}

export default App;

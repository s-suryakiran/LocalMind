import { useEffect, useState } from "react";
import { api, listen, remoteStatus } from "./lib/api";
import { useApp } from "./lib/store";
import { isTauri } from "./lib/util";
import { Sidebar } from "./components/Sidebar";
import { Chat } from "./pages/Chat";
import { Marketplace } from "./pages/Marketplace";
import { Models } from "./pages/Models";
import { Knowledge } from "./pages/Knowledge";
import { ImageGen } from "./pages/ImageGen";
import { Synapse } from "./pages/Synapse";
import { Settings } from "./pages/Settings";
import { Connect } from "./pages/Connect";

function App() {
  const { view, setHardware, setInstalled, setLanUrl, setLlama, connection } = useApp();
  const remote = !isTauri();
  // Mobile: sidebar starts closed, slides in over the chat. md+: always open.
  const [sidebarOpen, setSidebarOpen] = useState(false);

  // Recovery hatch: visiting any URL with `?reset` (or `?logout`) clears the
  // persisted store and reloads. Useful when a paired phone has a stale token
  // or got into a bad state we can't reach via the UI.
  useEffect(() => {
    if (typeof window === "undefined") return;
    const params = new URL(window.location.href).searchParams;
    if (params.has("reset") || params.has("logout")) {
      try {
        localStorage.removeItem("localmind-store");
      } catch {
        /* private mode etc. */
      }
      window.location.replace(window.location.origin + "/");
    }
  }, []);

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
      remoteStatus().then(setLlama).catch(() => {});
      const t = setInterval(() => remoteStatus().then(setLlama).catch(() => {}), 5000);
      return () => clearInterval(t);
    }
  }, [remote, connection, setHardware, setInstalled, setLanUrl, setLlama]);

  // Phone / PWA path: not in Tauri and no paired token yet.
  if (remote && !connection) {
    return <Connect />;
  }

  return (
    <div className="h-screen w-screen flex overflow-hidden">
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
  );
}

export default App;

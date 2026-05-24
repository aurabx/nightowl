import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Sidebar, type PageId } from "./components/Sidebar";
import { PeersPage } from "./pages/Peers";
import { ScuPage } from "./pages/Scu";
import { ActivityPage } from "./pages/Activity";
import { StorePage } from "./pages/Store";
import { WorklistPage } from "./pages/Worklist";
import { SettingsPage } from "./pages/Settings";

function App() {
  const [currentPage, setCurrentPage] = useState<PageId>("peers");
  const [pingResult, setPingResult] = useState<string>("(checking IPC...)");

  // Verify the Tauri IPC channel is alive on mount. The backend `ping`
  // command returns "pong"; if anything else shows up we know wiring is
  // broken before any feature work begins.
  useEffect(() => {
    invoke<string>("ping")
      .then((result) => setPingResult(result))
      .catch((err) => setPingResult(`error: ${String(err)}`));
  }, []);

  return (
    <div className="flex h-full w-full bg-slate-950 text-slate-100">
      <Sidebar currentPage={currentPage} onSelect={setCurrentPage} />
      <main className="flex-1 overflow-auto">
        <div className="px-8 py-6">
          {currentPage === "peers" && <PeersPage />}
          {currentPage === "scu" && <ScuPage />}
          {currentPage === "activity" && <ActivityPage />}
          {currentPage === "store" && <StorePage />}
          {currentPage === "worklist" && <WorklistPage />}
          {currentPage === "settings" && <SettingsPage />}
        </div>
        <footer className="px-8 py-3 text-xs text-slate-500 border-t border-slate-800">
          IPC self-check: {pingResult}
        </footer>
      </main>
    </div>
  );
}

export default App;

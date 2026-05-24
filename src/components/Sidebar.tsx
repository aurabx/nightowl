import {
  Activity,
  ClipboardList,
  Database,
  Info,
  Network,
  Send,
  Settings,
} from "lucide-react";
import type { ComponentType, SVGProps } from "react";

export type PageId =
  | "peers"
  | "scu"
  | "activity"
  | "store"
  | "worklist"
  | "settings"
  | "about";

interface SidebarItem {
  id: PageId;
  label: string;
  icon: ComponentType<SVGProps<SVGSVGElement>>;
}

const ITEMS: ReadonlyArray<SidebarItem> = [
  { id: "peers", label: "Peers", icon: Network },
  { id: "scu", label: "SCU", icon: Send },
  { id: "activity", label: "Activity", icon: Activity },
  { id: "store", label: "Store", icon: Database },
  { id: "worklist", label: "Worklist", icon: ClipboardList },
  { id: "settings", label: "Settings", icon: Settings },
  { id: "about", label: "About", icon: Info },
];

interface SidebarProps {
  currentPage: PageId;
  onSelect: (id: PageId) => void;
}

export function Sidebar({ currentPage, onSelect }: SidebarProps) {
  return (
    <aside className="w-56 shrink-0 border-r border-slate-800 bg-slate-900">
      <div className="px-5 py-5 border-b border-slate-800">
        <div className="text-lg font-semibold">NightOwl</div>
        <div className="text-xs text-slate-500">DICOM service tester</div>
      </div>
      <nav className="py-3">
        {ITEMS.map((item) => {
          const Icon = item.icon;
          const active = item.id === currentPage;
          return (
            <button
              type="button"
              key={item.id}
              onClick={() => onSelect(item.id)}
              className={
                "flex w-full items-center gap-3 px-5 py-2 text-sm transition-colors " +
                (active
                  ? "bg-slate-800 text-slate-100"
                  : "text-slate-400 hover:bg-slate-800/60 hover:text-slate-200")
              }
            >
              <Icon className="size-4" />
              {item.label}
            </button>
          );
        })}
      </nav>
    </aside>
  );
}

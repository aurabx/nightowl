import type { MouseEvent } from "react";
import { formatError, openUrl } from "../lib/api";

const NIGHTOWL_URL = "https://aurabox.cloud/nightowl";

export function AboutPage() {
  const handleOpen = async (e: MouseEvent<HTMLAnchorElement>) => {
    e.preventDefault();
    try {
      await openUrl(NIGHTOWL_URL);
    } catch (err) {
      console.error("failed to open url", formatError(err));
    }
  };

  return (
    <div>
      <h1 className="text-2xl font-semibold">About</h1>
      <div className="mt-6 space-y-2">
        <div className="text-lg">NightOwl by Aurabox</div>
        <a
          href={NIGHTOWL_URL}
          onClick={handleOpen}
          className="text-sm text-sky-400 hover:text-sky-300 hover:underline"
        >
          {NIGHTOWL_URL}
        </a>
      </div>
    </div>
  );
}

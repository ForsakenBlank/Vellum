import { TriangleAlert, X } from "lucide-react";

import { IconButton } from "../components/Button";
import { useApp } from "../stores/app";

/** shows the last thing that went wrong until the user dismisses it */
export function Notice() {
  const notice = useApp((state) => state.notice);
  const setNotice = useApp((state) => state.setNotice);
  if (!notice) return null;

  return (
    <div
      role="alert"
      className="flex items-center gap-2 border-b border-line bg-raised px-3 py-1.5 text-sm"
    >
      <TriangleAlert className="size-4 shrink-0 text-warn" aria-hidden />
      <span className="min-w-0 flex-1 select-text">{notice}</span>
      <IconButton
        label="Dismiss"
        onClick={() => {
          setNotice(null);
        }}
      >
        <X className="size-4" />
      </IconButton>
    </div>
  );
}

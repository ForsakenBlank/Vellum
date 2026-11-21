// the rename preview and the rename itself, for the converter's rename mode

import type { DocumentInfo } from "../../bindings/DocumentInfo";
import type { RenamePattern } from "../../bindings/RenamePattern";
import { ipc } from "../../lib/ipc";
import { fail, useDebounced, useLatest } from "./session";

/** rust's preview of the new names, following the pattern as it is typed */
export function usePlan(doc: DocumentInfo, pattern: RenamePattern) {
  const settled = useDebounced(pattern, 150);
  // the inputs are in the key, so the list follows renames, undo and added files
  const key = JSON.stringify([settled, doc.inputs]);
  return useLatest(() => ipc.renamePlan(doc.id, settled), key);
}

/** renames as one undo step */
export function renameFiles(doc: DocumentInfo, renames: { from: string; to: string }[]) {
  return ipc
    .documentCommand(doc.id, "tool.rename_files", { renames })
    .catch(fail("The files were not renamed"));
}

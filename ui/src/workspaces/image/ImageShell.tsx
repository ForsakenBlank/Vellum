import { save } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";

import { inDesktopApp, ipc } from "../../lib/ipc";
import { registerUiHandlers } from "../../state/commands";
import { activeDocument, useDocuments } from "../../state/documents";
import { useUi } from "../../state/ui";
import { NewImageDialog } from "./dialogs";

function fail(err: unknown) {
  useUi.getState().toast({ level: "error", title: "That did not work", body: String(err) });
}

/** asks where to save the active image document, then saves it there as a .vellum file */
async function saveAs() {
  const doc = activeDocument(useDocuments.getState());
  if (!doc) return;
  if (doc.docType !== "image") {
    useUi.getState().toast({
      level: "info",
      title: "Nothing to save as",
      body: "Only pictures in the image editor can be saved as a file so far.",
    });
    return;
  }
  const stem = doc.title.replace(/\.[^.]+$/, "") || "Untitled";
  const path = await save({
    title: `Save ${doc.title} as`,
    defaultPath: `${stem}.vellum`,
    filters: [{ name: "Vellum picture", extensions: ["vellum"] }],
  });
  if (path) await ipc.imageSaveAs(doc.id, path);
}

/** the image editor's commands that work from anywhere: a new picture and save as */
export function ImageShell() {
  const [creating, setCreating] = useState(false);
  useEffect(
    () =>
      registerUiHandlers({
        "file.new": () => {
          if (inDesktopApp) setCreating(true);
        },
        "file.save_as": () => saveAs().catch(fail),
      }),
    [],
  );
  return (
    <NewImageDialog
      open={creating}
      onClose={() => {
        setCreating(false);
      }}
      fail={fail}
    />
  );
}

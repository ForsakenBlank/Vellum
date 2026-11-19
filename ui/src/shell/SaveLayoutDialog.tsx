import { useState } from "react";

import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { saveLayoutAs } from "./actions";

/** asks for a name, then keeps the current panel arrangement under it */
export function SaveLayoutDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [name, setName] = useState("");
  const save = () => {
    if (!name.trim()) return;
    saveLayoutAs(name);
    setName("");
    onClose();
  };
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
      title="Save layout"
      description="Keep where every panel sits, and switch back to it from View, Layout or the palette."
      width="24rem"
      footer={
        <>
          <Button variant="ghost" tooltip="Do not save" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            tooltip="Save this layout"
            disabled={!name.trim()}
            onClick={save}
          >
            Save
          </Button>
        </>
      }
    >
      <div className="px-5 py-4">
        <input
          autoFocus
          aria-label="Layout name"
          placeholder="For example, Colour grading"
          value={name}
          maxLength={60}
          onChange={(event) => {
            setName(event.target.value);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") save();
          }}
          className="h-(--row) w-full rounded-md border border-line-strong bg-raised px-2.5 text-sm outline-none focus:border-accent-line"
        />
      </div>
    </Dialog>
  );
}

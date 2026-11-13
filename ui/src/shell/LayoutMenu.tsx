import { EyeOff, LayoutPanelLeft, RotateCcw, Save, Trash2 } from "lucide-react";
import { useState } from "react";

import { useKeymap } from "../commands/registry";
import { Button, IconButton } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { Menu } from "../components/Menu";
import { findPanel } from "../dock/layout";
import {
  deleteNamedLayout,
  resetLayout,
  saveLayout,
  savedLayouts,
  saveNamedLayout,
  toggleAllPanels,
  togglePanelIn,
  useLayout,
} from "../dock/state";
import { useSettings } from "../stores/settings";
import type { Workspace } from "../workspaces/types";

/** shows and hides panels, and saves and switches panel layouts */
export function LayoutMenu({ workspace }: { workspace: Workspace }) {
  const layout = useLayout(workspace);
  const layouts = useSettings((state) => state.sections?.layouts);
  const keymap = useKeymap();
  const [naming, setNaming] = useState(false);
  const saved = savedLayouts(workspace, layouts);

  return (
    <>
      <Menu.Root>
        <Menu.Trigger asChild>
          <IconButton label="Panels and layouts">
            <LayoutPanelLeft className="size-4" />
          </IconButton>
        </Menu.Trigger>
        <Menu.Content align="end">
          <Menu.Label>{workspace.title} panels</Menu.Label>
          {workspace.panels.map((panel) => {
            const place = findPanel(layout, panel.id);
            const showing =
              place !== null &&
              layout[place.zone].open &&
              layout[place.zone].stacks[place.stack]?.active === panel.id;
            return (
              <Menu.Item
                key={panel.id}
                checked={showing}
                shortcut={keymap[`view.panel.${panel.id}`]?.[0]}
                onSelect={() => void togglePanelIn(workspace, panel.id)}
              >
                {panel.title}
              </Menu.Item>
            );
          })}
          <Menu.Separator />
          <Menu.Item
            icon={<EyeOff className="size-3.5" />}
            shortcut={keymap["view.panels"]?.[0]}
            onSelect={() => void toggleAllPanels(workspace)}
          >
            Hide or show all panels
          </Menu.Item>
          <Menu.Item
            icon={<RotateCcw className="size-3.5" />}
            onSelect={() => void resetLayout(workspace)}
          >
            Reset to the default layout
          </Menu.Item>
          <Menu.Separator />
          {saved.length > 0 && <Menu.Label>Saved layouts</Menu.Label>}
          {saved.map((entry) => (
            <Menu.Item
              key={entry.name}
              icon={<LayoutPanelLeft className="size-3.5" />}
              onSelect={() => void saveLayout(workspace, entry.layout)}
            >
              {entry.name}
            </Menu.Item>
          ))}
          <Menu.Item
            icon={<Save className="size-3.5" />}
            onSelect={() => {
              setNaming(true);
            }}
          >
            Save this layout
          </Menu.Item>
          {saved.length > 0 && (
            <Menu.Sub label="Delete a saved layout" icon={<Trash2 className="size-3.5" />}>
              {saved.map((entry) => (
                <Menu.Item
                  key={entry.name}
                  onSelect={() => void deleteNamedLayout(workspace, entry.name)}
                >
                  {entry.name}
                </Menu.Item>
              ))}
            </Menu.Sub>
          )}
        </Menu.Content>
      </Menu.Root>
      <SaveLayoutDialog
        workspace={workspace}
        open={naming}
        onClose={() => {
          setNaming(false);
        }}
      />
    </>
  );
}

function SaveLayoutDialog({
  workspace,
  open,
  onClose,
}: {
  workspace: Workspace;
  open: boolean;
  onClose: () => void;
}) {
  const [name, setName] = useState("");
  const save = () => {
    if (!name.trim()) return;
    void saveNamedLayout(workspace, name);
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
      description={`Give the current ${workspace.title} panel layout a name to switch back to it later. A layout with the same name is replaced.`}
      showDescription
      className="w-[min(26rem,calc(100vw-2rem))]"
    >
      <form
        className="flex flex-col gap-4 p-5"
        onSubmit={(event) => {
          event.preventDefault();
          save();
        }}
      >
        <input
          autoFocus
          aria-label="Layout name"
          placeholder="For example, Editing"
          maxLength={60}
          value={name}
          onChange={(event) => {
            setName(event.target.value);
          }}
          className="v-focus h-9 rounded-md border border-line bg-canvas px-3 text-sm outline-none focus:border-accent"
        />
        <div className="flex justify-end gap-2">
          <Button onClick={onClose}>Cancel</Button>
          <Button type="submit" variant="primary" disabled={!name.trim()}>
            Save layout
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

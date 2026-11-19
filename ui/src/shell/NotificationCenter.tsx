import { Popover } from "radix-ui";
import { Bell, Inbox, X } from "lucide-react";
import { useEffect, useState } from "react";

import type { Notification } from "../bindings/Notification";
import { Button, IconButton } from "../components/Button";
import { EmptyState } from "../components/EmptyState";
import { Tooltip } from "../components/Tooltip";
import { formatAgo } from "../lib/format";
import { ipc } from "../lib/ipc";
import { runCommand } from "../state/commands";
import { useNotifications } from "../state/notifications";
import { useUi } from "../state/ui";
import { levelColours, levelIcons, performAction } from "./notify";

/** the bell in the title bar, with the list in a popover */
export function NotificationBell() {
  const open = useUi((s) => s.notificationsOpen);
  const setOpen = useUi((s) => s.setNotificationsOpen);
  const unread = useNotifications((s) => s.unread);

  return (
    <Popover.Root
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        // opening the list counts as reading it
        if (next && unread > 0) void runCommand("notifications.mark_read");
      }}
    >
      <Tooltip
        text={unread > 0 ? `${unread} new notifications` : "Notifications"}
        command="app.notifications"
      >
        <Popover.Trigger asChild>
          <button
            type="button"
            aria-label="Notifications"
            className="relative flex size-7 shrink-0 items-center justify-center rounded-md text-fg-muted transition-colors duration-(--motion-fast) hover:bg-raised hover:text-fg data-[state=open]:bg-raised data-[state=open]:text-fg"
          >
            <Bell className="size-4" strokeWidth={1.8} />
            {unread > 0 && (
              <span className="absolute top-1 right-1 size-2 rounded-full bg-accent ring-2 ring-panel" />
            )}
          </button>
        </Popover.Trigger>
      </Tooltip>
      <Popover.Portal>
        <Popover.Content
          align="end"
          sideOffset={8}
          collisionPadding={8}
          className="z-[var(--z-popover)] flex max-h-[min(32rem,80vh)] w-96 animate-pop-in flex-col overflow-hidden rounded-xl bg-elevated text-fg shadow-pop"
        >
          <NotificationList />
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}

function NotificationList() {
  const items = useNotifications((s) => s.items);
  // a clock so "2 min ago" stays true while the list is open
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => {
      setNow(Date.now());
    }, 30_000);
    return () => {
      window.clearInterval(timer);
    };
  }, []);

  return (
    <>
      <div className="flex items-center justify-between border-b border-line px-3 py-2">
        <h2 className="text-sm font-semibold">Notifications</h2>
        <Button
          variant="ghost"
          size="sm"
          tooltip="Remove every notification"
          command="notifications.clear"
          disabled={items.length === 0}
          onClick={() => void runCommand("notifications.clear")}
        >
          Clear all
        </Button>
      </div>
      {items.length === 0 ? (
        <EmptyState icon={Inbox} title="You are all caught up" compact>
          Finished exports and other news show up here.
        </EmptyState>
      ) : (
        <ol className="min-h-0 flex-1 overflow-y-auto p-1">
          {items.map((item) => (
            <NotificationRow key={item.id} item={item} now={now} />
          ))}
        </ol>
      )}
    </>
  );
}

function NotificationRow({ item, now }: { item: Notification; now: number }) {
  const Icon = levelIcons[item.level];
  return (
    <li className="group flex gap-2.5 rounded-lg px-2.5 py-2 hover:bg-pressed/50">
      <Icon className={`mt-0.5 size-4 shrink-0 ${levelColours[item.level]}`} />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <div className="flex items-baseline gap-2">
          <span className="min-w-0 flex-1 truncate text-sm font-medium">{item.title}</span>
          <span className="shrink-0 text-2xs text-fg-subtle tabular">
            {formatAgo(item.createdAtMs, now)}
          </span>
        </div>
        {item.body && <p className="text-xs text-fg-muted">{item.body}</p>}
        {item.action && item.actionLabel && (
          <div className="mt-1">
            <Button
              size="sm"
              tooltip={item.actionLabel}
              onClick={() => {
                if (item.action) performAction(item.action);
              }}
            >
              {item.actionLabel}
            </Button>
          </div>
        )}
      </div>
      <IconButton
        size="sm"
        label="Dismiss"
        icon={<X className="size-3" />}
        className="opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
        onClick={() => void ipc.dismissNotification(item.id)}
      />
    </li>
  );
}

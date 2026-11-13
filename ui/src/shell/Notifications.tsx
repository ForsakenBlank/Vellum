import { clsx } from "clsx";
import { Bell, BellOff, CircleAlert, CircleCheck, Info, X } from "lucide-react";
import { Popover } from "radix-ui";
import { useEffect, useState } from "react";

import { useShortcut } from "../commands/registry";
import { Button, IconButton } from "../components/Button";
import { EmptyState } from "../components/EmptyState";
import { formatAgo } from "../lib/format";
import { perform } from "../stores/app";
import {
  type AppNotification,
  type NotificationTone,
  unreadCount,
  useNotifications,
} from "../stores/notifications";
import { useUi } from "../stores/ui";

const TOAST_MS = 6000;

const toneIcon: Record<NotificationTone, typeof Info> = {
  info: Info,
  good: CircleCheck,
  bad: CircleAlert,
};

const toneColor: Record<NotificationTone, string> = {
  info: "text-accent",
  good: "text-good",
  bad: "text-bad",
};

function openResult(item: AppNotification) {
  const path = item.path;
  if (path) void perform((api) => api.files.open(path));
}

/** the bell in the top bar and the list of everything that happened */
export function NotificationCentre() {
  const open = useUi((state) => state.notificationsOpen);
  const setOpen = useUi((state) => state.setNotificationsOpen);
  const items = useNotifications((state) => state.items);
  const markAllRead = useNotifications((state) => state.markAllRead);
  const clear = useNotifications((state) => state.clear);
  const remove = useNotifications((state) => state.remove);
  const shortcut = useShortcut("app.notifications");
  const unread = unreadCount(items);

  return (
    <Popover.Root
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        // seeing the list counts as reading it
        if (!next) markAllRead();
      }}
    >
      <Popover.Trigger asChild>
        <IconButton
          label={unread > 0 ? `Notifications, ${unread} new` : "Notifications"}
          shortcut={shortcut}
          className="relative"
        >
          <Bell className="size-4" />
          {unread > 0 && (
            <span className="absolute -top-0.5 -right-0.5 flex h-3.5 min-w-3.5 items-center justify-center rounded-full bg-accent px-1 text-[0.5625rem] font-semibold text-on-accent">
              {unread > 9 ? "9+" : unread}
            </span>
          )}
        </IconButton>
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content
          align="end"
          sideOffset={6}
          aria-label="Notifications"
          className="v-pop z-50 flex max-h-[min(30rem,70vh)] w-80 flex-col overflow-hidden rounded-xl border border-line bg-raised shadow-pop"
        >
          <header className="flex h-10 shrink-0 items-center justify-between border-b border-line px-3">
            <h2 className="text-sm font-semibold">Notifications</h2>
            {items.length > 0 && (
              <Button className="h-6 px-2 text-xs" tip="Remove every notification" onClick={clear}>
                Clear all
              </Button>
            )}
          </header>
          {items.length === 0 ? (
            <EmptyState
              icon={BellOff}
              title="All quiet"
              body="Finished exports and other jobs show up here."
              compact
            />
          ) : (
            <ul className="overflow-y-auto">
              {items.map((item) => (
                <NotificationRow
                  key={item.id}
                  item={item}
                  onOpen={() => {
                    openResult(item);
                  }}
                  onRemove={() => {
                    remove(item.id);
                  }}
                />
              ))}
            </ul>
          )}
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}

function NotificationRow({
  item,
  onOpen,
  onRemove,
}: {
  item: AppNotification;
  onOpen: () => void;
  onRemove: () => void;
}) {
  const Icon = toneIcon[item.tone];
  return (
    <li className="group flex gap-2.5 border-b border-line px-3 py-2.5 last:border-b-0">
      <Icon className={clsx("mt-0.5 size-4 shrink-0", toneColor[item.tone])} aria-hidden />
      <div className="min-w-0 flex-1">
        <p className={clsx("text-sm", !item.read && "font-medium")}>{item.title}</p>
        {item.body && <p className="mt-0.5 text-xs text-ink-soft select-text">{item.body}</p>}
        <p className="mt-1 flex items-center gap-2 text-2xs text-ink-faint">
          {formatAgo(item.createdMs)}
          {item.path && (
            <button
              type="button"
              className="v-focus rounded text-accent hover:underline"
              onClick={onOpen}
            >
              Open
            </button>
          )}
        </p>
      </div>
      <IconButton
        label="Remove"
        className="size-5 opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
        onClick={onRemove}
      >
        <X className="size-3" />
      </IconButton>
    </li>
  );
}

/** the newest notifications, briefly, in the corner */
export function Toasts() {
  const toasts = useNotifications((state) => state.toasts);
  const items = useNotifications((state) => state.items);
  const shown = toasts
    .map((id) => items.find((item) => item.id === id))
    .filter((item): item is AppNotification => item !== undefined);

  return (
    <div
      aria-live="polite"
      className="pointer-events-none fixed right-4 bottom-4 z-[55] flex w-80 flex-col-reverse gap-2"
    >
      {shown.map((item) => (
        <Toast key={item.id} item={item} />
      ))}
    </div>
  );
}

function Toast({ item }: { item: AppNotification }) {
  const dismiss = useNotifications((state) => state.dismissToast);
  const [hovered, setHovered] = useState(false);
  const Icon = toneIcon[item.tone];

  useEffect(() => {
    if (hovered) return;
    const timer = setTimeout(() => {
      dismiss(item.id);
    }, TOAST_MS);
    return () => {
      clearTimeout(timer);
    };
  }, [hovered, item.id, dismiss]);

  return (
    <div
      role="status"
      onPointerEnter={() => {
        setHovered(true);
      }}
      onPointerLeave={() => {
        setHovered(false);
      }}
      className="v-toast pointer-events-auto flex gap-2.5 rounded-xl border border-line bg-raised p-3 shadow-modal"
    >
      <Icon className={clsx("mt-0.5 size-4 shrink-0", toneColor[item.tone])} aria-hidden />
      <button
        type="button"
        disabled={!item.path}
        onClick={() => {
          openResult(item);
          dismiss(item.id);
        }}
        className="v-focus min-w-0 flex-1 rounded text-left disabled:cursor-default"
      >
        <p className="text-sm font-medium">{item.title}</p>
        {item.body && <p className="mt-0.5 line-clamp-2 text-xs text-ink-soft">{item.body}</p>}
      </button>
      <IconButton
        label="Dismiss"
        className="size-5"
        onClick={() => {
          dismiss(item.id);
        }}
      >
        <X className="size-3" />
      </IconButton>
    </div>
  );
}

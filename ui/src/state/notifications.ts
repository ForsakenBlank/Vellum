import { create } from "zustand";

import type { Notification } from "../bindings/Notification";
import type { NotificationsChanged } from "../bindings/NotificationsChanged";

interface NotificationsStore {
  items: Notification[];
  unread: number;
  /** the newest id seen, so only notifications that are really new pop up as toasts */
  newestSeen: number;
  replace: (change: NotificationsChanged) => Notification[];
}

export const useNotifications = create<NotificationsStore>()((set, get) => ({
  items: [],
  unread: 0,
  newestSeen: 0,
  replace: (change) => {
    const { newestSeen } = get();
    const fresh = change.items.filter((n) => n.id > newestSeen && !n.read);
    const newest = Math.max(newestSeen, ...change.items.map((n) => n.id));
    set({ items: change.items, unread: change.unread, newestSeen: newest });
    return fresh;
  },
}));

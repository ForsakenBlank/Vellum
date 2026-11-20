// a quick tool session as its view sees it. rust owns the options (they are part of the
// document, so they undo, autosave and come back after a crash); the view reads them back
// after every history step and sends changes as `tool.set_options` commands.
//
// while a slider drags, each step goes to rust and merges into one undo step. the view
// shows its own copy of the change straight away so the slider never lags, and drops that
// copy once rust's answer has caught up.

import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { DocumentInfo } from "../../bindings/DocumentInfo";
import type { JobSnapshot } from "../../bindings/JobSnapshot";
import type { TargetSize } from "../../bindings/TargetSize";
import { inDesktopApp, ipc } from "../../lib/ipc";
import { fileKinds } from "../../lib/routing";
import { optionsFor, sessionJobs, type RunnableTool, type ToolOptions } from "../../lib/tools";
import { useDocuments } from "../../state/documents";
import { useJobs } from "../../state/jobs";
import { useSettings } from "../../state/settings";
import { useUi } from "../../state/ui";

/** a partial change to options, objects merged key by key, like rust's `merge_options` */
export type OptionsPatch<T> = {
  [K in keyof T]?: T[K] extends unknown[]
    ? T[K]
    : T[K] extends object | null
      ? OptionsPatch<NonNullable<T[K]>> | T[K]
      : T[K];
};

export function mergeOptions(target: unknown, patch: unknown): unknown {
  const isObject = (v: unknown): v is Record<string, unknown> =>
    typeof v === "object" && v !== null && !Array.isArray(v);
  if (!isObject(target) || !isObject(patch)) return patch;
  const out: Record<string, unknown> = { ...target };
  for (const [key, value] of Object.entries(patch)) {
    out[key] = key in target ? mergeOptions(target[key], value) : value;
  }
  return out;
}

export function fail(title: string) {
  return (err: unknown) => {
    useUi.getState().toast({ level: "error", title, body: String(err) });
  };
}

export interface Session<K extends RunnableTool> {
  options: ToolOptions[K];
  loaded: boolean;
  /** changes options. `seal` ends a drag, so the next change is a new undo step. */
  set: (patch: OptionsPatch<ToolOptions[K]>) => void;
  seal: () => void;
  inputs: string[];
  addFiles: () => Promise<void>;
  removeFile: (path: string) => void;
  run: () => Promise<void>;
  jobs: JobSnapshot[];
  running: boolean;
}

export function useSession<K extends RunnableTool>(doc: DocumentInfo, tool: K): Session<K> {
  const summary = useDocuments((s) => s.summaries[doc.id]);
  const [stored, setStored] = useState<unknown>(undefined);
  const [local, setLocal] = useState<unknown>(undefined);
  const [loaded, setLoaded] = useState(!inDesktopApp);
  const inflight = useRef(0);

  useEffect(() => {
    if (!inDesktopApp) return;
    let stale = false;
    ipc
      .getDocument(doc.id)
      .then((result) => {
        if (stale) return;
        const state = result.state as { options?: unknown } | null;
        setStored(state?.options);
        setLoaded(true);
        // rust has caught up with everything sent, so its copy is the truth again
        if (inflight.current === 0) setLocal(undefined);
      })
      .catch(() => {
        // the session closed while loading, the tab is about to go
      });
    return () => {
      stale = true;
    };
  }, [doc.id, summary]);

  const options = useMemo(
    () => optionsFor(tool, local === undefined ? stored : mergeOptions(stored ?? {}, local)),
    [tool, stored, local],
  );

  const set = useCallback(
    (patch: OptionsPatch<ToolOptions[K]>) => {
      setLocal((prev: unknown) => mergeOptions(prev ?? {}, patch));
      if (!inDesktopApp) return;
      inflight.current++;
      ipc
        .documentCommand(doc.id, "tool.set_options", { patch })
        .catch(fail("The change was not made"))
        .finally(() => {
          inflight.current--;
        });
    },
    [doc.id],
  );

  const seal = useCallback(() => {
    if (inDesktopApp) ipc.sealDocument(doc.id).catch(fail("The change was not saved"));
  }, [doc.id]);

  const addFiles = useCallback(async () => {
    if (!inDesktopApp) return;
    const extensions = fileKinds.flatMap((row) => row.extensions);
    const picked = await open({
      multiple: true,
      title: "Add files",
      filters: [{ name: "Media Vellum can open", extensions }],
    });
    if (picked && picked.length > 0) {
      await ipc
        .documentCommand(doc.id, "tool.add_inputs", { paths: picked })
        .catch(fail("The files were not added"));
    }
  }, [doc.id]);

  const removeFile = useCallback(
    (path: string) => {
      ipc
        .documentCommand(doc.id, "tool.remove_inputs", { paths: [path] })
        .catch(fail("The file was not removed"));
    },
    [doc.id],
  );

  const allJobs = useJobs((s) => s.jobs);
  const jobs = useMemo(() => sessionJobs(allJobs, doc.id), [allJobs, doc.id]);
  const running = jobs.some((job) => !["completed", "failed", "cancelled"].includes(job.state));

  const run = useCallback(async () => {
    try {
      await ipc.runQuickTool(doc.id);
    } catch (err) {
      fail("Nothing was started")(err);
    }
  }, [doc.id]);

  return {
    options,
    loaded,
    set,
    seal,
    inputs: doc.inputs,
    addFiles,
    removeFile,
    run,
    jobs,
    running,
  };
}

/** a value that only follows `value` once it has stopped changing for `ms` */
export function useDebounced<T>(value: T, ms: number): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const timer = window.setTimeout(() => {
      setSettled(value);
    }, ms);
    return () => {
      window.clearTimeout(timer);
    };
  }, [value, ms]);
  return settled;
}

export interface Latest<T> {
  value: T | null;
  error: string | null;
  loading: boolean;
}

/**
 * asks rust something about the current options, keeping only the newest answer. `key`
 * names the question, so a new key means a new question and the old answer counts as
 * loading until the new one arrives.
 */
export function useLatest<T>(ask: (() => Promise<T>) | null, key: string): Latest<T> {
  const [answer, setAnswer] = useState<{ key: string; value: T | null; error: string | null }>({
    key: "",
    value: null,
    error: null,
  });
  useEffect(() => {
    if (!ask || !inDesktopApp) return;
    let stale = false;
    ask()
      .then((value) => {
        if (!stale) setAnswer({ key, value, error: null });
      })
      .catch((err: unknown) => {
        if (!stale) setAnswer({ key, value: null, error: String(err) });
      });
    return () => {
      stale = true;
    };
    // the key names the question; `ask` is a new closure on every render
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  const asking = ask !== null && inDesktopApp;
  return {
    value: answer.value,
    error: answer.key === key ? answer.error : null,
    loading: asking && answer.key !== key,
  };
}

/** the size a session aims for, with settings filling the gap */
export function useTarget(target: TargetSize | null): TargetSize {
  const settings = useSettings((s) => s.settings?.export);
  return target ?? { amount: settings?.targetAmount ?? 10, unit: settings?.unit ?? "MB" };
}

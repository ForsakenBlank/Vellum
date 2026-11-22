import { Grid3x3, Magnet, PanelTop, Settings2, SquareDashed } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { DocumentInfo } from "../../bindings/DocumentInfo";
import type { ImageState } from "../../bindings/ImageState";
import type { PreviewRequest } from "../../bindings/PreviewRequest";
import { IconButton } from "../../components/Button";
import { Skeleton } from "../../components/Skeleton";
import { GpuViewport } from "../../components/viewport/GpuViewport";
import type { ViewportApi } from "../../components/viewport/tool";
import { ipc } from "../../lib/ipc";
import { levelForZoom } from "../../lib/viewport/camera";
import { registerUiHandlers } from "../../state/commands";
import { useDocuments } from "../../state/documents";
import { useUi } from "../../state/ui";
import { makeSnapper, withAids } from "./aids";
import { ColorRangeDialog, ModifyDialog, type ModifyOp } from "./dialogs";
import { type Thumb, cycle, findLayer, parseThumbnails, stepHardness, stepSize } from "./geometry";
import { LayersPanel } from "./LayersPanel";
import { Navigator } from "./Navigator";
import { OptionsBar } from "./OptionsBar";
import { useAids, useSessions } from "./session";
import { ChannelsPanel, ColorPanel } from "./SidePanels";
import { CanvasSizeDialog, ExportDialog, GridDialog, ImageSizeDialog } from "./sizeDialogs";
import { useImageTools } from "./store";
import { Toolbox } from "./Toolbox";
import { drawAnts, latest, makeTool } from "./tools";
import { useViewCamera } from "./viewCamera";
import type { Snap } from "./transformMath";
import {
  beginTransform,
  cancelTransform,
  commitTransform,
  cropTool,
  startCrop,
  transformSpec,
  transformTool,
} from "./transformTool";

type Pt = [number, number];
type Dialogs = "canvas" | "image" | "export" | "grid" | null;

function useImageState(doc: string): ImageState | null {
  const summary = useDocuments((s) => s.summaries[doc]);
  const [state, setState] = useState<ImageState | null>(null);
  useEffect(() => {
    let stale = false;
    ipc
      .getDocument(doc)
      .then((result) => {
        if (!stale) setState(result.state as ImageState);
      })
      .catch(() => {
        // the document closed while loading; its tab is about to go
      });
    return () => {
      stale = true;
    };
  }, [doc, summary]);
  return state;
}

export function ImageWorkspace({ doc }: { doc: DocumentInfo }) {
  const id = doc.id;
  const state = useImageState(id);
  const tools = useImageTools();
  const [ants, setAnts] = useState<Pt[][]>([]);
  const [level, setLevel] = useState(0);
  const [thumbs, setThumbs] = useState<Map<string, Thumb>>(new Map());
  const [modify, setModify] = useState<ModifyOp | null>(null);
  const [range, setRange] = useState(false);
  const [maskView, setMaskView] = useState<number | null>(null);
  const [view, setView] = useState<ViewportApi | null>(null);
  const [dialog, setDialog] = useState<Dialogs>(null);
  const aids = useAids();
  const transform = useSessions((s) => (s.transform?.doc === id ? s.transform : null));
  const crop = useSessions((s) => (s.crop?.doc === id ? s.crop : null));
  const transforming = transform !== null;
  const cropping = crop !== null;

  const fail = useCallback((err: unknown) => {
    useUi.getState().toast({ level: "error", title: "That did not work", body: String(err) });
  }, []);
  const stateRef = useRef(state);
  useEffect(() => {
    stateRef.current = state;
  }, [state]);

  // marching ants, refetched when the selection or the zoom level changes
  const selectionKey = state?.selectionBounds
    ? JSON.stringify(state.selectionBounds) + String(state.quickMask)
    : "";
  const showAnts = Boolean(state?.selection) && !state?.quickMask;
  useEffect(() => {
    if (!showAnts) return;
    let stale = false;
    ipc
      .imageOutline(id, level)
      .then((loops) => {
        if (!stale) setAnts(loops);
      })
      .catch(() => undefined);
    return () => {
      stale = true;
    };
    // the outline only depends on the selection, which the key stands for
  }, [id, level, selectionKey, showAnts]);

  // layer thumbnails, a moment after the last change so a stroke does not ask for many
  const layerKey = state
    ? JSON.stringify(state.layers.map((l) => [l.id, l.bounds, l.mask?.raster.file])) +
      JSON.stringify(state.layers)
    : "";
  useEffect(() => {
    if (!state || state.loading) return;
    const wanted: [number, boolean][] = [];
    const walk = (list: typeof state.layers) => {
      for (const l of list) {
        if (!l.group) wanted.push([l.id, false]);
        if (l.mask) wanted.push([l.id, true]);
        walk(l.children);
      }
    };
    walk(state.layers);
    let stale = false;
    const timer = window.setTimeout(() => {
      ipc
        .imageThumbnails(id, wanted, 64)
        .then((buf) => {
          if (!stale) setThumbs(parseThumbnails(buf));
        })
        .catch(() => undefined);
    }, 250);
    return () => {
      stale = true;
      window.clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id, layerKey]);

  // one preview request carries both the mask view and a free transform in progress
  const sendPreview = useMemo(
    () => latest((request: PreviewRequest) => ipc.imagePreview(id, request).catch(() => undefined)),
    [id],
  );
  useEffect(() => {
    sendPreview({
      offset: null,
      showMask: maskView,
      transform:
        transform && transform.target === "layer"
          ? { target: transform.target, spec: transformSpec(transform) }
          : null,
    });
    view?.redraw();
  }, [sendPreview, maskView, transform, view]);

  // the crop box follows the canvas, and leaves with the crop tool
  const cropKind = tools.cropKind;
  const width = state?.width;
  const height = state?.height;
  useEffect(() => {
    const sessions = useSessions.getState();
    if (tools.tool !== "crop" || !stateRef.current || stateRef.current.loading) {
      if (sessions.crop?.doc === id) sessions.setCrop(null);
      return;
    }
    sessions.setCrop(startCrop(id, stateRef.current, cropKind === "perspective"));
  }, [id, tools.tool, cropKind, width, height]);
  useEffect(() => {
    view?.redraw();
  }, [crop, view]);

  // picking another tool applies a free transform, like photopea
  const toolId = tools.tool;
  useEffect(() => {
    if (useSessions.getState().transform?.doc === id) void commitTransform(fail);
  }, [toolId, id, fail]);

  const run = useCallback(
    (kind: string, params: unknown = {}) => {
      ipc.imageCommand(id, kind, params).catch(fail);
    },
    [id, fail],
  );

  // the image editor's commands, for shortcuts, menus and the palette
  useEffect(() => {
    const t = () => useImageTools.getState();
    const set = (patch: Parameters<ReturnType<typeof t>["set"]>[0]) => {
      t().set(patch);
    };
    const info = (title: string, body: string) => {
      useUi.getState().toast({ level: "info", title, body });
    };
    // anything that changes the canvas first drops a transform in progress
    const reshape = (kind: string, params: unknown) => {
      cancelTransform();
      run(kind, params);
    };
    const aid = useAids.getState;
    const active = () => {
      const s = stateRef.current;
      return s && s.active !== null ? findLayer(s.layers, s.active) : undefined;
    };
    const brushPatch = (patch: { size?: number; hardness?: number }) => {
      t().setBrush(patch);
    };
    const current = () => (t().tool === "eraser" ? t().eraser : t().brush);
    return registerUiHandlers({
      "image.tool.move": () => {
        set({ tool: "move" });
      },
      "image.tool.marquee": () => {
        set({ tool: "marquee" });
      },
      "image.tool.marquee_cycle": () => {
        set({ tool: "marquee", marquee: cycle(["rect", "ellipse"] as const, t().marquee) });
      },
      "image.tool.lasso": () => {
        set({ tool: "lasso" });
      },
      "image.tool.lasso_cycle": () => {
        set({ tool: "lasso", lasso: cycle(["free", "polygon", "magnetic"] as const, t().lasso) });
      },
      "image.tool.wand": () => {
        set({ tool: "wand" });
      },
      "image.tool.wand_cycle": () => {
        set({ tool: "wand", wand: cycle(["wand", "quick"] as const, t().wand) });
      },
      "image.tool.eyedropper": () => {
        set({ tool: "eyedropper" });
      },
      "image.tool.brush": () => {
        set({ tool: "brush" });
      },
      "image.tool.brush_cycle": () => {
        set({ tool: "brush", brushKind: cycle(["brush", "pencil"] as const, t().brushKind) });
      },
      "image.tool.eraser": () => {
        set({ tool: "eraser" });
      },
      "image.tool.gradient": () => {
        set({ tool: "gradient" });
      },
      "image.tool.gradient_cycle": () => {
        set({ tool: "gradient", fill: cycle(["gradient", "bucket"] as const, t().fill) });
      },
      "image.tool.hand": () => {
        set({ tool: "hand" });
      },
      "image.tool.zoom": () => {
        set({ tool: "zoom" });
      },
      "image.tool.crop": () => {
        set({ tool: "crop" });
      },
      "image.tool.crop_cycle": () => {
        set({ tool: "crop", cropKind: cycle(["crop", "perspective"] as const, t().cropKind) });
      },
      "image.transform": () => {
        if (!useSessions.getState().transform) void beginTransform(id, "layer", fail);
      },
      "image.select.transform": () => {
        if (!stateRef.current?.selection) {
          info("Nothing is selected", "Select something first, then transform the selection.");
          return;
        }
        if (!useSessions.getState().transform) void beginTransform(id, "selection", fail);
      },
      "image.canvas_size": () => {
        setDialog("canvas");
      },
      "image.image_size": () => {
        setDialog("image");
      },
      "image.crop_to_selection": () => {
        const b = stateRef.current?.selectionBounds;
        if (!stateRef.current?.selection || !b || b.w === 0) {
          info("Nothing is selected", "Select the part to keep, then crop to the selection.");
          return;
        }
        reshape("image.crop", { rect: [b.x, b.y, b.w, b.h], deleteOutside: true });
      },
      "image.rotate_right": () => {
        reshape("image.canvas.orient", { op: "rotate90" });
      },
      "image.rotate_left": () => {
        reshape("image.canvas.orient", { op: "rotate270" });
      },
      "image.rotate_180": () => {
        reshape("image.canvas.orient", { op: "rotate180" });
      },
      "image.flip_canvas_h": () => {
        reshape("image.canvas.orient", { op: "flipHorizontal" });
      },
      "image.flip_canvas_v": () => {
        reshape("image.canvas.orient", { op: "flipVertical" });
      },
      "image.flip_layer_h": () => {
        reshape("image.layer.orient", { op: "flipHorizontal" });
      },
      "image.flip_layer_v": () => {
        reshape("image.layer.orient", { op: "flipVertical" });
      },
      "image.export": () => {
        setDialog("export");
      },
      "image.view.rulers": () => {
        aid().toggle("rulers");
      },
      "image.view.grid": () => {
        aid().toggle("grid");
      },
      "image.view.guides": () => {
        aid().toggle("guides");
      },
      "image.view.snap": () => {
        aid().toggle("snap");
      },
      "image.view.lock_guides": () => {
        aid().toggle("lockGuides");
        info(
          aid().lockGuides ? "Guides locked" : "Guides unlocked",
          aid().lockGuides
            ? "Guides stay where they are until you unlock them."
            : "Guides can be dragged again with the move tool.",
        );
      },
      "image.view.clear_guides": () => {
        if ((stateRef.current?.guides.length ?? 0) === 0) {
          info("No guides", "Drag from a ruler to add a guide. Ctrl+R shows the rulers.");
          return;
        }
        run("image.guide.clear", {});
      },
      "image.view.fit": () => view?.fit(),
      "image.view.actual_pixels": () => view?.actualPixels(),
      "image.view.zoom_in": () => view?.zoomStep(1),
      "image.view.zoom_out": () => view?.zoomStep(-1),
      "image.select.all": () => {
        run("image.select.op", { op: "all" });
      },
      "image.select.deselect": () => {
        run("image.select.op", { op: "none" });
      },
      "image.select.reselect": () => {
        run("image.select.op", { op: "reselect" });
      },
      "image.select.invert": () => {
        run("image.select.op", { op: "invert" });
      },
      "image.select.feather": () => {
        setModify("feather");
      },
      "image.select.expand": () => {
        setModify("expand");
      },
      "image.select.contract": () => {
        setModify("contract");
      },
      "image.select.border": () => {
        setModify("border");
      },
      "image.select.smooth": () => {
        setModify("smooth");
      },
      "image.select.color_range": () => {
        setRange(true);
      },
      "image.select.save": () => {
        run("image.select.save", {});
      },
      "image.select.load": () => {
        const first = stateRef.current?.channels[0];
        if (first) run("image.select.load", { channel: first.id });
        else
          useUi.getState().toast({
            level: "info",
            title: "No saved selections",
            body: "Save a selection first, then load it from the panel on the right.",
          });
      },
      "image.quick_mask": () => {
        run("image.quick_mask", { on: !stateRef.current?.quickMask });
      },
      "image.colors.default": () => {
        set({ foreground: [0, 0, 0], background: [1, 1, 1] });
      },
      "image.colors.swap": () => {
        set({ foreground: t().background, background: t().foreground });
      },
      "image.brush.smaller": () => {
        brushPatch({ size: stepSize(current().size, -1) });
      },
      "image.brush.bigger": () => {
        brushPatch({ size: stepSize(current().size, 1) });
      },
      "image.brush.softer": () => {
        brushPatch({ hardness: stepHardness(current().hardness, -1) });
      },
      "image.brush.harder": () => {
        brushPatch({ hardness: stepHardness(current().hardness, 1) });
      },
      "image.edit.clear": () => {
        run("image.paint.fill", { color: null });
      },
      "image.edit.fill_foreground": () => {
        run("image.paint.fill", { color: t().foreground });
      },
      "image.edit.fill_background": () => {
        run("image.paint.fill", { color: t().background });
      },
      "image.layer.new": () => {
        run("image.layer.add");
      },
      "image.layer.via_copy": () => {
        run("image.layer.via_copy", { cut: false });
      },
      "image.layer.via_cut": () => {
        run("image.layer.via_copy", { cut: true });
      },
      "image.layer.duplicate": () => {
        run("image.layer.duplicate");
      },
      "image.layer.delete": () => {
        run("image.layer.remove");
      },
      "image.layer.group": () => {
        run("image.layer.group");
      },
      "image.layer.ungroup": () => {
        run("image.layer.ungroup");
      },
      "image.layer.clip": () => {
        const l = active();
        if (l) run("image.layer.props", { id: l.id, clipped: !l.clipped });
      },
      "image.layer.merge_down": () => {
        run("image.layer.merge_down");
      },
      "image.layer.merge_visible": () => {
        run("image.layer.merge_visible", { flatten: false });
      },
      "image.layer.flatten": () => {
        run("image.layer.merge_visible", { flatten: true });
      },
      "image.layer.add_mask": () => {
        run("image.mask.add", { reveal: true });
      },
    });
  }, [run, view, id, fail]);

  const closeDialog = useCallback(() => {
    setDialog(null);
  }, []);

  const pickColor = useCallback((rgb: [number, number, number], background: boolean) => {
    useImageTools.getState().set(background ? { background: rgb } : { foreground: rgb });
  }, []);

  // a plain holder rather than a ref, since tools built during render keep it
  const [lines] = useState<{ current: Snap["lines"] }>(() => ({ current: [] }));
  const tool = useMemo(() => {
    const env = {
      doc: id,
      state,
      aids,
      canDragGuides: tools.tool === "move" && !transforming,
      lines,
      fail,
    };
    // the layer being moved never snaps to itself
    const moving = tools.tool === "move" && state?.active != null ? [state.active] : [];
    const snap = makeSnapper(env, moving);
    const shown = showAnts ? ants : [];
    const base = transforming
      ? transformTool({ ants: shown, snap, fail, drawAnts })
      : cropping && state
        ? cropTool({ state, snap, fail })
        : makeTool({ doc: id, state, tools, view, fail, pickColor, snap }, shown);
    return withAids(base, env);
  }, [
    id,
    state,
    tools,
    view,
    fail,
    pickColor,
    ants,
    showAnts,
    aids,
    transforming,
    cropping,
    lines,
  ]);

  if (!state || state.loading !== null) {
    return (
      <div className="flex h-full flex-col gap-3 p-6" aria-busy aria-label="Opening the picture">
        <p className="text-sm text-fg-muted">
          {state?.loading ? `Opening ${doc.title}…` : "Loading…"}
        </p>
        <Skeleton className="w-full flex-1 rounded-xl" />
      </div>
    );
  }

  const active = state.active === null ? undefined : findLayer(state.layers, state.active);
  return (
    <div className="flex h-full min-h-0 flex-col">
      <OptionsBar state={state} fail={fail} />
      <div className="flex min-h-0 flex-1">
        <Toolbox
          quickMask={state.quickMask}
          onQuickMask={() => {
            run("image.quick_mask", { on: !state.quickMask });
          }}
        />
        <div className="flex min-w-0 flex-1 flex-col p-2">
          <GpuViewport
            document={id}
            title={doc.title}
            tool={tool}
            onReady={setView}
            onZoom={(zoom) => {
              setLevel(levelForZoom(zoom));
            }}
            onCamera={(camera, size) => {
              useViewCamera.getState().set(id, camera, size);
            }}
          />
          <div className="flex items-center gap-0.5 pt-1">
            <IconButton
              size="sm"
              label="Rulers. Drag from one to add a guide"
              command="image.view.rulers"
              active={aids.rulers}
              icon={<PanelTop className="size-3.5" />}
              onClick={() => {
                aids.toggle("rulers");
              }}
            />
            <IconButton
              size="sm"
              label="Grid"
              command="image.view.grid"
              active={aids.grid}
              icon={<Grid3x3 className="size-3.5" />}
              onClick={() => {
                aids.toggle("grid");
              }}
            />
            <IconButton
              size="sm"
              label="Guides"
              command="image.view.guides"
              active={aids.guides}
              icon={<SquareDashed className="size-3.5" />}
              onClick={() => {
                aids.toggle("guides");
              }}
            />
            <IconButton
              size="sm"
              label="Snap to guides, the grid, the canvas and other layers"
              command="image.view.snap"
              active={aids.snap}
              icon={<Magnet className="size-3.5" />}
              onClick={() => {
                aids.toggle("snap");
              }}
            />
            <IconButton
              size="sm"
              label="Grid size and what snaps"
              icon={<Settings2 className="size-3.5" />}
              onClick={() => {
                setDialog("grid");
              }}
            />
            <p className="min-w-0 flex-1 truncate px-1 text-2xs text-fg-subtle" aria-live="polite">
              {`${String(state.width)} × ${String(state.height)} px · ${state.depth === "u8" ? "8" : state.depth === "u16" ? "16" : "32"} bit`}
              {state.profile ? ` · ${state.profile}` : ""}
              {active ? ` · ${active.name}${state.target === "mask" ? " (mask)" : ""}` : ""}
              {state.quickMask ? " · Quick mask: paint white to select, black to deselect" : ""}
              {maskView !== null
                ? " · Showing a mask on its own, Alt click it again to go back"
                : ""}
              {transforming ? " · Enter applies the transform, Esc cancels" : ""}
              {cropping ? " · Enter crops, Esc starts over" : ""}
            </p>
          </div>
        </div>
        <aside
          aria-label="Image panels"
          className="flex w-72 shrink-0 flex-col border-l border-line bg-panel"
        >
          <Navigator
            doc={id}
            width={state.width}
            height={state.height}
            revision={layerKey}
            view={view}
          />
          <ColorPanel />
          <LayersPanel
            doc={id}
            state={state}
            thumbs={thumbs}
            fail={fail}
            maskView={maskView}
            setMaskView={setMaskView}
          />
          <ChannelsPanel doc={id} state={state} fail={fail} />
        </aside>
      </div>
      <ModifyDialog
        key={modify ?? "none"}
        doc={id}
        op={modify}
        onClose={() => {
          setModify(null);
        }}
        fail={fail}
      />
      <ColorRangeDialog
        doc={id}
        open={range}
        onClose={() => {
          setRange(false);
        }}
        fail={fail}
      />
      <CanvasSizeDialog
        key={`canvas-${String(state.width)}x${String(state.height)}`}
        doc={id}
        state={state}
        open={dialog === "canvas"}
        onClose={closeDialog}
        fail={fail}
      />
      <ImageSizeDialog
        key={`image-${String(state.width)}x${String(state.height)}`}
        doc={id}
        state={state}
        open={dialog === "image"}
        onClose={closeDialog}
        fail={fail}
      />
      <ExportDialog
        doc={id}
        title={doc.title}
        state={state}
        open={dialog === "export"}
        onClose={closeDialog}
        fail={fail}
      />
      <GridDialog open={dialog === "grid"} onClose={closeDialog} />
    </div>
  );
}

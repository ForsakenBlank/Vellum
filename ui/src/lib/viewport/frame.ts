// reads the binary frames rust sends, see `pack` in crates/vellum-gpu/src/view.rs

import type { TileId } from "../../bindings/TileId";

export interface TileRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface FrameTile {
  id: TileId;
  /** the pixels' rectangle at the tile's level, gutter included */
  rect: TileRect;
  /** premultiplied rgba8 in the display's encoding */
  pixels: Uint8Array;
}

export interface Frame {
  tiles: FrameTile[];
  dropped: TileId[];
  pending: number;
  level: number;
}

const MAGIC = "VTL1";

export function parseFrame(buffer: ArrayBuffer): Frame {
  const view = new DataView(buffer);
  if (buffer.byteLength < 20) throw new Error("The picture frame was cut short.");
  const magic = String.fromCharCode(
    view.getUint8(0),
    view.getUint8(1),
    view.getUint8(2),
    view.getUint8(3),
  );
  if (magic !== MAGIC) throw new Error("The picture frame is in a format this window cannot read.");
  const count = view.getUint32(4, true);
  const droppedCount = view.getUint32(8, true);
  const pending = view.getUint32(12, true);
  const level = view.getUint32(16, true);
  let at = 20;
  const dropped: TileId[] = [];
  for (let i = 0; i < droppedCount; i++) {
    dropped.push({
      level: view.getUint32(at, true),
      col: view.getInt32(at + 4, true),
      row: view.getInt32(at + 8, true),
    });
    at += 12;
  }
  const tiles: FrameTile[] = [];
  for (let i = 0; i < count; i++) {
    const id = {
      level: view.getUint32(at, true),
      col: view.getInt32(at + 4, true),
      row: view.getInt32(at + 8, true),
    };
    const rect = {
      x: view.getInt32(at + 12, true),
      y: view.getInt32(at + 16, true),
      w: view.getUint32(at + 20, true),
      h: view.getUint32(at + 24, true),
    };
    at += 28;
    const bytes = rect.w * rect.h * 4;
    if (at + bytes > buffer.byteLength) throw new Error("The picture frame was cut short.");
    tiles.push({ id, rect, pixels: new Uint8Array(buffer, at, bytes) });
    at += bytes;
  }
  return { tiles, dropped, pending, level };
}

export function tileKey(id: TileId): string {
  return `${String(id.level)}/${String(id.col)}/${String(id.row)}`;
}

/** writes a frame, for tests and the e2e backend */
export function packFrame(frame: Frame): ArrayBuffer {
  const size =
    20 + frame.dropped.length * 12 + frame.tiles.reduce((n, t) => n + 28 + t.pixels.byteLength, 0);
  const buffer = new ArrayBuffer(size);
  const view = new DataView(buffer);
  for (let i = 0; i < MAGIC.length; i++) view.setUint8(i, MAGIC.charCodeAt(i));
  view.setUint32(4, frame.tiles.length, true);
  view.setUint32(8, frame.dropped.length, true);
  view.setUint32(12, frame.pending, true);
  view.setUint32(16, frame.level, true);
  let at = 20;
  for (const d of frame.dropped) {
    view.setUint32(at, d.level, true);
    view.setInt32(at + 4, d.col, true);
    view.setInt32(at + 8, d.row, true);
    at += 12;
  }
  for (const t of frame.tiles) {
    view.setUint32(at, t.id.level, true);
    view.setInt32(at + 4, t.id.col, true);
    view.setInt32(at + 8, t.id.row, true);
    view.setInt32(at + 12, t.rect.x, true);
    view.setInt32(at + 16, t.rect.y, true);
    view.setUint32(at + 20, t.rect.w, true);
    view.setUint32(at + 24, t.rect.h, true);
    at += 28;
    new Uint8Array(buffer, at, t.pixels.byteLength).set(t.pixels);
    at += t.pixels.byteLength;
  }
  return buffer;
}

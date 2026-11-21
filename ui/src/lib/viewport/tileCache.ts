// the display tiles a window holds, as gpu textures, with a byte budget. tiles that fall out
// are reported back to rust so it sends them again when they are needed.

import type { TileId } from "../../bindings/TileId";
import type { TileRect } from "./frame";
import { tileKey } from "./frame";

export interface Tile<T> {
  id: TileId;
  /** the texture's rectangle at the tile's level, gutter included */
  rect: TileRect;
  texture: T;
  bytes: number;
  usedAt: number;
}

export class TileCache<T> {
  private tiles = new Map<string, Tile<T>>();
  private bytes = 0;
  private tick = 0;

  constructor(
    private readonly budget: number,
    private readonly release: (texture: T) => void,
  ) {}

  get size(): number {
    return this.tiles.size;
  }

  get usedBytes(): number {
    return this.bytes;
  }

  get(id: TileId): Tile<T> | undefined {
    return this.tiles.get(tileKey(id));
  }

  /** stores a tile, replacing one with the same id. returns tiles evicted to make room. */
  put(id: TileId, rect: TileRect, texture: T, bytes: number, keep: Set<string>): TileId[] {
    const key = tileKey(id);
    const old = this.tiles.get(key);
    if (old) {
      if (old.texture !== texture) this.release(old.texture);
      this.bytes -= old.bytes;
    }
    this.tiles.set(key, { id, rect, texture, bytes, usedAt: ++this.tick });
    this.bytes += bytes;
    return this.trim(keep);
  }

  /** marks tiles as just drawn, so they are the last to go */
  touch(id: TileId) {
    const tile = this.tiles.get(tileKey(id));
    if (tile) tile.usedAt = ++this.tick;
  }

  drop(id: TileId) {
    const key = tileKey(id);
    const tile = this.tiles.get(key);
    if (!tile) return;
    this.release(tile.texture);
    this.bytes -= tile.bytes;
    this.tiles.delete(key);
  }

  clear() {
    for (const tile of this.tiles.values()) this.release(tile.texture);
    this.tiles.clear();
    this.bytes = 0;
  }

  /** every tile, coarsest level first, so finer tiles draw on top */
  drawOrder(): Tile<T>[] {
    return [...this.tiles.values()].sort((a, b) => b.id.level - a.id.level);
  }

  /** drops the least recently used tiles until under budget, never ones in `keep` */
  private trim(keep: Set<string>): TileId[] {
    if (this.bytes <= this.budget) return [];
    const evicted: TileId[] = [];
    const order = [...this.tiles.entries()].sort((a, b) => a[1].usedAt - b[1].usedAt);
    for (const [key, tile] of order) {
      if (this.bytes <= this.budget * 0.9) break;
      if (keep.has(key)) continue;
      this.release(tile.texture);
      this.bytes -= tile.bytes;
      this.tiles.delete(key);
      evicted.push(tile.id);
    }
    return evicted;
  }
}

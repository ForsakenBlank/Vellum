// draws display tiles with webgl2. rust renders the tiles; this only places them, so pan,
// zoom and rotate run at the display's refresh rate whatever the picture costs to render.

import type { Camera, Size } from "./camera";
import { clipMatrix } from "./camera";
import type { Tile } from "./tileCache";

/** device pixels per side of a transparency checker square */
const CHECKER = 8;
/** the pixel grid shows from this zoom up */
export const PIXEL_GRID_ZOOM = 8;
/** tiles magnified at least this much are drawn with hard pixel edges */
const NEAREST_FROM = 2;

export interface Palette {
  /** around the picture */
  outside: [number, number, number];
  checkerA: [number, number, number];
  checkerB: [number, number, number];
  grid: [number, number, number, number];
}

const QUAD_VS = `#version 300 es
in vec2 a_pos;
uniform mat3 u_matrix;
uniform vec4 u_rect;
uniform vec4 u_tex;
out vec2 v_uv;
void main() {
  vec2 p = u_rect.xy + a_pos * u_rect.zw;
  v_uv = mix(u_tex.xy, u_tex.zw, a_pos);
  vec3 c = u_matrix * vec3(p, 1.0);
  gl_Position = vec4(c.xy, 0.0, 1.0);
}`;

const TILE_FS = `#version 300 es
precision highp float;
uniform sampler2D u_image;
in vec2 v_uv;
out vec4 color;
void main() {
  color = texture(u_image, v_uv);
}`;

const SCREEN_VS = `#version 300 es
in vec2 a_pos;
void main() {
  gl_Position = vec4(a_pos * 2.0 - 1.0, 0.0, 1.0);
}`;

// the backdrop: a checkerboard inside the document, a plain colour outside, and the pixel
// grid on top when zoomed in far enough
const BACKDROP_FS = `#version 300 es
precision highp float;
uniform mat3 u_inverse;
uniform vec2 u_size;
uniform vec2 u_doc;
uniform vec3 u_outside;
uniform vec3 u_checker_a;
uniform vec3 u_checker_b;
uniform float u_checker;
out vec4 color;
void main() {
  vec2 clip = gl_FragCoord.xy / u_size * 2.0 - 1.0;
  vec2 doc = (u_inverse * vec3(clip, 1.0)).xy;
  if (doc.x < 0.0 || doc.y < 0.0 || doc.x >= u_doc.x || doc.y >= u_doc.y) {
    color = vec4(u_outside, 1.0);
    return;
  }
  vec2 cell = floor(vec2(gl_FragCoord.x, u_size.y - gl_FragCoord.y) / u_checker);
  bool odd = mod(cell.x + cell.y, 2.0) > 0.5;
  color = vec4(odd ? u_checker_b : u_checker_a, 1.0);
}`;

const GRID_FS = `#version 300 es
precision highp float;
uniform mat3 u_inverse;
uniform vec2 u_size;
uniform vec2 u_doc;
uniform vec4 u_grid;
out vec4 color;
void main() {
  vec2 clip = gl_FragCoord.xy / u_size * 2.0 - 1.0;
  vec2 doc = (u_inverse * vec3(clip, 1.0)).xy;
  if (doc.x < 0.0 || doc.y < 0.0 || doc.x > u_doc.x || doc.y > u_doc.y) discard;
  vec2 d = abs(fract(doc + 0.5) - 0.5) / fwidth(doc);
  float line = 1.0 - clamp(min(d.x, d.y), 0.0, 1.0);
  color = vec4(u_grid.rgb, 1.0) * u_grid.a * line;
}`;

function compile(gl: WebGL2RenderingContext, type: number, source: string): WebGLShader {
  const shader = gl.createShader(type);
  if (!shader) throw new Error("The graphics card refused to make a shader.");
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (gl.isContextLost()) throw new Error("The graphics card is resetting. Try again in a moment.");
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    const log = gl.getShaderInfoLog(shader) ?? "";
    gl.deleteShader(shader);
    throw new Error(`A viewport shader did not compile: ${log}`);
  }
  return shader;
}

interface Program {
  program: WebGLProgram;
  uniforms: Record<string, WebGLUniformLocation | null>;
}

function link(gl: WebGL2RenderingContext, vs: string, fs: string, names: string[]): Program {
  const program = gl.createProgram();
  gl.attachShader(program, compile(gl, gl.VERTEX_SHADER, vs));
  gl.attachShader(program, compile(gl, gl.FRAGMENT_SHADER, fs));
  gl.bindAttribLocation(program, 0, "a_pos");
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    throw new Error(`A viewport shader did not link: ${gl.getProgramInfoLog(program) ?? ""}`);
  }
  const uniforms: Record<string, WebGLUniformLocation | null> = {};
  for (const name of names) uniforms[name] = gl.getUniformLocation(program, name);
  return { program, uniforms };
}

/** the inverse of a column major 3x3 matrix */
export function invert3(m: Float32Array): Float32Array {
  const [a = 1, b = 0, c = 0, d = 0, e = 1, f = 0, g = 0, h = 0, i = 1] = m;
  const A = e * i - f * h;
  const B = -(d * i - f * g);
  const C = d * h - e * g;
  const det = a * A + b * B + c * C;
  if (Math.abs(det) < 1e-20) return new Float32Array([1, 0, 0, 0, 1, 0, 0, 0, 1]);
  const k = 1 / det;
  return new Float32Array([
    A * k,
    -(b * i - c * h) * k,
    (b * f - c * e) * k,
    B * k,
    (a * i - c * g) * k,
    -(a * f - c * d) * k,
    C * k,
    -(a * h - b * g) * k,
    (a * e - b * d) * k,
  ]);
}

export interface DrawStats {
  tiles: number;
  cpuMs: number;
}

export class TileRenderer {
  readonly gl: WebGL2RenderingContext;
  private tile: Program;
  private backdrop: Program;
  private grid: Program;
  private quad: WebGLVertexArrayObject;
  private buffer: WebGLBuffer;

  constructor(canvas: HTMLCanvasElement) {
    const gl = canvas.getContext("webgl2", {
      alpha: false,
      antialias: false,
      depth: false,
      stencil: false,
      premultipliedAlpha: true,
      preserveDrawingBuffer: false,
      powerPreference: "high-performance",
    });
    if (!gl) throw new Error("This window cannot draw with the graphics card (no WebGL 2).");
    this.gl = gl;
    this.tile = link(gl, QUAD_VS, TILE_FS, ["u_matrix", "u_rect", "u_tex", "u_image"]);
    const screen = ["u_inverse", "u_size", "u_doc"];
    this.backdrop = link(gl, SCREEN_VS, BACKDROP_FS, [
      ...screen,
      "u_outside",
      "u_checker_a",
      "u_checker_b",
      "u_checker",
    ]);
    this.grid = link(gl, SCREEN_VS, GRID_FS, [...screen, "u_grid"]);
    const vao = gl.createVertexArray();
    gl.bindVertexArray(vao);
    const buffer = gl.createBuffer();
    this.buffer = buffer;
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    this.quad = vao;
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
  }

  /** a texture holding one tile's premultiplied rgba8 pixels */
  upload(width: number, height: number, pixels: Uint8Array): WebGLTexture {
    const gl = this.gl;
    const texture = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, width, height, 0, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
    return texture;
  }

  release(texture: WebGLTexture) {
    this.gl.deleteTexture(texture);
  }

  /**
   * frees the programs and buffers. the context itself is left alone: react may mount
   * the same canvas again (strict mode does), and a context lost on purpose cannot be
   * used again. a canvas that leaves the page takes its context with it.
   */
  dispose() {
    const gl = this.gl;
    for (const p of [this.tile, this.backdrop, this.grid]) gl.deleteProgram(p.program);
    gl.deleteBuffer(this.buffer);
    gl.deleteVertexArray(this.quad);
  }

  /** draws one frame. `tiles` must be coarsest first so finer ones land on top. */
  draw(
    cam: Camera,
    size: Size,
    doc: Size,
    tiles: Tile<WebGLTexture>[],
    palette: Palette,
    pixelGrid: boolean,
  ): DrawStats {
    const start = performance.now();
    const gl = this.gl;
    gl.viewport(0, 0, size.width, size.height);
    gl.bindVertexArray(this.quad);
    const matrix = clipMatrix(cam, size);
    const inverse = invert3(matrix);

    gl.disable(gl.BLEND);
    const b = this.backdrop;
    gl.useProgram(b.program);
    gl.uniformMatrix3fv(b.uniforms.u_inverse ?? null, false, inverse);
    gl.uniform2f(b.uniforms.u_size ?? null, size.width, size.height);
    gl.uniform2f(b.uniforms.u_doc ?? null, doc.width, doc.height);
    gl.uniform3fv(b.uniforms.u_outside ?? null, palette.outside);
    gl.uniform3fv(b.uniforms.u_checker_a ?? null, palette.checkerA);
    gl.uniform3fv(b.uniforms.u_checker_b ?? null, palette.checkerB);
    gl.uniform1f(b.uniforms.u_checker ?? null, CHECKER * (window.devicePixelRatio || 1));
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);

    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    const t = this.tile;
    gl.useProgram(t.program);
    gl.uniformMatrix3fv(t.uniforms.u_matrix ?? null, false, matrix);
    gl.uniform1i(t.uniforms.u_image ?? null, 0);
    gl.activeTexture(gl.TEXTURE0);
    let drawn = 0;
    for (const tile of tiles) {
      const scale = 2 ** tile.id.level;
      // the tile proper, without its gutter, clipped to the document
      const tx = tile.id.col * 256;
      const ty = tile.id.row * 256;
      const levelW = Math.ceil(doc.width / scale);
      const levelH = Math.ceil(doc.height / scale);
      const x0 = Math.max(tx, 0);
      const y0 = Math.max(ty, 0);
      const x1 = Math.min(tx + 256, levelW);
      const y1 = Math.min(ty + 256, levelH);
      if (x1 <= x0 || y1 <= y0) continue;
      const r = tile.rect;
      gl.bindTexture(gl.TEXTURE_2D, tile.texture);
      const filter = cam.zoom * scale >= NEAREST_FROM ? gl.NEAREST : gl.LINEAR;
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter);
      // document pixels, clamped to the document so the last partial pixel stays inside
      const dx0 = x0 * scale;
      const dy0 = y0 * scale;
      const dx1 = Math.min(x1 * scale, doc.width);
      const dy1 = Math.min(y1 * scale, doc.height);
      gl.uniform4f(t.uniforms.u_rect ?? null, dx0, dy0, dx1 - dx0, dy1 - dy0);
      gl.uniform4f(
        t.uniforms.u_tex ?? null,
        (x0 - r.x) / r.w,
        (y0 - r.y) / r.h,
        (dx1 / scale - r.x) / r.w,
        (dy1 / scale - r.y) / r.h,
      );
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
      drawn++;
    }

    if (pixelGrid && cam.zoom >= PIXEL_GRID_ZOOM) {
      const g = this.grid;
      gl.useProgram(g.program);
      gl.uniformMatrix3fv(g.uniforms.u_inverse ?? null, false, inverse);
      gl.uniform2f(g.uniforms.u_size ?? null, size.width, size.height);
      gl.uniform2f(g.uniforms.u_doc ?? null, doc.width, doc.height);
      gl.uniform4fv(g.uniforms.u_grid ?? null, palette.grid);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    }
    return { tiles: drawn, cpuMs: performance.now() - start };
  }
}

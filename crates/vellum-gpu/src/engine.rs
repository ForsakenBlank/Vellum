//! renders graphs: picks the gpu or the cpu, walks the graph, caches results by content and
//! recovers when the gpu goes away.
//!
//! a render asks for one node's pixels over a rectangle at a mip level. the walk computes
//! each node's bounds once, clips every request to them (so empty areas cost nothing),
//! hashes a cache key per node and region, and only runs nodes whose key is not cached.
//! results stay on the device they were made on: gpu textures in the gpu cache, cpu images
//! in the cpu cache, each with its own byte budget.
//!
//! the engine is `Send + Sync` and serialises renders with one lock. wgpu's device and
//! queue are thread safe, so phase 8's decode threads can upload frames on their own
//! threads; only graph walks take the lock.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rayon::prelude::*;
use serde::Serialize;
use ts_rs::TS;

use crate::cache::Lru;
use crate::color::{ColorTransform, WorkingSpace};
use crate::gpu::{
    Dispatch, GpuContext, GpuError, GpuExec, GpuImage, GpuOptions, Kernel, StorageFormat,
};
use crate::graph::{Graph, Node, NodeCx, NodeId, Out, RenderError};
use crate::image::{CpuImage, Precision};
use crate::params::Params;
use crate::rect::{Bounds, IRect};

/// the most pixels one node may cover in a single render, about 268 megapixels. bigger
/// requests must go through [`crate::tiles::render_tiled`].
pub const MAX_REGION_PIXELS: u64 = 1 << 28;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackendChoice {
    /// the gpu when there is one, the cpu otherwise
    #[default]
    Auto,
    Cpu,
    /// the gpu or an error, for tests
    Gpu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Backend {
    Gpu,
    Cpu,
}

/// one request: `node`'s pixels over `region` at `level`
#[derive(Debug, Clone, Copy)]
pub struct RenderRequest {
    pub node: NodeId,
    pub region: IRect,
    pub level: u32,
    pub precision: Precision,
    pub working: WorkingSpace,
    pub backend: BackendChoice,
}

impl RenderRequest {
    pub fn new(node: NodeId, region: IRect) -> Self {
        Self {
            node,
            region,
            level: 0,
            precision: Precision::F16,
            working: WorkingSpace::default(),
            backend: BackendChoice::Auto,
        }
    }

    pub fn level(mut self, level: u32) -> Self {
        self.level = level;
        self
    }

    pub fn precision(mut self, precision: Precision) -> Self {
        self.precision = precision;
        self
    }

    pub fn working(mut self, working: WorkingSpace) -> Self {
        self.working = working;
        self
    }

    pub fn backend(mut self, backend: BackendChoice) -> Self {
        self.backend = backend;
        self
    }
}

/// what the last render did, for tests and the status bar
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RenderStats {
    pub gpu: bool,
    pub nodes_run: u32,
    pub cache_hits: u32,
}

#[derive(Debug, Clone)]
pub struct EngineOptions {
    pub gpu: GpuOptions,
    pub use_gpu: bool,
    pub gpu_cache_bytes: u64,
    pub cpu_cache_bytes: u64,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            gpu: GpuOptions::default(),
            use_gpu: true,
            gpu_cache_bytes: 1 << 30,
            cpu_cache_bytes: 1 << 30,
        }
    }
}

/// shown in settings and the status bar
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EngineStatus {
    pub backend: Backend,
    pub adapter: Option<String>,
    pub software: bool,
    /// why the cpu is in use, in plain words
    pub note: Option<String>,
    #[ts(type = "number")]
    pub gpu_cache_bytes: u64,
    #[ts(type = "number")]
    pub cpu_cache_bytes: u64,
    pub recoveries: u32,
}

const OFF_IN_SETTINGS: &str =
    "The graphics card is turned off in settings, so Vellum draws with the processor.";

enum GpuState {
    Ready,
    Off(String),
    Retry {
        at: Instant,
        attempts: u32,
        reason: String,
    },
}

struct Inner {
    options: EngineOptions,
    gpu: Option<Arc<GpuContext>>,
    state: GpuState,
    gpu_cache: Lru<Option<Arc<GpuImage>>>,
    cpu_cache: Lru<Option<Arc<CpuImage>>>,
    stats: RenderStats,
    recoveries: u32,
}

pub struct Engine {
    inner: Mutex<Inner>,
}

impl Engine {
    /// starts the engine, opening the gpu if allowed. never fails: without a gpu it renders
    /// on the cpu and says why in [`Engine::status`].
    pub fn new(options: EngineOptions) -> Self {
        let mut inner = Inner {
            gpu_cache: Lru::new(options.gpu_cache_bytes),
            cpu_cache: Lru::new(options.cpu_cache_bytes),
            gpu: None,
            state: GpuState::Off(OFF_IN_SETTINGS.into()),
            stats: RenderStats::default(),
            recoveries: 0,
            options,
        };
        if inner.options.use_gpu {
            inner.state = GpuState::Retry {
                at: Instant::now(),
                attempts: 0,
                reason: String::new(),
            };
            inner.gpu();
        }
        Self {
            inner: Mutex::new(inner),
        }
    }

    /// turns the gpu on or off, eg from settings. turning it on tries to open it at once.
    pub fn set_use_gpu(&self, on: bool) {
        let mut inner = self.inner.lock();
        if inner.options.use_gpu == on {
            return;
        }
        inner.options.use_gpu = on;
        if on {
            inner.state = GpuState::Retry {
                at: Instant::now(),
                attempts: 0,
                reason: String::new(),
            };
            inner.gpu();
        } else {
            inner.gpu = None;
            inner.gpu_cache.clear();
            inner.state = GpuState::Off(OFF_IN_SETTINGS.into());
        }
    }

    pub fn cpu_only() -> Self {
        Self::new(EngineOptions {
            use_gpu: false,
            ..Default::default()
        })
    }

    pub fn status(&self) -> EngineStatus {
        let inner = self.inner.lock();
        let note = match &inner.state {
            GpuState::Ready => None,
            GpuState::Off(r) => Some(r.clone()),
            GpuState::Retry { reason, .. } => Some(reason.clone()),
        };
        EngineStatus {
            backend: if inner.gpu.is_some() {
                Backend::Gpu
            } else {
                Backend::Cpu
            },
            adapter: inner.gpu.as_ref().map(|g| g.info.name.clone()),
            software: inner
                .gpu
                .as_ref()
                .is_some_and(|g| g.info.device_type == wgpu::DeviceType::Cpu),
            note,
            gpu_cache_bytes: inner.gpu_cache.used(),
            cpu_cache_bytes: inner.cpu_cache.used(),
            recoveries: inner.recoveries,
        }
    }

    pub fn stats(&self) -> RenderStats {
        self.inner.lock().stats
    }

    /// the gpu context, if one is open. tests use it to simulate a lost device.
    pub fn gpu_context(&self) -> Option<Arc<GpuContext>> {
        self.inner.lock().gpu.clone()
    }

    pub fn clear_caches(&self) {
        let mut inner = self.inner.lock();
        inner.gpu_cache.clear();
        inner.cpu_cache.clear();
    }

    pub fn set_budgets(&self, gpu_bytes: u64, cpu_bytes: u64) {
        let mut inner = self.inner.lock();
        inner.gpu_cache.set_budget(gpu_bytes);
        inner.cpu_cache.set_budget(cpu_bytes);
    }

    /// the cache key of a request without rendering it. two requests with the same key
    /// produce the same pixels, which is how the viewport skips tiles that did not change.
    pub fn key(&self, graph: &Graph, req: &RenderRequest) -> Result<u64, RenderError> {
        graph.validate(req.node)?;
        let mut plan = Planner::new(graph, req);
        plan.key(req.node, req.region)
    }

    /// renders a request into a cpu image covering exactly `req.region`, in premultiplied
    /// working space values
    pub fn render_image(
        &self,
        graph: &Graph,
        req: &RenderRequest,
    ) -> Result<CpuImage, RenderError> {
        let mut inner = self.inner.lock();
        inner.render(graph, req, |inner, root, gpu| match (root, gpu) {
            (Root::Cpu(img), _) => Ok(match img {
                Some(img) => img.crop(req.region),
                None => CpuImage::transparent(req.region),
            }),
            (Root::Gpu(img), Some(ctx)) => {
                let _ = inner;
                Ok(match img {
                    Some(img) => ctx.read_image(&img)?.crop(req.region),
                    None => CpuImage::transparent(req.region),
                })
            }
            (Root::Gpu(_), None) => Err(RenderError::Gpu(GpuError::Lost)),
        })
    }

    /// renders several requests for display: premultiplied rgba8 in the display's encoding,
    /// dithered, one tightly packed buffer per request. all gpu work is read back with a
    /// single wait.
    pub fn render_rgba8(
        &self,
        graph: &Graph,
        reqs: &[RenderRequest],
        display: &ColorTransform,
    ) -> Result<Vec<Vec<u8>>, RenderError> {
        let mut inner = self.inner.lock();
        inner.render_display(graph, reqs, display)
    }
}

/// the result of the root node, on whichever device made it
enum Root {
    Cpu(Option<Arc<CpuImage>>),
    Gpu(Option<Arc<GpuImage>>),
}

impl Inner {
    /// the gpu, reopening it after a loss when the backoff allows
    fn gpu(&mut self) -> Option<Arc<GpuContext>> {
        if !self.options.use_gpu {
            return None;
        }
        if let Some(ctx) = &self.gpu {
            if !ctx.is_lost() {
                return Some(ctx.clone());
            }
            tracing::warn!("reopening the gpu after it was lost");
            self.gpu = None;
            self.gpu_cache.clear();
            self.recoveries += 1;
            self.state = GpuState::Retry {
                at: Instant::now(),
                attempts: 0,
                reason: "The graphics card stopped responding. Vellum is reconnecting to it."
                    .into(),
            };
        }
        let attempts = match &self.state {
            GpuState::Ready | GpuState::Off(_) => return None,
            GpuState::Retry { at, attempts, .. } => {
                if Instant::now() < *at {
                    return None;
                }
                *attempts
            }
        };
        match GpuContext::new(self.options.gpu) {
            Ok(ctx) => {
                let ctx = Arc::new(ctx);
                self.gpu = Some(ctx.clone());
                self.state = GpuState::Ready;
                Some(ctx)
            }
            Err(GpuError::NoAdapter) if attempts == 0 && self.recoveries == 0 => {
                self.state = GpuState::Off(
                    "No graphics card that Vellum can use was found, so it draws with the processor.".into(),
                );
                None
            }
            Err(err) => {
                tracing::warn!(%err, "the gpu could not be opened");
                let wait = Duration::from_secs((1u64 << attempts.min(5)).min(30));
                self.state = GpuState::Retry {
                    at: Instant::now() + wait,
                    attempts: attempts + 1,
                    reason: format!(
                        "The graphics card is not available ({err}). Vellum draws with the processor and keeps trying."
                    ),
                };
                None
            }
        }
    }

    fn render<R>(
        &mut self,
        graph: &Graph,
        req: &RenderRequest,
        finish: impl Fn(&mut Inner, Root, Option<&Arc<GpuContext>>) -> Result<R, RenderError>,
    ) -> Result<R, RenderError> {
        graph.validate(req.node)?;
        let gpu = match req.backend {
            BackendChoice::Cpu => None,
            BackendChoice::Auto => self.gpu(),
            BackendChoice::Gpu => Some(self.gpu().ok_or(RenderError::Gpu(GpuError::NoAdapter))?),
        };
        if let Some(ctx) = gpu {
            match self.walk_gpu(&ctx, graph, req) {
                Ok(root) => match finish(self, Root::Gpu(root), Some(&ctx)) {
                    Ok(r) => return Ok(r),
                    Err(e) if req.backend == BackendChoice::Gpu => return Err(e),
                    Err(e) => tracing::warn!(%e, "gpu readback failed, using the cpu"),
                },
                Err(e) if req.backend == BackendChoice::Gpu => return Err(e),
                Err(e) => tracing::warn!(%e, "gpu render failed, using the cpu"),
            }
        }
        let root = self.walk_cpu(graph, req)?;
        finish(self, Root::Cpu(root), None)
    }

    fn walk_cpu(
        &mut self,
        graph: &Graph,
        req: &RenderRequest,
    ) -> Result<Option<Arc<CpuImage>>, RenderError> {
        let mut plan = Planner::new(graph, req);
        let mut exec = CpuWalk {
            cache: &mut self.cpu_cache,
            stats: RenderStats::default(),
        };
        let root = eval(&mut plan, &mut exec, req.node, req.region)?;
        self.stats = exec.stats;
        Ok(root)
    }

    /// walks the graph on the gpu inside error scopes, so running out of memory clears the
    /// cache and tries once more, and a lost device or bad shader falls back to the cpu
    fn walk_gpu(
        &mut self,
        ctx: &Arc<GpuContext>,
        graph: &Graph,
        req: &RenderRequest,
    ) -> Result<Option<Arc<GpuImage>>, RenderError> {
        for attempt in 0..2 {
            let validation = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let oom = ctx.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
            let mut plan = Planner::new(graph, req);
            let mut exec = GpuWalk {
                gx: GpuExec::new(ctx, req.precision, req.working, req.level),
                cache: &mut self.gpu_cache,
                stats: RenderStats {
                    gpu: true,
                    ..Default::default()
                },
            };
            let result = eval(&mut plan, &mut exec, req.node, req.region);
            exec.gx.flush();
            let stats = exec.stats;
            drop(exec);
            let oom_err = pollster::block_on(oom.pop());
            let validation_err = pollster::block_on(validation.pop());
            if ctx.is_lost() {
                return Err(RenderError::Gpu(GpuError::Lost));
            }
            if oom_err.is_some() {
                self.gpu_cache.clear();
                if attempt == 0 {
                    continue;
                }
                return Err(RenderError::Gpu(GpuError::OutOfMemory));
            }
            if let Some(err) = validation_err {
                self.gpu_cache.clear();
                return Err(RenderError::Gpu(GpuError::Validation(err.to_string())));
            }
            self.stats = stats;
            return result;
        }
        Err(RenderError::Gpu(GpuError::OutOfMemory))
    }

    fn render_display(
        &mut self,
        graph: &Graph,
        reqs: &[RenderRequest],
        display: &ColorTransform,
    ) -> Result<Vec<Vec<u8>>, RenderError> {
        let Some(first) = reqs.first() else {
            return Ok(Vec::new());
        };
        let gpu = match first.backend {
            BackendChoice::Cpu => None,
            _ => self.gpu(),
        };
        if let Some(ctx) = gpu {
            match self.display_gpu(&ctx, graph, reqs, display) {
                Ok(out) => return Ok(out),
                Err(e) if first.backend == BackendChoice::Gpu => return Err(e),
                Err(e) => tracing::warn!(%e, "gpu tiles failed, using the cpu"),
            }
        }
        let mut out = Vec::with_capacity(reqs.len());
        let mut stats = RenderStats::default();
        for req in reqs {
            graph.validate(req.node)?;
            let root = self.walk_cpu(graph, req)?;
            stats.nodes_run += self.stats.nodes_run;
            stats.cache_hits += self.stats.cache_hits;
            out.push(display_cpu(root.as_deref(), req.region, req.level, display));
        }
        self.stats = stats;
        Ok(out)
    }

    fn display_gpu(
        &mut self,
        ctx: &Arc<GpuContext>,
        graph: &Graph,
        reqs: &[RenderRequest],
        display: &ColorTransform,
    ) -> Result<Vec<Vec<u8>>, RenderError> {
        let mut params = Vec::new();
        display.pack(&mut params);
        let mut tiles = Vec::with_capacity(reqs.len());
        let mut stats = RenderStats {
            gpu: true,
            ..Default::default()
        };
        for req in reqs {
            graph.validate(req.node)?;
            let root = self.walk_gpu(ctx, graph, req)?;
            stats.nodes_run += self.stats.nodes_run;
            stats.cache_hits += self.stats.cache_hits;
            let mut gx = GpuExec::new(ctx, req.precision, req.working, req.level);
            let tile = gx.dispatch(
                &DISPLAY,
                &[root.as_deref()],
                req.region,
                Dispatch {
                    params: &params,
                    lut: display.lut().map(|l| &**l),
                    format: Some(StorageFormat::Rgba8),
                    ..Default::default()
                },
            )?;
            gx.flush();
            tiles.push(tile);
        }
        let refs: Vec<&GpuImage> = tiles.iter().collect();
        let out = ctx.read_many(&refs)?;
        if ctx.is_lost() {
            return Err(RenderError::Gpu(GpuError::Lost));
        }
        self.stats = stats;
        Ok(out)
    }
}

/// working space to display values, dithered by half a step so smooth gradients do not band
const DISPLAY: Kernel = Kernel {
    name: "display",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let c = unpremultiply(load0(p));
    if (c.a <= 0.0) { return vec4<f32>(0.0); }
    let rgb = clamp(color_steps(0u, c.rgb), vec3<f32>(0.0), vec3<f32>(1.0));
    let p0 = level0(p);
    let n = (pixel_noise(p0.x, p0.y) - 0.5) / 255.0;
    let a = clamp(c.a, 0.0, 1.0);
    return vec4<f32>(clamp((rgb + n) * a, vec3<f32>(0.0), vec3<f32>(a)), a);
}
"#,
};

fn display_cpu(
    img: Option<&CpuImage>,
    region: IRect,
    level: u32,
    display: &ColorTransform,
) -> Vec<u8> {
    let mut out = vec![0u8; region.area() as usize * 4];
    let Some(img) = img else {
        return out;
    };
    let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let row_bytes = region.w as usize * 4;
    out.par_chunks_mut(row_bytes.max(4))
        .enumerate()
        .for_each(|(y, row)| {
            let ay = region.y + y as i32;
            for (x, px) in row.chunks_exact_mut(4).enumerate() {
                let ax = region.x + x as i32;
                let c = crate::nodes::unpremultiply(img.get(ax, ay));
                if c[3] <= 0.0 {
                    continue;
                }
                let rgb = display.apply([c[0], c[1], c[2]]).map(|v| v.clamp(0.0, 1.0));
                let n = (crate::blend::pixel_noise(ax << level, ay << level) - 0.5) / 255.0;
                let a = c[3].clamp(0.0, 1.0);
                for k in 0..3 {
                    px[k] = to_u8(((rgb[k] + n) * a).clamp(0.0, a));
                }
                px[3] = to_u8(a);
            }
        });
    out
}

/// bounds and cache keys for one request, memoised per node and region
struct Planner<'g> {
    graph: &'g Graph,
    level: u32,
    working: WorkingSpace,
    precision: Precision,
    bounds: HashMap<NodeId, Bounds>,
    keys: HashMap<(NodeId, IRect), u64>,
}

impl<'g> Planner<'g> {
    fn new(graph: &'g Graph, req: &RenderRequest) -> Self {
        Self {
            graph,
            level: req.level,
            working: req.working,
            precision: req.precision,
            bounds: HashMap::new(),
            keys: HashMap::new(),
        }
    }

    fn cx<'n>(&self, node: &'n Node) -> NodeCx<'n> {
        NodeCx {
            params: Params {
                specs: node.kind.info().params,
                values: &node.params,
            },
            level: self.level,
            working: self.working,
            precision: self.precision,
            attachment: node.attachment.as_ref(),
        }
    }

    fn bounds(&mut self, id: NodeId) -> Result<Bounds, RenderError> {
        if let Some(b) = self.bounds.get(&id) {
            return Ok(*b);
        }
        let node = self.graph.node(id)?;
        let mut inputs = Vec::with_capacity(node.inputs.len());
        for input in &node.inputs {
            inputs.push(match input {
                Some(i) => Some(self.bounds(*i)?),
                None => None,
            });
        }
        let b = if node.bypass {
            inputs.first().copied().flatten().unwrap_or(Bounds::Empty)
        } else {
            node.kind.bounds(&self.cx(node), &inputs)
        };
        self.bounds.insert(id, b);
        Ok(b)
    }

    /// the part of `region` this node can produce
    fn clip(&mut self, id: NodeId, region: IRect) -> Result<IRect, RenderError> {
        let r = self.bounds(id)?.clip(region);
        if r.area() > MAX_REGION_PIXELS {
            return Err(RenderError::TooLarge(r.w, r.h));
        }
        Ok(r)
    }

    fn key(&mut self, id: NodeId, region: IRect) -> Result<u64, RenderError> {
        let node = self.graph.node(id)?;
        if node.bypass {
            return match node.inputs.first().copied().flatten() {
                Some(input) => self.key(input, region),
                None => Ok(0),
            };
        }
        let r = self.clip(id, region)?;
        if r.is_empty() {
            return Ok(0);
        }
        if let Some(k) = self.keys.get(&(id, r)) {
            return Ok(*k);
        }
        let cx = self.cx(node);
        let mut h = std::hash::DefaultHasher::new();
        node.kind.info().id.hash(&mut h);
        cx.params.hash_into(&mut h);
        node.kind.key(&cx, r, &mut h);
        r.hash(&mut h);
        self.level.hash(&mut h);
        self.precision.hash(&mut h);
        self.working.hash(&mut h);
        for (i, input) in node.inputs.iter().enumerate() {
            let k = match input {
                Some(n) => self.key(*n, node.kind.input_region(&cx, i, r))?,
                None => 0,
            };
            h.write_u64(k);
        }
        let k = h.finish().max(1);
        self.keys.insert((id, r), k);
        Ok(k)
    }
}

/// what differs between walking on the cpu and on the gpu
trait Walk {
    type Img;
    fn cached(&mut self, key: u64) -> Option<Option<Arc<Self::Img>>>;
    fn store(&mut self, key: u64, img: Option<Arc<Self::Img>>);
    fn run(
        &mut self,
        node: &Node,
        cx: &NodeCx,
        inputs: &[Option<&Self::Img>],
        region: IRect,
    ) -> Result<Out<Self::Img>, RenderError>;
    fn rect(img: &Self::Img) -> IRect;
    fn stats(&mut self) -> &mut RenderStats;
}

fn eval<W: Walk>(
    plan: &mut Planner,
    walk: &mut W,
    id: NodeId,
    region: IRect,
) -> Result<Option<Arc<W::Img>>, RenderError> {
    let node = plan.graph.node(id)?;
    if node.bypass {
        return match node.inputs.first().copied().flatten() {
            Some(input) => eval(plan, walk, input, region),
            None => Ok(None),
        };
    }
    let r = plan.clip(id, region)?;
    if r.is_empty() {
        return Ok(None);
    }
    let key = plan.key(id, r)?;
    if let Some(hit) = walk.cached(key) {
        walk.stats().cache_hits += 1;
        return Ok(hit);
    }
    let cx = plan.cx(node);
    let mut inputs: Vec<Option<Arc<W::Img>>> = Vec::with_capacity(node.inputs.len());
    for (i, input) in node.inputs.iter().enumerate() {
        inputs.push(match input {
            Some(n) => eval(plan, walk, *n, node.kind.input_region(&cx, i, r))?,
            None => None,
        });
    }
    let refs: Vec<Option<&W::Img>> = inputs.iter().map(|i| i.as_deref()).collect();
    let out = walk.run(node, &cx, &refs, r)?;
    walk.stats().nodes_run += 1;
    let result = match out {
        Out::New(img) if W::rect(&img).is_empty() => None,
        Out::New(img) => Some(Arc::new(img)),
        Out::Input(i) => inputs.get(i).cloned().flatten(),
        Out::Empty => None,
    };
    walk.store(key, result.clone());
    Ok(result)
}

struct CpuWalk<'a> {
    cache: &'a mut Lru<Option<Arc<CpuImage>>>,
    stats: RenderStats,
}

impl Walk for CpuWalk<'_> {
    type Img = CpuImage;

    fn cached(&mut self, key: u64) -> Option<Option<Arc<CpuImage>>> {
        self.cache.get(key)
    }

    fn store(&mut self, key: u64, img: Option<Arc<CpuImage>>) {
        let bytes = img.as_ref().map_or(64, |i| i.bytes());
        self.cache.put(key, img, bytes);
    }

    fn run(
        &mut self,
        node: &Node,
        cx: &NodeCx,
        inputs: &[Option<&CpuImage>],
        region: IRect,
    ) -> Result<Out<CpuImage>, RenderError> {
        Ok(node.kind.cpu(cx, inputs, region))
    }

    fn rect(img: &CpuImage) -> IRect {
        img.rect
    }

    fn stats(&mut self) -> &mut RenderStats {
        &mut self.stats
    }
}

struct GpuWalk<'a, 'c> {
    gx: GpuExec<'c>,
    cache: &'a mut Lru<Option<Arc<GpuImage>>>,
    stats: RenderStats,
}

impl Walk for GpuWalk<'_, '_> {
    type Img = GpuImage;

    fn cached(&mut self, key: u64) -> Option<Option<Arc<GpuImage>>> {
        self.cache.get(key)
    }

    fn store(&mut self, key: u64, img: Option<Arc<GpuImage>>) {
        let bytes = img.as_ref().map_or(64, |i| i.bytes());
        self.cache.put(key, img, bytes);
    }

    fn run(
        &mut self,
        node: &Node,
        cx: &NodeCx,
        inputs: &[Option<&GpuImage>],
        region: IRect,
    ) -> Result<Out<GpuImage>, RenderError> {
        Ok(node.kind.gpu(cx, &mut self.gx, inputs, region)?)
    }

    fn rect(img: &GpuImage) -> IRect {
        img.rect
    }

    fn stats(&mut self) -> &mut RenderStats {
        &mut self.stats
    }
}

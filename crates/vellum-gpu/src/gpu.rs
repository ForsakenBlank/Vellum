//! the wgpu device and the plumbing every kernel shares.
//!
//! every kernel is a compute shader with the same bindings, so there is one bind group
//! layout per output format and pipelines are built lazily per kernel and format:
//!
//! - 0: the output, a write only storage texture
//! - 1: a uniform block with the output and input rectangles, the level, the working
//!   space encoding and 32 vec4s of parameters
//! - 2 to 5: up to four inputs, sampled with `textureLoad`
//! - 6: a read only storage buffer of floats (blur weights, gradient stops)
//! - 7: a 3d texture for colour luts

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use parking_lot::Mutex;
use wgpu::util::DeviceExt;

use crate::color::{Lut3d, WorkingSpace};
use crate::image::{CpuImage, Precision, SourceData, SourcePixels};
use crate::rect::IRect;

const PRELUDE: &str = include_str!("shaders/prelude.wgsl");

/// a compute kernel: its wgsl body defines `fn kernel(p: vec2<i32>) -> vec4<f32>`
#[derive(Debug, Clone, Copy)]
pub struct Kernel {
    /// unique, used to cache the compiled pipeline
    pub name: &'static str,
    pub body: &'static str,
}

/// formats a kernel can write
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageFormat {
    Rgba8,
    Rgba16F,
    Rgba32F,
}

impl StorageFormat {
    pub fn of(precision: Precision) -> Self {
        match precision {
            Precision::U8 => StorageFormat::Rgba8,
            Precision::F16 => StorageFormat::Rgba16F,
            Precision::F32 => StorageFormat::Rgba32F,
        }
    }

    pub fn texture_format(self) -> wgpu::TextureFormat {
        match self {
            StorageFormat::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
            StorageFormat::Rgba16F => wgpu::TextureFormat::Rgba16Float,
            StorageFormat::Rgba32F => wgpu::TextureFormat::Rgba32Float,
        }
    }

    fn wgsl(self) -> &'static str {
        match self {
            StorageFormat::Rgba8 => "rgba8unorm",
            StorageFormat::Rgba16F => "rgba16float",
            StorageFormat::Rgba32F => "rgba32float",
        }
    }

    pub fn bytes_per_pixel(self) -> u32 {
        match self {
            StorageFormat::Rgba8 => 4,
            StorageFormat::Rgba16F => 8,
            StorageFormat::Rgba32F => 16,
        }
    }
}

#[derive(Debug, thiserror::Error, Clone)]
pub enum GpuError {
    #[error("no usable graphics adapter was found")]
    NoAdapter,
    #[error("the graphics device could not be opened: {0}")]
    Device(String),
    #[error("the graphics device was lost")]
    Lost,
    #[error("a shader did not compile: {0}")]
    Shader(String),
    #[error("reading pixels back from the gpu failed: {0}")]
    Readback(String),
    #[error("the gpu ran out of memory")]
    OutOfMemory,
    #[error("a {0} pixel image is bigger than this gpu can hold in one piece")]
    TooLarge(String),
    #[error("the gpu reported an error: {0}")]
    Validation(String),
}

/// how to pick an adapter
#[derive(Debug, Clone, Copy)]
pub struct GpuOptions {
    pub power: wgpu::PowerPreference,
    /// accept software rasterisers (warp, lavapipe). off in the app, where vellum's own cpu
    /// path is more predictable; on in tests so the gpu path runs in ci.
    pub allow_software: bool,
}

impl GpuOptions {
    /// any adapter, software rasterisers included
    pub fn allowing_software() -> Self {
        Self {
            allow_software: true,
            ..Self::default()
        }
    }
}

impl Default for GpuOptions {
    fn default() -> Self {
        Self {
            power: wgpu::PowerPreference::HighPerformance,
            allow_software: std::env::var("VELLUM_GPU").is_ok_and(|v| v == "software"),
        }
    }
}

/// a gpu texture holding a premultiplied image over `rect`
#[derive(Debug)]
pub struct GpuImage {
    pub rect: IRect,
    pub format: StorageFormat,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

impl GpuImage {
    pub fn bytes(&self) -> u64 {
        self.rect.area() * u64::from(self.format.bytes_per_pixel())
    }
}

/// the uniform block every kernel sees. must match `struct Frame` in the prelude.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FrameUniform {
    out_rect: [i32; 4],
    in_rect: [[i32; 4]; 4],
    level: u32,
    encoding: u32,
    gamma: f32,
    data_len: u32,
    params: [[f32; 4]; MAX_PARAMS],
}

/// how many vec4 parameters a kernel can receive
pub const MAX_PARAMS: usize = 32;

struct Layouts {
    group: wgpu::BindGroupLayout,
    pipeline: wgpu::PipelineLayout,
}

pub struct GpuContext {
    pub info: wgpu::AdapterInfo,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub max_texture: u32,
    lost: Arc<AtomicBool>,
    errors: Arc<AtomicU64>,
    layouts: HashMap<StorageFormat, Layouts>,
    pipelines: Mutex<HashMap<(&'static str, StorageFormat), Arc<wgpu::ComputePipeline>>>,
    luts: Mutex<HashMap<u64, Arc<wgpu::TextureView>>>,
    dummy_2d: wgpu::TextureView,
    dummy_3d: wgpu::TextureView,
    dummy_data: wgpu::Buffer,
}

impl std::fmt::Debug for GpuContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuContext")
            .field("adapter", &self.info.name)
            .finish()
    }
}

impl GpuContext {
    /// opens the best adapter allowed by `options`. blocks until the device is ready.
    pub fn new(options: GpuOptions) -> Result<Self, GpuError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            flags: wgpu::InstanceFlags::empty(),
            memory_budget_thresholds: Default::default(),
            backend_options: Default::default(),
            display: None,
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: options.power,
            force_fallback_adapter: false,
            compatible_surface: None,
            // bucketing hides exact limits from untrusted web content; vellum is native
            apply_limit_buckets: false,
        }))
        .map_err(|_| GpuError::NoAdapter)?;
        let info = adapter.get_info();
        if info.device_type == wgpu::DeviceType::Cpu && !options.allow_software {
            return Err(GpuError::NoAdapter);
        }
        let adapter_limits = adapter.limits();
        let limits = wgpu::Limits {
            max_texture_dimension_2d: adapter_limits.max_texture_dimension_2d,
            max_buffer_size: adapter_limits.max_buffer_size,
            max_storage_buffer_binding_size: adapter_limits.max_storage_buffer_binding_size,
            ..wgpu::Limits::downlevel_defaults()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("vellum"),
            required_features: wgpu::Features::empty(),
            required_limits: limits,
            experimental_features: Default::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: Default::default(),
        }))
        .map_err(|e| GpuError::Device(e.to_string()))?;

        let lost = Arc::new(AtomicBool::new(false));
        let errors = Arc::new(AtomicU64::new(0));
        {
            let lost = lost.clone();
            device.set_device_lost_callback(move |reason, message| {
                tracing::warn!(?reason, %message, "the gpu device was lost");
                lost.store(true, Ordering::SeqCst);
            });
        }
        {
            // wgpu panics on errors nobody captured; log them instead and let the next
            // render notice through the error counter
            let errors = errors.clone();
            device.on_uncaptured_error(Arc::new(move |err: wgpu::Error| {
                tracing::error!(%err, "gpu error");
                errors.fetch_add(1, Ordering::SeqCst);
            }));
        }

        let mut layouts = HashMap::new();
        for format in [
            StorageFormat::Rgba8,
            StorageFormat::Rgba16F,
            StorageFormat::Rgba32F,
        ] {
            layouts.insert(format, make_layouts(&device, format));
        }

        let dummy_2d = device
            .create_texture_with_data(
                &queue,
                &wgpu::TextureDescriptor {
                    label: Some("empty input"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &[0, 0, 0, 0],
            )
            .create_view(&Default::default());
        let dummy_3d = device
            .create_texture_with_data(
                &queue,
                &wgpu::TextureDescriptor {
                    label: Some("empty lut"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D3,
                    format: wgpu::TextureFormat::Rgba32Float,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                bytemuck::cast_slice(&[0f32; 4]),
            )
            .create_view(&Default::default());
        let dummy_data = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("empty data"),
            contents: bytemuck::cast_slice(&[0f32; 4]),
            usage: wgpu::BufferUsages::STORAGE,
        });

        tracing::info!(adapter = %info.name, backend = ?info.backend, "gpu ready");
        Ok(Self {
            max_texture: device.limits().max_texture_dimension_2d,
            info,
            device,
            queue,
            lost,
            errors,
            layouts,
            pipelines: Mutex::new(HashMap::new()),
            luts: Mutex::new(HashMap::new()),
            dummy_2d,
            dummy_3d,
            dummy_data,
        })
    }

    pub fn is_lost(&self) -> bool {
        self.lost.load(Ordering::SeqCst)
    }

    /// how many uncaptured errors the device has reported
    pub fn error_count(&self) -> u64 {
        self.errors.load(Ordering::SeqCst)
    }

    /// marks the device as lost, as if the driver had reset. used by tests.
    pub fn simulate_loss(&self) {
        self.device.destroy();
        self.lost.store(true, Ordering::SeqCst);
    }

    fn pipeline(
        &self,
        kernel: &Kernel,
        format: StorageFormat,
    ) -> Result<Arc<wgpu::ComputePipeline>, GpuError> {
        if let Some(p) = self.pipelines.lock().get(&(kernel.name, format)) {
            return Ok(p.clone());
        }
        let source = format!(
            "{}\n{}\n",
            PRELUDE.replace("{{FORMAT}}", format.wgsl()),
            kernel.body
        );
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(kernel.name),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let layout = &self.layouts[&format];
        let pipeline = self
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(kernel.name),
                layout: Some(&layout.pipeline),
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(GpuError::Shader(format!("{}: {err}", kernel.name)));
        }
        let pipeline = Arc::new(pipeline);
        self.pipelines
            .lock()
            .insert((kernel.name, format), pipeline.clone());
        Ok(pipeline)
    }

    fn lut_view(&self, lut: &Lut3d) -> Arc<wgpu::TextureView> {
        let mut luts = self.luts.lock();
        if let Some(v) = luts.get(&lut.id) {
            return v.clone();
        }
        if luts.len() > 16 {
            luts.clear();
        }
        let data: Vec<f32> = lut
            .data
            .iter()
            .flat_map(|c| [c[0], c[1], c[2], 1.0])
            .collect();
        let view = self
            .device
            .create_texture_with_data(
                &self.queue,
                &wgpu::TextureDescriptor {
                    label: Some("colour lut"),
                    size: wgpu::Extent3d {
                        width: lut.size,
                        height: lut.size,
                        depth_or_array_layers: lut.size,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D3,
                    format: wgpu::TextureFormat::Rgba32Float,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                bytemuck::cast_slice(&data),
            )
            .create_view(&Default::default());
        let view = Arc::new(view);
        luts.insert(lut.id, view.clone());
        view
    }

    pub fn create_image(
        &self,
        rect: IRect,
        format: StorageFormat,
        label: &str,
    ) -> Result<GpuImage, GpuError> {
        if rect.w > self.max_texture || rect.h > self.max_texture {
            return Err(GpuError::TooLarge(format!("{}x{}", rect.w, rect.h)));
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: rect.w.max(1),
                height: rect.h.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: format.texture_format(),
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Ok(GpuImage {
            rect,
            format,
            texture,
            view,
        })
    }

    /// uploads raw source pixels as they are. the ingest kernel converts them.
    pub fn upload_source(&self, px: &SourcePixels) -> Result<GpuImage, GpuError> {
        let rect = px.rect;
        Ok(match &*px.data {
            SourceData::U8(d) => {
                let img = self.create_image(rect, StorageFormat::Rgba8, "source")?;
                self.write(&img, d, 4);
                img
            }
            SourceData::U16(d) => {
                // core webgpu has no 16 bit normalised format, so widen to floats
                let f: Vec<f32> = d.iter().map(|v| f32::from(*v) / 65535.0).collect();
                let img = self.create_image(rect, StorageFormat::Rgba32F, "source")?;
                self.write(&img, bytemuck::cast_slice(&f), 16);
                img
            }
            SourceData::F32(d) => {
                let img = self.create_image(rect, StorageFormat::Rgba32F, "source")?;
                self.write(&img, bytemuck::cast_slice(d), 16);
                img
            }
        })
    }

    /// uploads a cpu image (premultiplied working values) at the given format
    pub fn upload_image(
        &self,
        img: &CpuImage,
        format: StorageFormat,
    ) -> Result<GpuImage, GpuError> {
        let out = self.create_image(img.rect, format, "upload")?;
        match format {
            StorageFormat::Rgba32F => {
                self.write(&out, bytemuck::cast_slice(&img.data), 16);
            }
            StorageFormat::Rgba16F => {
                let h: Vec<half::f16> = img
                    .data
                    .iter()
                    .flatten()
                    .map(|v| half::f16::from_f32(*v))
                    .collect();
                self.write(&out, bytemuck::cast_slice(&h), 8);
            }
            StorageFormat::Rgba8 => {
                let b: Vec<u8> = img
                    .data
                    .iter()
                    .flatten()
                    .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
                    .collect();
                self.write(&out, &b, 4);
            }
        }
        Ok(out)
    }

    fn write(&self, img: &GpuImage, bytes: &[u8], bpp: u32) {
        if img.rect.is_empty() {
            return;
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &img.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(img.rect.w * bpp),
                rows_per_image: Some(img.rect.h),
            },
            wgpu::Extent3d {
                width: img.rect.w,
                height: img.rect.h,
                depth_or_array_layers: 1,
            },
        );
    }

    /// copies a texture back to memory, one tightly packed row after another
    pub fn read_bytes(&self, img: &GpuImage) -> Result<Vec<u8>, GpuError> {
        Ok(self.read_many(&[img])?.pop().unwrap_or_default())
    }

    /// copies several textures back with one wait, which matters for viewport tiles where
    /// a wait per tile would cost a frame each
    pub fn read_many(&self, images: &[&GpuImage]) -> Result<Vec<Vec<u8>>, GpuError> {
        let align = u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let mut layout = Vec::with_capacity(images.len());
        let mut total = 0u64;
        for img in images {
            let row = u64::from(img.rect.w * img.format.bytes_per_pixel());
            let padded = row.div_ceil(align) * align;
            layout.push((total, row, padded));
            total += padded * u64::from(img.rect.h.max(1));
        }
        if total == 0 {
            return Ok(images.iter().map(|_| Vec::new()).collect());
        }
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: total,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for (img, (offset, _, padded)) in images.iter().zip(&layout) {
            if img.rect.is_empty() {
                continue;
            }
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &img.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: *offset,
                        bytes_per_row: Some(*padded as u32),
                        rows_per_image: Some(img.rect.h),
                    },
                },
                wgpu::Extent3d {
                    width: img.rect.w,
                    height: img.rect.h,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        match rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(GpuError::Readback(e.to_string())),
            Err(_) => {
                return Err(if self.is_lost() {
                    GpuError::Lost
                } else {
                    GpuError::Readback("no reply".into())
                });
            }
        }
        let mapped = slice
            .get_mapped_range()
            .map_err(|e| GpuError::Readback(format!("{e:?}")))?;
        let mut out = Vec::with_capacity(images.len());
        for (img, (offset, row, padded)) in images.iter().zip(&layout) {
            let mut bytes = Vec::with_capacity((*row * u64::from(img.rect.h)) as usize);
            for y in 0..u64::from(img.rect.h) {
                let start = (offset + y * padded) as usize;
                bytes.extend_from_slice(&mapped[start..start + *row as usize]);
            }
            out.push(bytes);
        }
        drop(mapped);
        buffer.unmap();
        Ok(out)
    }

    /// reads a texture back as a cpu image of floats
    pub fn read_image(&self, img: &GpuImage) -> Result<CpuImage, GpuError> {
        let bytes = self.read_bytes(img)?;
        let data: Vec<[f32; 4]> = match img.format {
            StorageFormat::Rgba32F => bytemuck::cast_slice::<u8, [f32; 4]>(&bytes).to_vec(),
            StorageFormat::Rgba16F => bytes
                .chunks_exact(8)
                .map(|c| {
                    let h: &[half::f16] = bytemuck::cast_slice(c);
                    [h[0].to_f32(), h[1].to_f32(), h[2].to_f32(), h[3].to_f32()]
                })
                .collect(),
            StorageFormat::Rgba8 => bytes
                .chunks_exact(4)
                .map(|c| [0, 1, 2, 3].map(|k| f32::from(c[k]) / 255.0))
                .collect(),
        };
        Ok(CpuImage {
            rect: img.rect,
            data,
        })
    }
}

fn make_layouts(device: &wgpu::Device, format: StorageFormat) -> Layouts {
    let texture = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let group = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("kernel"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: format.texture_format(),
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            texture(2),
            texture(3),
            texture(4),
            texture(5),
            wgpu::BindGroupLayoutEntry {
                binding: 6,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 7,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D3,
                    multisampled: false,
                },
                count: None,
            },
        ],
    });
    let pipeline = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("kernel"),
        bind_group_layouts: &[Some(&group)],
        immediate_size: 0,
    });
    Layouts { group, pipeline }
}

/// what one kernel dispatch needs besides its inputs
#[derive(Default)]
pub struct Dispatch<'a> {
    pub params: &'a [[f32; 4]],
    pub data: Option<&'a [f32]>,
    pub lut: Option<&'a Lut3d>,
    /// overrides the pipeline's precision, eg rgba8 for display tiles
    pub format: Option<StorageFormat>,
}

/// records kernel dispatches for one render into a single command buffer
pub struct GpuExec<'a> {
    pub ctx: &'a GpuContext,
    pub precision: Precision,
    pub working: WorkingSpace,
    pub level: u32,
    encoder: Option<wgpu::CommandEncoder>,
    dispatches: usize,
}

impl<'a> GpuExec<'a> {
    pub fn new(
        ctx: &'a GpuContext,
        precision: Precision,
        working: WorkingSpace,
        level: u32,
    ) -> Self {
        Self {
            ctx,
            precision,
            working,
            level,
            encoder: None,
            dispatches: 0,
        }
    }

    pub fn format(&self) -> StorageFormat {
        StorageFormat::of(self.precision)
    }

    /// runs `kernel` over `out`. missing inputs read as transparent.
    pub fn dispatch(
        &mut self,
        kernel: &Kernel,
        inputs: &[Option<&GpuImage>],
        out: IRect,
        d: Dispatch<'_>,
    ) -> Result<GpuImage, GpuError> {
        let format = d.format.unwrap_or_else(|| self.format());
        let image = self.ctx.create_image(out, format, kernel.name)?;
        let pipeline = self.ctx.pipeline(kernel, format)?;
        if out.is_empty() {
            return Ok(image);
        }
        let mut uniform = FrameUniform {
            out_rect: [out.x, out.y, out.w as i32, out.h as i32],
            in_rect: [[0; 4]; 4],
            level: self.level,
            encoding: self.working.encoding.id(),
            gamma: self.working.encoding.gamma(),
            data_len: d.data.map_or(0, |v| v.len() as u32),
            params: [[0.0; 4]; MAX_PARAMS],
        };
        for (i, input) in inputs.iter().take(4).enumerate() {
            if let Some(img) = input {
                uniform.in_rect[i] = [img.rect.x, img.rect.y, img.rect.w as i32, img.rect.h as i32];
            }
        }
        for (dst, src) in uniform.params.iter_mut().zip(d.params) {
            *dst = *src;
        }
        let device = &self.ctx.device;
        let ubuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("frame"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let dbuf = d.data.filter(|v| !v.is_empty()).map(|v| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kernel data"),
                contents: bytemuck::cast_slice(v),
                usage: wgpu::BufferUsages::STORAGE,
            })
        });
        let lut = d.lut.map(|l| self.ctx.lut_view(l));
        let view = |i: usize| {
            inputs
                .get(i)
                .copied()
                .flatten()
                .map_or(&self.ctx.dummy_2d, |img| &img.view)
        };
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(kernel.name),
            layout: &self.ctx.layouts[&format].group,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&image.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: ubuf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(view(0)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(view(1)),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(view(2)),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(view(3)),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: dbuf
                        .as_ref()
                        .unwrap_or(&self.ctx.dummy_data)
                        .as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(
                        lut.as_deref().unwrap_or(&self.ctx.dummy_3d),
                    ),
                },
            ],
        });
        let encoder = self
            .encoder
            .get_or_insert_with(|| device.create_command_encoder(&Default::default()));
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(kernel.name),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(out.w.div_ceil(8), out.h.div_ceil(8), 1);
        }
        self.dispatches += 1;
        // keep command buffers from growing without bound on very large graphs
        if self.dispatches >= 256 {
            self.flush();
        }
        Ok(image)
    }

    /// submits what has been recorded so far
    pub fn flush(&mut self) {
        if let Some(encoder) = self.encoder.take() {
            self.ctx.queue.submit([encoder.finish()]);
        }
        self.dispatches = 0;
    }
}

impl Drop for GpuExec<'_> {
    fn drop(&mut self) {
        self.flush();
    }
}

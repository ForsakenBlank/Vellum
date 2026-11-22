//! the shape most effects share: one kernel over one input, with a cpu twin that works out
//! each output pixel from the input. an effect implements [`Simple`] (its info, its
//! kernel, how it packs its parameters and the cpu maths for one pixel) and gets
//! [`NodeKind`] for free, including bounds and input regions grown by how far it reads.
//!
//! colour adjustments are simpler still: [`point_kernel!`] wraps a wgsl `adjust` function
//! over straight colour, and [`point`] does the same on the cpu.

use crate::gpu::{Dispatch, GpuError, GpuExec, GpuImage, Kernel};
use crate::graph::{NodeCx, NodeInfo, NodeKind, Out, input_bounds};
use crate::image::CpuImage;
use crate::rect::{Bounds, IRect};

/// what a kernel needs for one render
#[derive(Debug, Clone, Default)]
pub struct Setup {
    pub params: Vec<[f32; 4]>,
    /// tables, weights and other floats too many for the parameters
    pub data: Vec<f32>,
}

impl Setup {
    pub fn new(params: Vec<[f32; 4]>) -> Self {
        Self {
            params,
            data: Vec::new(),
        }
    }

    pub fn with_data(mut self, data: Vec<f32>) -> Self {
        self.data = data;
        self
    }

    /// parameter `i`, zeros past the end
    pub fn p(&self, i: usize) -> [f32; 4] {
        self.params.get(i).copied().unwrap_or_default()
    }
}

/// an effect with one kernel dispatch and a per pixel cpu twin
pub trait Simple: Send + Sync + 'static {
    fn info(&self) -> &'static NodeInfo;
    fn kernel(&self) -> &'static Kernel;

    /// the packed parameters, or none when the effect leaves its input as it is
    fn setup(&self, cx: &NodeCx) -> Option<Setup>;

    /// how far from a pixel the effect reads, in pixels at `cx.level`
    fn reach(&self, _cx: &NodeCx) -> i32 {
        0
    }

    /// true if the effect draws past its input's edges by its reach, like a blur fading out
    fn spreads(&self, _cx: &NodeCx) -> bool {
        false
    }

    /// true if the effect paints without any input, like rendered clouds
    fn generates(&self) -> bool {
        false
    }

    /// one output pixel, premultiplied. `input` is transparent where it has no pixels.
    fn pixel(&self, s: &Setup, cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4];
}

impl<T: Simple> NodeKind for T {
    fn info(&self) -> &'static NodeInfo {
        Simple::info(self)
    }

    fn bounds(&self, cx: &NodeCx, inputs: &[Option<Bounds>]) -> Bounds {
        if self.generates() {
            return Bounds::Infinite;
        }
        let b = input_bounds(inputs, 0);
        if self.spreads(cx) {
            b.expand(self.reach(cx))
        } else {
            b
        }
    }

    fn input_region(&self, cx: &NodeCx, _input: usize, region: IRect) -> IRect {
        let r = self.reach(cx);
        if r > 0 { region.expand(r) } else { region }
    }

    fn cpu(&self, cx: &NodeCx, inputs: &[Option<&CpuImage>], region: IRect) -> Out<CpuImage> {
        let empty;
        let input = match inputs.first().copied().flatten() {
            Some(i) => i,
            None if self.generates() => {
                empty = CpuImage::transparent(IRect::EMPTY);
                &empty
            }
            None => return Out::Empty,
        };
        let Some(s) = self.setup(cx) else {
            return Out::Input(0);
        };
        Out::New(CpuImage::from_fn(region, |x, y| {
            self.pixel(&s, cx, input, x, y)
        }))
    }

    fn gpu(
        &self,
        cx: &NodeCx,
        gx: &mut GpuExec,
        inputs: &[Option<&GpuImage>],
        region: IRect,
    ) -> Result<Out<GpuImage>, GpuError> {
        let input = inputs.first().copied().flatten();
        if input.is_none() && !self.generates() {
            return Ok(Out::Empty);
        }
        let Some(s) = self.setup(cx) else {
            return Ok(Out::Input(0));
        };
        Ok(Out::New(gx.dispatch(
            self.kernel(),
            &[input],
            region,
            Dispatch {
                params: &s.params,
                data: (!s.data.is_empty()).then_some(s.data.as_slice()),
                ..Default::default()
            },
        )?))
    }
}

/// runs a colour adjustment on one premultiplied pixel: `f` sees and returns straight
/// colour, transparent pixels stay transparent, and colours are clamped to [0, 1] in
/// gamma encoded working spaces. the twin of the kernel [`point_kernel!`] builds.
pub fn point(cx: &NodeCx, c: [f32; 4], f: impl FnOnce([f32; 4]) -> [f32; 4]) -> [f32; 4] {
    let c = super::unpremultiply(c);
    if c[3] <= 0.0 {
        return [0.0; 4];
    }
    let mut out = f(c);
    if cx.clamp() {
        for v in out.iter_mut().take(3) {
            *v = v.clamp(0.0, 1.0);
        }
    }
    out[3] = out[3].clamp(0.0, 1.0);
    super::premultiply(out)
}

/// a kernel for a colour adjustment. the body defines
/// `fn adjust(c: vec4<f32>, p: vec2<i32>) -> vec4<f32>` over straight colour.
#[macro_export]
macro_rules! point_kernel {
    ($name:literal, $body:literal) => {
        $crate::gpu::Kernel {
            name: $name,
            body: concat!(
                $body,
                r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let c = unpremultiply(load0(p));
    if (c.a <= 0.0) { return vec4<f32>(0.0); }
    var out = adjust(c, p);
    if (clamps()) { out = vec4<f32>(clamp01(out.rgb), out.a); }
    out.a = clamp(out.a, 0.0, 1.0);
    return premultiply(out);
}
"#
            ),
        }
    };
}

/// the input pixel at (x, y) for effects that only look at that pixel
pub fn at(input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
    input.get(x, y)
}

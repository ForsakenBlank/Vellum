//! colour spaces, transfer functions and colour transforms.
//!
//! a colour space here is a set of primaries (with a white point) plus a transfer function.
//! a [`ColorTransform`] is a short list of steps (decode, matrix, encode, 3d lut, clamp)
//! that runs the same way on the cpu and in wgsl, so the two paths agree. icc profiles are
//! turned into a 3d lut with moxcms, which keeps the gpu side to one generic kernel.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// chromaticities of a set of rgb primaries and their white point
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Primaries {
    /// srgb and rec.709 share these
    Rec709,
    DisplayP3,
    Rec2020,
    AdobeRgb,
    /// acescg and acescct
    AcesAp1,
    /// aces2065-1
    AcesAp0,
}

impl Primaries {
    /// (red, green, blue, white) as xy chromaticities
    fn xy(self) -> [[f64; 2]; 4] {
        const D65: [f64; 2] = [0.3127, 0.3290];
        const ACES_WHITE: [f64; 2] = [0.32168, 0.33767];
        match self {
            Primaries::Rec709 => [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06], D65],
            Primaries::DisplayP3 => [[0.680, 0.320], [0.265, 0.690], [0.150, 0.060], D65],
            Primaries::Rec2020 => [[0.708, 0.292], [0.170, 0.797], [0.131, 0.046], D65],
            Primaries::AdobeRgb => [[0.64, 0.33], [0.21, 0.71], [0.15, 0.06], D65],
            Primaries::AcesAp1 => [[0.713, 0.293], [0.165, 0.830], [0.128, 0.044], ACES_WHITE],
            Primaries::AcesAp0 => [[0.7347, 0.2653], [0.0, 1.0], [0.0001, -0.0770], ACES_WHITE],
        }
    }

    pub fn white(self) -> [f64; 3] {
        xy_to_xyz(self.xy()[3])
    }

    /// linear rgb in these primaries to cie xyz (with this white point)
    pub fn to_xyz(self) -> Mat3 {
        let [r, g, b, w] = self.xy();
        let (r, g, b) = (xy_to_xyz(r), xy_to_xyz(g), xy_to_xyz(b));
        let m = Mat3([[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]]);
        // scale each primary so that rgb 1,1,1 lands on the white point
        let s = m.inverse().apply(xy_to_xyz(w));
        Mat3([
            [r[0] * s[0], g[0] * s[1], b[0] * s[2]],
            [r[1] * s[0], g[1] * s[1], b[1] * s[2]],
            [r[2] * s[0], g[2] * s[1], b[2] * s[2]],
        ])
    }
}

fn xy_to_xyz([x, y]: [f64; 2]) -> [f64; 3] {
    [x / y, 1.0, (1.0 - x - y) / y]
}

/// a transfer function between linear light and stored values
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
#[ts(export)]
pub enum Transfer {
    Linear,
    Srgb,
    /// a pure power curve, eg 2.2, or 2.4 for bt.1886 displays
    Gamma(f32),
    /// the rec.709 and rec.2020 camera curve
    Bt709,
    /// smpte st 2084. linear 1.0 is hdr reference white (203 nits, itu-r bt.2408)
    Pq,
    /// hybrid log gamma, scaled so 75 percent signal (reference white) is linear 1.0
    Hlg,
    AcesCct,
}

impl Eq for Transfer {}

impl std::hash::Hash for Transfer {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id().hash(state);
        if let Transfer::Gamma(g) = self {
            g.to_bits().hash(state);
        }
    }
}

const PQ_M1: f32 = 2610.0 / 16384.0;
const PQ_M2: f32 = 2523.0 / 4096.0 * 128.0;
const PQ_C1: f32 = 3424.0 / 4096.0;
const PQ_C2: f32 = 2413.0 / 4096.0 * 32.0;
const PQ_C3: f32 = 2392.0 / 4096.0 * 32.0;
const PQ_WHITE_NITS: f32 = 203.0;
const HLG_A: f32 = 0.178_832_77;
const HLG_B: f32 = 0.284_668_92;
const HLG_C: f32 = 0.559_910_7;
/// scene linear value of a 75 percent hlg signal
const HLG_WHITE: f32 = 0.265_498;

impl Transfer {
    /// the id used by the wgsl side
    pub fn id(self) -> u32 {
        match self {
            Transfer::Linear => 0,
            Transfer::Srgb => 1,
            Transfer::Gamma(_) => 2,
            Transfer::Bt709 => 3,
            Transfer::Pq => 4,
            Transfer::Hlg => 5,
            Transfer::AcesCct => 6,
        }
    }

    pub fn gamma(self) -> f32 {
        match self {
            Transfer::Gamma(g) => g,
            _ => 1.0,
        }
    }

    /// stored value to linear light. negative values mirror, so wide gamut values that
    /// went below zero survive a round trip.
    pub fn decode(self, v: f32) -> f32 {
        let (s, a) = (v.signum(), v.abs());
        match self {
            Transfer::Linear => v,
            Transfer::Srgb => {
                s * if a <= 0.04045 {
                    a / 12.92
                } else {
                    ((a + 0.055) / 1.055).powf(2.4)
                }
            }
            Transfer::Gamma(g) => s * a.powf(g),
            Transfer::Bt709 => {
                s * if a < 0.081 {
                    a / 4.5
                } else {
                    ((a + 0.099) / 1.099).powf(1.0 / 0.45)
                }
            }
            Transfer::Pq => {
                let p = v.max(0.0).powf(1.0 / PQ_M2);
                let n = (p - PQ_C1).max(0.0) / (PQ_C2 - PQ_C3 * p);
                n.powf(1.0 / PQ_M1) * 10000.0 / PQ_WHITE_NITS
            }
            Transfer::Hlg => {
                let v = v.max(0.0);
                let e = if v <= 0.5 {
                    v * v / 3.0
                } else {
                    (((v - HLG_C) / HLG_A).exp() + HLG_B) / 12.0
                };
                e / HLG_WHITE
            }
            Transfer::AcesCct => {
                if v <= 0.155_251_14 {
                    (v - 0.072_905_53) / 10.540_238
                } else if v < 1.468 {
                    (v * 17.52 - 9.72).exp2()
                } else {
                    65504.0
                }
            }
        }
    }

    /// linear light to a stored value
    pub fn encode(self, l: f32) -> f32 {
        let (s, a) = (l.signum(), l.abs());
        match self {
            Transfer::Linear => l,
            Transfer::Srgb => {
                s * if a <= 0.003_130_8 {
                    a * 12.92
                } else {
                    1.055 * a.powf(1.0 / 2.4) - 0.055
                }
            }
            Transfer::Gamma(g) => s * a.powf(1.0 / g),
            Transfer::Bt709 => {
                s * if a < 0.018 {
                    a * 4.5
                } else {
                    1.099 * a.powf(0.45) - 0.099
                }
            }
            Transfer::Pq => {
                let y = (l.max(0.0) * PQ_WHITE_NITS / 10000.0).powf(PQ_M1);
                ((PQ_C1 + PQ_C2 * y) / (1.0 + PQ_C3 * y)).powf(PQ_M2)
            }
            Transfer::Hlg => {
                let e = (l.max(0.0) * HLG_WHITE).min(1.0);
                if e <= 1.0 / 12.0 {
                    (3.0 * e).sqrt()
                } else {
                    HLG_A * (12.0 * e - HLG_B).ln() + HLG_C
                }
            }
            Transfer::AcesCct => {
                if l <= 0.007_812_5 {
                    10.540_238 * l + 0.072_905_53
                } else {
                    (l.log2() + 9.72) / 17.52
                }
            }
        }
    }
}

/// primaries plus a transfer function
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ColorSpace {
    pub primaries: Primaries,
    pub transfer: Transfer,
}

impl ColorSpace {
    pub const SRGB: ColorSpace = ColorSpace::new(Primaries::Rec709, Transfer::Srgb);
    pub const LINEAR_SRGB: ColorSpace = ColorSpace::new(Primaries::Rec709, Transfer::Linear);
    pub const REC709: ColorSpace = ColorSpace::new(Primaries::Rec709, Transfer::Bt709);
    pub const REC2020: ColorSpace = ColorSpace::new(Primaries::Rec2020, Transfer::Bt709);
    pub const REC2020_PQ: ColorSpace = ColorSpace::new(Primaries::Rec2020, Transfer::Pq);
    pub const REC2020_HLG: ColorSpace = ColorSpace::new(Primaries::Rec2020, Transfer::Hlg);
    pub const DISPLAY_P3: ColorSpace = ColorSpace::new(Primaries::DisplayP3, Transfer::Srgb);
    pub const ADOBE_RGB: ColorSpace =
        ColorSpace::new(Primaries::AdobeRgb, Transfer::Gamma(563.0 / 256.0));
    pub const ACESCG: ColorSpace = ColorSpace::new(Primaries::AcesAp1, Transfer::Linear);
    pub const ACESCCT: ColorSpace = ColorSpace::new(Primaries::AcesAp1, Transfer::AcesCct);
    pub const ACES2065_1: ColorSpace = ColorSpace::new(Primaries::AcesAp0, Transfer::Linear);

    pub const fn new(primaries: Primaries, transfer: Transfer) -> Self {
        Self {
            primaries,
            transfer,
        }
    }

    pub fn linear(self) -> Self {
        Self::new(self.primaries, Transfer::Linear)
    }
}

/// where the render graph does its maths: the primaries of the working space and how
/// values are stored in its buffers.
///
/// linear storage is physically correct and what video, grading and 32 bit documents use.
/// photoshop blends 8 and 16 bit documents on gamma encoded values, so the image editor
/// picks srgb storage for those to match its results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkingSpace {
    pub primaries: Primaries,
    pub encoding: Transfer,
}

impl WorkingSpace {
    pub const LINEAR_SRGB: WorkingSpace = WorkingSpace {
        primaries: Primaries::Rec709,
        encoding: Transfer::Linear,
    };
    pub const SRGB: WorkingSpace = WorkingSpace {
        primaries: Primaries::Rec709,
        encoding: Transfer::Srgb,
    };
    pub const ACESCG: WorkingSpace = WorkingSpace {
        primaries: Primaries::AcesAp1,
        encoding: Transfer::Linear,
    };

    pub fn space(self) -> ColorSpace {
        ColorSpace::new(self.primaries, self.encoding)
    }
}

impl Default for WorkingSpace {
    fn default() -> Self {
        Self::LINEAR_SRGB
    }
}

/// a 3x3 matrix, row major
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat3(pub [[f64; 3]; 3]);

impl Mat3 {
    pub const IDENTITY: Mat3 = Mat3([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

    pub fn mul(&self, o: &Mat3) -> Mat3 {
        let mut r = [[0.0; 3]; 3];
        for (i, row) in r.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = (0..3).map(|k| self.0[i][k] * o.0[k][j]).sum();
            }
        }
        Mat3(r)
    }

    pub fn apply(&self, v: [f64; 3]) -> [f64; 3] {
        let m = &self.0;
        [
            m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
            m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
            m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
        ]
    }

    pub fn determinant(&self) -> f64 {
        let m = &self.0;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }

    /// the inverse, or identity for a singular matrix (never the case for real primaries)
    pub fn inverse(&self) -> Mat3 {
        let m = &self.0;
        let det = self.determinant();
        if det.abs() < 1e-12 {
            return Mat3::IDENTITY;
        }
        let d = 1.0 / det;
        Mat3([
            [
                (m[1][1] * m[2][2] - m[1][2] * m[2][1]) * d,
                (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * d,
                (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * d,
            ],
            [
                (m[1][2] * m[2][0] - m[1][0] * m[2][2]) * d,
                (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * d,
                (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * d,
            ],
            [
                (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * d,
                (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * d,
                (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * d,
            ],
        ])
    }

    pub fn is_identity(&self) -> bool {
        self.0
            .iter()
            .flatten()
            .zip(Mat3::IDENTITY.0.iter().flatten())
            .all(|(a, b)| (a - b).abs() < 1e-9)
    }

    pub fn to_f32(&self) -> [[f32; 3]; 3] {
        self.0.map(|row| row.map(|v| v as f32))
    }
}

/// bradford chromatic adaptation between two white points (as xyz)
pub fn bradford(from: [f64; 3], to: [f64; 3]) -> Mat3 {
    let b = Mat3([
        [0.8951, 0.2664, -0.1614],
        [-0.7502, 1.7135, 0.0367],
        [0.0389, -0.0685, 1.0296],
    ]);
    let (s, d) = (b.apply(from), b.apply(to));
    let scale = Mat3([
        [d[0] / s[0], 0.0, 0.0],
        [0.0, d[1] / s[1], 0.0],
        [0.0, 0.0, d[2] / s[2]],
    ]);
    b.inverse().mul(&scale).mul(&b)
}

/// linear rgb in one set of primaries to linear rgb in another, adapting the white point
pub fn primaries_matrix(from: Primaries, to: Primaries) -> Mat3 {
    if from == to {
        return Mat3::IDENTITY;
    }
    let mut m = from.to_xyz();
    if from.white() != to.white() {
        m = bradford(from.white(), to.white()).mul(&m);
    }
    to.to_xyz().inverse().mul(&m)
}

/// a 3d lookup table over [0, 1] cubed, red changing fastest
#[derive(Debug, Clone, PartialEq)]
pub struct Lut3d {
    pub size: u32,
    pub data: Vec<[f32; 3]>,
    /// identifies the table for cache keys
    pub id: u64,
}

impl Lut3d {
    /// builds a table by sampling a function on an even lattice
    pub fn sample(size: u32, mut f: impl FnMut([f32; 3]) -> [f32; 3]) -> Self {
        let n = size.max(2);
        let step = 1.0 / (n - 1) as f32;
        let mut data = Vec::with_capacity((n * n * n) as usize);
        for b in 0..n {
            for g in 0..n {
                for r in 0..n {
                    data.push(f([r as f32 * step, g as f32 * step, b as f32 * step]));
                }
            }
        }
        let id = hash_floats(data.iter().flatten().copied());
        Self { size: n, data, id }
    }

    fn at(&self, r: u32, g: u32, b: u32) -> [f32; 3] {
        let n = self.size;
        self.data[(r + g * n + b * n * n) as usize]
    }

    /// trilinear lookup, inputs clamped to the table
    pub fn apply(&self, c: [f32; 3]) -> [f32; 3] {
        let n = self.size;
        let max = (n - 1) as f32;
        let mut i0 = [0u32; 3];
        let mut i1 = [0u32; 3];
        let mut t = [0f32; 3];
        for k in 0..3 {
            let x = (c[k].clamp(0.0, 1.0)) * max;
            let f = x.floor().min(max - 1.0).max(0.0);
            i0[k] = f as u32;
            i1[k] = (i0[k] + 1).min(n - 1);
            t[k] = x - f;
        }
        let lerp = |a: [f32; 3], b: [f32; 3], t: f32| {
            [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ]
        };
        let c00 = lerp(
            self.at(i0[0], i0[1], i0[2]),
            self.at(i1[0], i0[1], i0[2]),
            t[0],
        );
        let c10 = lerp(
            self.at(i0[0], i1[1], i0[2]),
            self.at(i1[0], i1[1], i0[2]),
            t[0],
        );
        let c01 = lerp(
            self.at(i0[0], i0[1], i1[2]),
            self.at(i1[0], i0[1], i1[2]),
            t[0],
        );
        let c11 = lerp(
            self.at(i0[0], i1[1], i1[2]),
            self.at(i1[0], i1[1], i1[2]),
            t[0],
        );
        lerp(lerp(c00, c10, t[1]), lerp(c01, c11, t[1]), t[2])
    }
}

fn hash_floats(values: impl Iterator<Item = f32>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    for v in values {
        v.to_bits().hash(&mut h);
    }
    h.finish()
}

/// one step of a colour transform
#[derive(Debug, Clone, PartialEq)]
pub enum ColorStep {
    Decode(Transfer),
    Encode(Transfer),
    Matrix([[f32; 3]; 3]),
    Lut(Arc<Lut3d>),
    /// clamp to [0, 1]
    Clamp,
}

/// the most steps a transform may have, so it fits the kernel's parameter block
pub const MAX_COLOR_STEPS: usize = 6;

/// a list of colour steps that runs the same way on the cpu and the gpu. at most one
/// step may be a lut.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ColorTransform {
    pub steps: Vec<ColorStep>,
}

#[derive(Debug, thiserror::Error)]
pub enum ColorError {
    #[error("the colour profile could not be read: {0}")]
    Profile(String),
}

impl ColorTransform {
    pub fn identity() -> Self {
        Self::default()
    }

    /// converts values in one space to another
    pub fn between(from: ColorSpace, to: ColorSpace) -> Self {
        let mut steps = Vec::new();
        if from == to {
            return Self { steps };
        }
        let m = primaries_matrix(from.primaries, to.primaries);
        if from.transfer != Transfer::Linear {
            steps.push(ColorStep::Decode(from.transfer));
        }
        if !m.is_identity() {
            steps.push(ColorStep::Matrix(m.to_f32()));
        }
        if to.transfer != Transfer::Linear {
            steps.push(ColorStep::Encode(to.transfer));
        }
        Self { steps }
    }

    pub fn is_identity(&self) -> bool {
        self.steps.is_empty()
    }

    pub fn then(mut self, step: ColorStep) -> Self {
        self.steps.push(step);
        self
    }

    pub fn lut(&self) -> Option<&Arc<Lut3d>> {
        self.steps.iter().find_map(|s| match s {
            ColorStep::Lut(l) => Some(l),
            _ => None,
        })
    }

    pub fn apply(&self, mut c: [f32; 3]) -> [f32; 3] {
        for step in &self.steps {
            c = match step {
                ColorStep::Decode(t) => c.map(|v| t.decode(v)),
                ColorStep::Encode(t) => c.map(|v| t.encode(v)),
                ColorStep::Matrix(m) => [
                    m[0][0] * c[0] + m[0][1] * c[1] + m[0][2] * c[2],
                    m[1][0] * c[0] + m[1][1] * c[1] + m[1][2] * c[2],
                    m[2][0] * c[0] + m[2][1] * c[1] + m[2][2] * c[2],
                ],
                ColorStep::Lut(l) => l.apply(c),
                ColorStep::Clamp => c.map(|v| v.clamp(0.0, 1.0)),
            };
        }
        c
    }

    /// packs the steps for the wgsl `color_steps` function: one vec4 with the count, then
    /// four vec4s per step (kind, transfer id, gamma, then three matrix rows).
    pub fn pack(&self, out: &mut Vec<[f32; 4]>) {
        let steps = &self.steps[..self.steps.len().min(MAX_COLOR_STEPS)];
        out.push([steps.len() as f32, 0.0, 0.0, 0.0]);
        for step in steps {
            let (kind, transfer, m) = match step {
                ColorStep::Decode(t) => (1.0, Some(*t), None),
                ColorStep::Encode(t) => (2.0, Some(*t), None),
                ColorStep::Matrix(m) => (3.0, None, Some(*m)),
                ColorStep::Lut(_) => (4.0, None, None),
                ColorStep::Clamp => (5.0, None, None),
            };
            let (id, gamma) = transfer.map_or((0.0, 1.0), |t| (t.id() as f32, t.gamma()));
            out.push([kind, id, gamma, 0.0]);
            let m = m.unwrap_or([[0.0; 3]; 3]);
            for row in m {
                out.push([row[0], row[1], row[2], 0.0]);
            }
        }
    }

    /// a stable key for caches
    pub fn key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        for step in &self.steps {
            match step {
                ColorStep::Decode(t) => (1u8, t).hash(&mut h),
                ColorStep::Encode(t) => (2u8, t).hash(&mut h),
                ColorStep::Matrix(m) => {
                    3u8.hash(&mut h);
                    for v in m.iter().flatten() {
                        v.to_bits().hash(&mut h);
                    }
                }
                ColorStep::Lut(l) => (4u8, l.id).hash(&mut h),
                ColorStep::Clamp => 5u8.hash(&mut h),
            }
        }
        h.finish()
    }
}

/// lattice size for icc luts. 33 points is what most colour tools use.
const ICC_LUT_SIZE: u32 = 33;

/// a linear rec.2020 profile, the wide gamut hand off between moxcms and vellum's own maths
fn linear_rec2020_profile() -> moxcms::ColorProfile {
    let mut p = moxcms::ColorProfile::new_bt2020();
    let linear = moxcms::ToneReprCurve::Parametric(vec![1.0]);
    p.red_trc = Some(linear.clone());
    p.green_trc = Some(linear.clone());
    p.blue_trc = Some(linear);
    p.cicp = None;
    p
}

/// an icc profile read from an image or from the display
#[derive(Clone)]
pub struct IccProfile {
    profile: Arc<moxcms::ColorProfile>,
    pub description: Option<String>,
}

impl std::fmt::Debug for IccProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IccProfile")
            .field("description", &self.description)
            .finish()
    }
}

impl IccProfile {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorError> {
        let profile = moxcms::ColorProfile::new_from_slice(bytes)
            .map_err(|e| ColorError::Profile(format!("{e:?}")))?;
        let description = profile.description.as_ref().and_then(|d| match d {
            moxcms::ProfileText::PlainString(s) => Some(s.clone()),
            moxcms::ProfileText::Localizable(v) => v.first().map(|s| s.value.clone()),
            _ => None,
        });
        Ok(Self {
            profile: Arc::new(profile),
            description,
        })
    }

    pub fn srgb() -> Self {
        Self {
            profile: Arc::new(moxcms::ColorProfile::new_srgb()),
            description: Some("sRGB IEC61966-2.1".into()),
        }
    }

    pub fn display_p3() -> Self {
        Self {
            profile: Arc::new(moxcms::ColorProfile::new_display_p3()),
            description: Some("Display P3".into()),
        }
    }

    /// pixels in this profile to the working space, as a lut over the encoded values
    pub fn to_working(&self, working: WorkingSpace) -> Result<ColorTransform, ColorError> {
        let dst = linear_rec2020_profile();
        let t = self
            .profile
            .create_transform_f32(
                moxcms::Layout::Rgb,
                &dst,
                moxcms::Layout::Rgb,
                moxcms::TransformOptions::default(),
            )
            .map_err(|e| ColorError::Profile(format!("{e:?}")))?;
        let after = ColorTransform::between(
            ColorSpace::new(Primaries::Rec2020, Transfer::Linear),
            working.space(),
        );
        let mut failed = None;
        let lut = Lut3d::sample(ICC_LUT_SIZE, |c| {
            let mut out = [0.0f32; 3];
            if let Err(e) = t.transform(&c, &mut out) {
                failed = Some(e);
            }
            after.apply(out)
        });
        if let Some(e) = failed {
            return Err(ColorError::Profile(format!("{e:?}")));
        }
        Ok(ColorTransform {
            steps: vec![ColorStep::Lut(Arc::new(lut))],
        })
    }

    /// working space values to this profile, for a display or an export. values are
    /// clamped to the displayable range and shaped with the srgb curve before the lut so
    /// dark tones keep their precision.
    pub fn from_working(&self, working: WorkingSpace) -> Result<ColorTransform, ColorError> {
        let src = linear_rec2020_profile();
        let t = src
            .create_transform_f32(
                moxcms::Layout::Rgb,
                &self.profile,
                moxcms::Layout::Rgb,
                moxcms::TransformOptions::default(),
            )
            .map_err(|e| ColorError::Profile(format!("{e:?}")))?;
        let to_2020 = ColorTransform::between(
            ColorSpace::new(working.primaries, Transfer::Linear),
            ColorSpace::new(Primaries::Rec2020, Transfer::Linear),
        );
        let mut failed = None;
        let lut = Lut3d::sample(ICC_LUT_SIZE, |c| {
            let lin = to_2020.apply(c.map(|v| Transfer::Srgb.decode(v)));
            let mut out = [0.0f32; 3];
            if let Err(e) = t.transform(&lin, &mut out) {
                failed = Some(e);
            }
            out
        });
        if let Some(e) = failed {
            return Err(ColorError::Profile(format!("{e:?}")));
        }
        let mut steps = Vec::new();
        if working.encoding != Transfer::Linear {
            steps.push(ColorStep::Decode(working.encoding));
        }
        steps.push(ColorStep::Clamp);
        steps.push(ColorStep::Encode(Transfer::Srgb));
        steps.push(ColorStep::Lut(Arc::new(lut)));
        Ok(ColorTransform { steps })
    }
}

/// rec.709 luma weights, used by luminosity masks and the hue and saturation maths
pub const LUMA_709: [f32; 3] = [0.2126, 0.7152, 0.0722];

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn srgb_to_xyz_matches_the_standard() {
        let m = Primaries::Rec709.to_xyz().0;
        let expected = [
            [0.4124, 0.3576, 0.1805],
            [0.2126, 0.7152, 0.0722],
            [0.0193, 0.1192, 0.9505],
        ];
        for i in 0..3 {
            for j in 0..3 {
                assert!(close(m[i][j], expected[i][j], 2e-4), "{i},{j}: {}", m[i][j]);
            }
        }
    }

    #[test]
    fn srgb_to_acescg_matches_the_published_bradford_matrix() {
        let m = primaries_matrix(Primaries::Rec709, Primaries::AcesAp1).0;
        let expected = [
            [0.6131, 0.3395, 0.0474],
            [0.0702, 0.9164, 0.0134],
            [0.0206, 0.1096, 0.8698],
        ];
        for i in 0..3 {
            for j in 0..3 {
                assert!(close(m[i][j], expected[i][j], 5e-4), "{i},{j}: {}", m[i][j]);
            }
        }
    }

    #[test]
    fn p3_to_srgb_matches_the_published_matrix() {
        let m = primaries_matrix(Primaries::DisplayP3, Primaries::Rec709).0;
        let expected = [
            [1.2249, -0.2247, 0.0],
            [-0.0420, 1.0419, 0.0],
            [-0.0197, -0.0786, 1.0979],
        ];
        for i in 0..3 {
            for j in 0..3 {
                assert!(close(m[i][j], expected[i][j], 5e-4), "{i},{j}: {}", m[i][j]);
            }
        }
    }

    #[test]
    fn primaries_round_trip() {
        for p in [
            Primaries::DisplayP3,
            Primaries::Rec2020,
            Primaries::AcesAp0,
            Primaries::AdobeRgb,
        ] {
            let m =
                primaries_matrix(Primaries::Rec709, p).mul(&primaries_matrix(p, Primaries::Rec709));
            assert!(
                m.is_identity()
                    || m.0
                        .iter()
                        .flatten()
                        .zip(Mat3::IDENTITY.0.iter().flatten())
                        .all(|(a, b)| close(*a, *b, 1e-9))
            );
        }
    }

    #[test]
    fn white_stays_white() {
        let t = ColorTransform::between(ColorSpace::SRGB, ColorSpace::ACESCG);
        let w = t.apply([1.0, 1.0, 1.0]);
        for v in w {
            assert!((v - 1.0).abs() < 1e-4, "{w:?}");
        }
    }

    #[test]
    fn transfer_functions_round_trip() {
        let transfers = [
            Transfer::Srgb,
            Transfer::Gamma(2.2),
            Transfer::Bt709,
            Transfer::Pq,
            Transfer::Hlg,
            Transfer::AcesCct,
        ];
        for t in transfers {
            for i in 0..=50 {
                let v = i as f32 / 50.0;
                let back = t.encode(t.decode(v));
                assert!((back - v).abs() < 2e-4, "{t:?} {v} -> {back}");
            }
        }
    }

    #[test]
    fn known_transfer_values() {
        assert!((Transfer::Srgb.decode(0.5) - 0.214_041).abs() < 1e-5);
        assert!((Transfer::Srgb.encode(0.18) - 0.461_356).abs() < 1e-5);
        // acescct: 0.18 linear is 0.4135884
        assert!((Transfer::AcesCct.encode(0.18) - 0.413_588_4).abs() < 1e-5);
        // pq: 203 nits (linear 1.0) is a 58 percent signal
        assert!((Transfer::Pq.encode(1.0) - 0.580_7).abs() < 2e-3);
        // hlg: reference white sits at 75 percent
        assert!((Transfer::Hlg.encode(1.0) - 0.75).abs() < 1e-3);
    }

    #[test]
    fn transforms_pack_into_a_fixed_layout() {
        let t = ColorTransform::between(ColorSpace::SRGB, ColorSpace::ACESCCT);
        let mut out = Vec::new();
        t.pack(&mut out);
        assert_eq!(out[0][0], 3.0);
        assert_eq!(out.len(), 1 + 3 * 4);
        assert_eq!(out[1][0], 1.0);
        assert_eq!(out[5][0], 3.0);
        assert_eq!(out[9][1], Transfer::AcesCct.id() as f32);
    }

    #[test]
    fn luts_interpolate_and_hit_their_points() {
        let lut = Lut3d::sample(17, |c| [c[0] * c[0], c[1], 1.0 - c[2]]);
        let out = lut.apply([0.5, 0.25, 0.75]);
        assert!((out[0] - 0.25).abs() < 2e-3);
        assert!((out[1] - 0.25).abs() < 1e-6);
        assert!((out[2] - 0.25).abs() < 1e-6);
    }

    #[test]
    fn srgb_icc_matches_the_analytic_transform() {
        let icc = IccProfile::srgb();
        let lut = icc.to_working(WorkingSpace::LINEAR_SRGB).unwrap();
        let analytic = ColorTransform::between(ColorSpace::SRGB, ColorSpace::LINEAR_SRGB);
        for c in [[0.2, 0.5, 0.9], [1.0, 0.0, 0.0], [0.5, 0.5, 0.5]] {
            let a = lut.apply(c);
            let b = analytic.apply(c);
            for k in 0..3 {
                assert!((a[k] - b[k]).abs() < 4e-3, "{c:?}: {a:?} vs {b:?}");
            }
        }
        let back = icc.from_working(WorkingSpace::LINEAR_SRGB).unwrap();
        let round = back.apply(analytic.apply([0.3, 0.6, 0.1]));
        for (k, v) in [0.3, 0.6, 0.1].iter().enumerate() {
            assert!((round[k] - v).abs() < 4e-3, "{round:?}");
        }
    }
}

//! typed node parameters. every node declares its parameters as a static list, which the
//! ui reads to build controls and later phases use to keyframe values. a parameter's value
//! at render time is plain data: animation is resolved before the graph is built.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::color::{ColorSpace, ColorTransform, WorkingSpace};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Unit {
    None,
    /// document pixels, scaled with the zoom level
    Pixels,
    Percent,
    Degrees,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChoiceOption {
    pub id: &'static str,
    pub label: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GradientStop {
    /// 0 to 1 along the gradient
    pub offset: f32,
    /// straight srgb colour with alpha
    pub color: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase", tag = "type")]
#[ts(export)]
pub enum ParamKind {
    Float {
        min: f32,
        max: f32,
        default: f32,
        step: f32,
        unit: Unit,
    },
    Int {
        min: i32,
        max: i32,
        default: i32,
    },
    Bool {
        default: bool,
    },
    Choice {
        options: &'static [ChoiceOption],
        default: &'static str,
    },
    /// a straight srgb colour with alpha
    Color {
        default: [f32; 4],
    },
    /// a position in document pixels
    Point {
        default: [f32; 2],
    },
    /// a 3x3 row major matrix in document pixels, for transforms and perspective
    Matrix {
        default: [f32; 9],
    },
    Gradient {
        default: &'static [GradientStop],
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ParamSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub kind: ParamKind,
    /// whether a later phase may keyframe it
    pub animatable: bool,
}

impl ParamSpec {
    pub const fn float(
        id: &'static str,
        label: &'static str,
        min: f32,
        max: f32,
        default: f32,
    ) -> Self {
        Self {
            id,
            label,
            kind: ParamKind::Float {
                min,
                max,
                default,
                step: 0.01,
                unit: Unit::None,
            },
            animatable: true,
        }
    }

    pub const fn pixels(id: &'static str, label: &'static str, max: f32, default: f32) -> Self {
        Self {
            id,
            label,
            kind: ParamKind::Float {
                min: 0.0,
                max,
                default,
                step: 0.1,
                unit: Unit::Pixels,
            },
            animatable: true,
        }
    }

    pub const fn percent(
        id: &'static str,
        label: &'static str,
        min: f32,
        max: f32,
        default: f32,
    ) -> Self {
        Self {
            id,
            label,
            kind: ParamKind::Float {
                min,
                max,
                default,
                step: 1.0,
                unit: Unit::Percent,
            },
            animatable: true,
        }
    }

    pub const fn degrees(
        id: &'static str,
        label: &'static str,
        min: f32,
        max: f32,
        default: f32,
    ) -> Self {
        Self {
            id,
            label,
            kind: ParamKind::Float {
                min,
                max,
                default,
                step: 1.0,
                unit: Unit::Degrees,
            },
            animatable: true,
        }
    }

    pub const fn int(
        id: &'static str,
        label: &'static str,
        min: i32,
        max: i32,
        default: i32,
    ) -> Self {
        Self {
            id,
            label,
            kind: ParamKind::Int { min, max, default },
            animatable: false,
        }
    }

    pub const fn boolean(id: &'static str, label: &'static str, default: bool) -> Self {
        Self {
            id,
            label,
            kind: ParamKind::Bool { default },
            animatable: false,
        }
    }

    pub const fn choice(
        id: &'static str,
        label: &'static str,
        options: &'static [ChoiceOption],
        default: &'static str,
    ) -> Self {
        Self {
            id,
            label,
            kind: ParamKind::Choice { options, default },
            animatable: false,
        }
    }

    pub const fn color(id: &'static str, label: &'static str, default: [f32; 4]) -> Self {
        Self {
            id,
            label,
            kind: ParamKind::Color { default },
            animatable: true,
        }
    }

    pub const fn point(id: &'static str, label: &'static str, default: [f32; 2]) -> Self {
        Self {
            id,
            label,
            kind: ParamKind::Point { default },
            animatable: true,
        }
    }

    pub const fn matrix(id: &'static str, label: &'static str) -> Self {
        Self {
            id,
            label,
            kind: ParamKind::Matrix {
                default: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            },
            animatable: true,
        }
    }

    pub const fn gradient(
        id: &'static str,
        label: &'static str,
        default: &'static [GradientStop],
    ) -> Self {
        Self {
            id,
            label,
            kind: ParamKind::Gradient { default },
            animatable: true,
        }
    }

    pub fn default_value(&self) -> ParamValue {
        match self.kind {
            ParamKind::Float { default, .. } => ParamValue::Float(default),
            ParamKind::Int { default, .. } => ParamValue::Int(default),
            ParamKind::Bool { default } => ParamValue::Bool(default),
            ParamKind::Choice { default, .. } => ParamValue::Choice(default.to_owned()),
            ParamKind::Color { default } => ParamValue::Color(default),
            ParamKind::Point { default } => ParamValue::Point(default),
            ParamKind::Matrix { default } => ParamValue::Matrix(default),
            ParamKind::Gradient { default } => ParamValue::Gradient(default.to_vec()),
        }
    }

    /// clamps or replaces a value that does not fit this parameter
    pub fn sanitize(&self, value: ParamValue) -> ParamValue {
        match (self.kind, value) {
            (ParamKind::Float { min, max, .. }, ParamValue::Float(v)) if v.is_finite() => {
                ParamValue::Float(v.clamp(min, max))
            }
            (ParamKind::Float { min, max, .. }, ParamValue::Int(v)) => {
                ParamValue::Float((v as f32).clamp(min, max))
            }
            (ParamKind::Int { min, max, .. }, ParamValue::Int(v)) => {
                ParamValue::Int(v.clamp(min, max))
            }
            (ParamKind::Int { min, max, .. }, ParamValue::Float(v)) if v.is_finite() => {
                ParamValue::Int((v.round() as i32).clamp(min, max))
            }
            (ParamKind::Bool { .. }, v @ ParamValue::Bool(_)) => v,
            (ParamKind::Choice { options, .. }, ParamValue::Choice(c))
                if options.iter().any(|o| o.id == c) =>
            {
                ParamValue::Choice(c)
            }
            (ParamKind::Color { .. }, ParamValue::Color(c)) if c.iter().all(|v| v.is_finite()) => {
                ParamValue::Color(c)
            }
            (ParamKind::Point { .. }, ParamValue::Point(p)) if p.iter().all(|v| v.is_finite()) => {
                ParamValue::Point(p)
            }
            (ParamKind::Matrix { .. }, ParamValue::Matrix(m))
                if m.iter().all(|v| v.is_finite()) =>
            {
                ParamValue::Matrix(m)
            }
            (ParamKind::Gradient { .. }, ParamValue::Gradient(mut stops)) if !stops.is_empty() => {
                stops.retain(|s| s.offset.is_finite());
                stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
                ParamValue::Gradient(stops)
            }
            _ => self.default_value(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", tag = "type", content = "value")]
#[ts(export)]
pub enum ParamValue {
    Float(f32),
    Int(i32),
    Bool(bool),
    Choice(String),
    Color([f32; 4]),
    Point([f32; 2]),
    Matrix([f32; 9]),
    Gradient(Vec<GradientStop>),
}

impl ParamValue {
    fn hash_into(&self, h: &mut dyn Hasher) {
        let mut put = |v: f32| h.write_u32(v.to_bits());
        match self {
            ParamValue::Float(v) => put(*v),
            ParamValue::Int(v) => put(*v as f32),
            ParamValue::Bool(v) => put(f32::from(u8::from(*v))),
            ParamValue::Choice(c) => {
                for b in c.bytes() {
                    put(f32::from(b));
                }
            }
            ParamValue::Color(c) => c.iter().for_each(|v| put(*v)),
            ParamValue::Point(p) => p.iter().for_each(|v| put(*v)),
            ParamValue::Matrix(m) => m.iter().for_each(|v| put(*v)),
            ParamValue::Gradient(stops) => {
                for s in stops {
                    put(s.offset);
                    s.color.iter().for_each(|v| put(*v));
                }
            }
        }
    }
}

impl From<f32> for ParamValue {
    fn from(v: f32) -> Self {
        ParamValue::Float(v)
    }
}

impl From<i32> for ParamValue {
    fn from(v: i32) -> Self {
        ParamValue::Int(v)
    }
}

impl From<bool> for ParamValue {
    fn from(v: bool) -> Self {
        ParamValue::Bool(v)
    }
}

impl From<&str> for ParamValue {
    fn from(v: &str) -> Self {
        ParamValue::Choice(v.to_owned())
    }
}

impl From<[f32; 4]> for ParamValue {
    fn from(v: [f32; 4]) -> Self {
        ParamValue::Color(v)
    }
}

impl From<[f32; 2]> for ParamValue {
    fn from(v: [f32; 2]) -> Self {
        ParamValue::Point(v)
    }
}

impl From<[f32; 9]> for ParamValue {
    fn from(v: [f32; 9]) -> Self {
        ParamValue::Matrix(v)
    }
}

impl From<Vec<GradientStop>> for ParamValue {
    fn from(v: Vec<GradientStop>) -> Self {
        ParamValue::Gradient(v)
    }
}

/// the values set on one node. unset parameters read as their default.
pub type ParamMap = BTreeMap<String, ParamValue>;

/// typed read access to a node's parameters
#[derive(Clone, Copy)]
pub struct Params<'a> {
    pub specs: &'static [ParamSpec],
    pub values: &'a ParamMap,
}

impl<'a> Params<'a> {
    fn get(&self, id: &str) -> ParamValue {
        let spec = self.specs.iter().find(|s| s.id == id);
        match (spec, self.values.get(id)) {
            (Some(spec), Some(v)) => spec.sanitize(v.clone()),
            (Some(spec), None) => spec.default_value(),
            (None, Some(v)) => v.clone(),
            (None, None) => ParamValue::Float(0.0),
        }
    }

    pub fn float(&self, id: &str) -> f32 {
        match self.get(id) {
            ParamValue::Float(v) => v,
            ParamValue::Int(v) => v as f32,
            ParamValue::Bool(b) => f32::from(u8::from(b)),
            _ => 0.0,
        }
    }

    pub fn int(&self, id: &str) -> i32 {
        match self.get(id) {
            ParamValue::Int(v) => v,
            ParamValue::Float(v) => v.round() as i32,
            _ => 0,
        }
    }

    pub fn bool(&self, id: &str) -> bool {
        matches!(self.get(id), ParamValue::Bool(true))
    }

    pub fn choice(&self, id: &str) -> String {
        match self.get(id) {
            ParamValue::Choice(c) => c,
            _ => String::new(),
        }
    }

    /// the index of the selected option, for packing into shaders
    pub fn choice_index(&self, id: &str) -> u32 {
        let chosen = self.choice(id);
        let spec = self.specs.iter().find(|s| s.id == id);
        if let Some(ParamSpec {
            kind: ParamKind::Choice { options, .. },
            ..
        }) = spec
        {
            return options.iter().position(|o| o.id == chosen).unwrap_or(0) as u32;
        }
        0
    }

    pub fn color(&self, id: &str) -> [f32; 4] {
        match self.get(id) {
            ParamValue::Color(c) => c,
            _ => [0.0; 4],
        }
    }

    pub fn point(&self, id: &str) -> [f32; 2] {
        match self.get(id) {
            ParamValue::Point(p) => p,
            _ => [0.0; 2],
        }
    }

    pub fn matrix(&self, id: &str) -> [f32; 9] {
        match self.get(id) {
            ParamValue::Matrix(m) => m,
            _ => [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        }
    }

    pub fn gradient(&self, id: &str) -> Vec<GradientStop> {
        match self.get(id) {
            ParamValue::Gradient(g) => g,
            _ => Vec::new(),
        }
    }

    /// feeds every effective value into a hasher, in declaration order
    pub fn hash_into(&self, h: &mut dyn Hasher) {
        for spec in self.specs {
            spec.id.hash(&mut HasherRef(h));
            self.get(spec.id).hash_into(h);
        }
    }
}

/// lets `Hash::hash` write into a `&mut dyn Hasher`
struct HasherRef<'a>(&'a mut dyn Hasher);

impl Hasher for HasherRef<'_> {
    fn finish(&self) -> u64 {
        self.0.finish()
    }
    fn write(&mut self, bytes: &[u8]) {
        self.0.write(bytes);
    }
}

/// an srgb colour parameter in working space values, still straight
pub fn color_to_working(c: [f32; 4], working: WorkingSpace) -> [f32; 4] {
    let t = ColorTransform::between(ColorSpace::SRGB, working.space());
    let rgb = t.apply([c[0], c[1], c[2]]);
    [rgb[0], rgb[1], rgb[2], c[3]]
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPECS: &[ParamSpec] = &[
        ParamSpec::pixels("radius", "Radius", 250.0, 4.0),
        ParamSpec::choice(
            "mode",
            "Mode",
            &[
                ChoiceOption {
                    id: "a",
                    label: "A",
                },
                ChoiceOption {
                    id: "b",
                    label: "B",
                },
            ],
            "a",
        ),
    ];

    #[test]
    fn defaults_and_clamping() {
        let mut values = ParamMap::new();
        let p = Params {
            specs: SPECS,
            values: &values,
        };
        assert_eq!(p.float("radius"), 4.0);
        assert_eq!(p.choice_index("mode"), 0);
        values.insert("radius".into(), ParamValue::Float(9999.0));
        values.insert("mode".into(), "b".into());
        let p = Params {
            specs: SPECS,
            values: &values,
        };
        assert_eq!(p.float("radius"), 250.0);
        assert_eq!(p.choice_index("mode"), 1);
        values.insert("mode".into(), "nope".into());
        let p = Params {
            specs: SPECS,
            values: &values,
        };
        assert_eq!(p.choice("mode"), "a");
    }

    #[test]
    fn hashes_follow_effective_values() {
        let hash = |values: &ParamMap| {
            let mut h = std::hash::DefaultHasher::new();
            Params {
                specs: SPECS,
                values,
            }
            .hash_into(&mut h);
            h.finish()
        };
        let empty = ParamMap::new();
        let mut explicit = ParamMap::new();
        explicit.insert("radius".into(), ParamValue::Float(4.0));
        assert_eq!(hash(&empty), hash(&explicit));
        explicit.insert("radius".into(), ParamValue::Float(5.0));
        assert_ne!(hash(&empty), hash(&explicit));
    }

    #[test]
    fn values_serialise_with_a_type_tag() {
        let v = ParamValue::Float(1.5);
        assert_eq!(
            serde_json::to_string(&v).unwrap(),
            r#"{"type":"float","value":1.5}"#
        );
        let back: ParamValue = serde_json::from_str(r#"{"type":"choice","value":"b"}"#).unwrap();
        assert_eq!(back, ParamValue::Choice("b".into()));
    }
}

import type { BlendMode } from "../../bindings/BlendMode";
import type { Label } from "../../bindings/Label";

export const BLEND_MODES: [BlendMode, string][] = [
  ["normal", "Normal"],
  ["dissolve", "Dissolve"],
  ["darken", "Darken"],
  ["multiply", "Multiply"],
  ["colorBurn", "Color Burn"],
  ["linearBurn", "Linear Burn"],
  ["darkerColor", "Darker Color"],
  ["lighten", "Lighten"],
  ["screen", "Screen"],
  ["colorDodge", "Color Dodge"],
  ["linearDodge", "Linear Dodge (Add)"],
  ["lighterColor", "Lighter Color"],
  ["overlay", "Overlay"],
  ["softLight", "Soft Light"],
  ["hardLight", "Hard Light"],
  ["vividLight", "Vivid Light"],
  ["linearLight", "Linear Light"],
  ["pinLight", "Pin Light"],
  ["hardMix", "Hard Mix"],
  ["difference", "Difference"],
  ["exclusion", "Exclusion"],
  ["subtract", "Subtract"],
  ["divide", "Divide"],
  ["hue", "Hue"],
  ["saturation", "Saturation"],
  ["color", "Color"],
  ["luminosity", "Luminosity"],
];

export const LABELS: [Label, string, string][] = [
  ["none", "No colour", "transparent"],
  ["red", "Red", "#e5484d"],
  ["orange", "Orange", "#f76b15"],
  ["yellow", "Yellow", "#ffc53d"],
  ["green", "Green", "#30a46c"],
  ["blue", "Blue", "#3e63dd"],
  ["violet", "Violet", "#8e4ec6"],
  ["gray", "Gray", "#8b8d98"],
];

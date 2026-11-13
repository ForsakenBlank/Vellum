import { Slider as Radix } from "radix-ui";

interface SliderProps {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  /** called while dragging */
  onChange: (value: number) => void;
  /** called once on release, for saving */
  onCommit?: (value: number) => void;
  id?: string;
}

export function Slider({ label, value, min, max, step, onChange, onCommit, id }: SliderProps) {
  return (
    <Radix.Root
      id={id}
      aria-label={label}
      value={[value]}
      min={min}
      max={max}
      step={step}
      onValueChange={([next]) => {
        if (next !== undefined) onChange(next);
      }}
      onValueCommit={([next]) => {
        if (next !== undefined) onCommit?.(next);
      }}
      className="relative flex h-5 w-40 touch-none items-center select-none"
    >
      <Radix.Track className="relative h-1 grow rounded-full bg-raised">
        <Radix.Range className="absolute h-full rounded-full bg-accent" />
      </Radix.Track>
      <Radix.Thumb
        aria-label={label}
        className="v-focus block size-3.5 rounded-full border-2 border-accent bg-surface shadow"
      />
    </Radix.Root>
  );
}

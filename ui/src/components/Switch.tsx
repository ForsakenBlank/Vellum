import { Switch as RadixSwitch } from "radix-ui";

interface SwitchProps {
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  label: string;
  disabled?: boolean;
  id?: string;
}

export function Switch({ checked, onCheckedChange, label, disabled, id }: SwitchProps) {
  return (
    <RadixSwitch.Root
      id={id}
      checked={checked}
      onCheckedChange={onCheckedChange}
      disabled={disabled}
      aria-label={label}
      className="relative inline-flex h-5 w-9 shrink-0 items-center rounded-full border border-line-strong bg-pressed transition-colors duration-(--motion-fast) data-[state=checked]:border-transparent data-[state=checked]:bg-accent disabled:opacity-45"
    >
      <RadixSwitch.Thumb className="block size-3.5 translate-x-0.5 rounded-full bg-fg shadow-1 transition-transform duration-(--motion-fast) ease-calm data-[state=checked]:translate-x-[18px] data-[state=checked]:bg-accent-fg" />
    </RadixSwitch.Root>
  );
}

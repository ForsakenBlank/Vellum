import { Switch as Radix } from "radix-ui";

interface SwitchProps {
  label: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
  id?: string;
}

export function Switch({ label, checked, disabled, onChange, id }: SwitchProps) {
  return (
    <Radix.Root
      id={id}
      aria-label={label}
      checked={checked}
      disabled={disabled}
      onCheckedChange={onChange}
      className="v-focus relative inline-flex h-5 w-9 shrink-0 items-center rounded-full border border-line bg-raised transition-colors duration-(--v-fast) data-[state=checked]:border-accent data-[state=checked]:bg-accent disabled:opacity-50"
    >
      <Radix.Thumb className="block size-3.5 translate-x-0.5 rounded-full bg-ink-soft shadow transition-transform duration-(--v-fast) data-[state=checked]:translate-x-[1.125rem] data-[state=checked]:bg-on-accent" />
    </Radix.Root>
  );
}

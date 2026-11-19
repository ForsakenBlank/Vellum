import { chordParts, isMac } from "../lib/keys";

interface KbdProps {
  chord: string;
  className?: string;
}

/** a shortcut drawn as key caps, using symbols on macos */
export function Kbd({ chord, className = "" }: KbdProps) {
  const parts = chordParts(chord);
  if (isMac) {
    return <kbd className={`kbd ${className}`}>{parts.join("")}</kbd>;
  }
  return (
    <span className={`inline-flex items-center gap-0.5 ${className}`}>
      {parts.map((part, i) => (
        <kbd key={i} className="kbd">
          {part}
        </kbd>
      ))}
    </span>
  );
}

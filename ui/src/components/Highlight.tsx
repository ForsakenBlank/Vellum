interface HighlightProps {
  text: string;
  /** indexes of characters to mark, from the fuzzy matcher */
  positions: number[];
}

/** text with fuzzy matched characters picked out */
export function Highlight({ text, positions }: HighlightProps) {
  if (positions.length === 0) return <>{text}</>;
  const marked = new Set(positions);
  const pieces: { text: string; on: boolean }[] = [];
  for (let i = 0; i < text.length; i++) {
    const on = marked.has(i);
    const last = pieces[pieces.length - 1];
    if (last && last.on === on) last.text += text.charAt(i);
    else pieces.push({ text: text.charAt(i), on });
  }
  return (
    <>
      {pieces.map((piece, i) =>
        piece.on ? (
          <mark key={i} className="bg-transparent font-semibold text-accent-text">
            {piece.text}
          </mark>
        ) : (
          <span key={i}>{piece.text}</span>
        ),
      )}
    </>
  );
}

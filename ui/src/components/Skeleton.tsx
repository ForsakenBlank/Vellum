import { clsx } from "clsx";

/** a grey block that stands in for content that is still loading */
export function Skeleton({ className }: { className?: string }) {
  return <div aria-hidden className={clsx("v-skeleton rounded-md bg-raised", className)} />;
}

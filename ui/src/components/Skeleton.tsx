interface SkeletonProps {
  className?: string;
}

/** a shimmering placeholder shown while something loads */
export function Skeleton({ className = "" }: SkeletonProps) {
  return <div aria-hidden className={`skeleton rounded-md ${className}`} />;
}

/** A single strip of headline numbers; cells share borders so the row reads as one summary. */
export function MetricStrip({ metrics }: { metrics: { label: string; value: number | null }[] }) {
  return (
    <dl className="mb-6 grid grid-cols-2 gap-px overflow-hidden rounded-lg border bg-border shadow-xs @3xl/main:grid-cols-4">
      {metrics.map(({ label, value }) => (
        <div key={label} className="min-w-0 bg-card px-5 py-4">
          <dt className="text-sm text-muted-foreground">{label}</dt>
          <dd className="mt-1 text-2xl font-semibold break-all tabular-nums">{value ?? "--"}</dd>
        </div>
      ))}
    </dl>
  );
}

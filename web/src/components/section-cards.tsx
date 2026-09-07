import { Card, CardDescription, CardFooter, CardHeader, CardTitle } from "@/components/ui/card";

// The dashboard-01 summary cards display current capacity, without demo trend data.
export function SectionCards({
  metrics,
}: {
  metrics: { label: string; value: number | null; detail?: string }[];
}) {
  return (
    <div className="mb-6 grid grid-cols-1 gap-4 *:data-[slot=card]:shadow-xs @xl/main:grid-cols-2 @5xl/main:grid-cols-4">
      {metrics.map(({ label, value, detail }) => (
        <Card key={label} className="@container/card min-w-0 rounded-lg">
          <CardHeader>
            <CardDescription>{label}</CardDescription>
            <CardTitle className="text-2xl font-semibold break-all tabular-nums">
              {value ?? "--"}
            </CardTitle>
          </CardHeader>
          {detail && (
            <CardFooter className="flex-col items-start gap-1.5 text-sm">
              <div className="text-muted-foreground">{detail}</div>
            </CardFooter>
          )}
        </Card>
      ))}
    </div>
  );
}

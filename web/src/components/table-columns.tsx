import { TableHead, TableHeader, TableRow } from "./ui/table";

/** A plain-text table header: one column per label, in order. */
export function TableColumns({ labels }: { labels: readonly string[] }) {
  return (
    <TableHeader>
      <TableRow>
        {labels.map((label) => (
          <TableHead key={label}>{label}</TableHead>
        ))}
      </TableRow>
    </TableHeader>
  );
}

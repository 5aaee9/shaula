import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import type { AccountBinding, AuthRevisionState } from "@/lib/types";

function healthLabel(binding: AccountBinding, now: number) {
  if (binding.valid_until_ms != null && binding.valid_until_ms <= now) {
    return { state: "Unknown", reason: "No fresh access observation" };
  }
  if (binding.health === "Healthy" || binding.health === "Validated") {
    if (
      !binding.checked_at_ms ||
      binding.checked_at_ms > now ||
      !binding.valid_until_ms ||
      binding.valid_until_ms <= now
    ) {
      return { state: "Unknown", reason: "No fresh access observation" };
    }
  }
  return { state: binding.health, reason: binding.reason };
}

export function AuthBindings({ revision }: { revision: AuthRevisionState }) {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, []);
  if (!revision.bindings?.length)
    return <p>No validated account bindings for r{revision.revision}.</p>;
  return (
    <section className="mt-4">
      <h3 className="mb-2 text-sm font-medium">Account bindings of r{revision.revision}</h3>
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            <tr className="text-left text-muted-foreground">
              <th className="py-1 pr-4">Account</th>
              <th className="py-1 pr-4">Type</th>
              <th className="py-1 pr-4">Installation</th>
              <th className="py-1 pr-4">Repository scope</th>
              <th className="py-1 pr-4">Health</th>
            </tr>
          </thead>
          <tbody>
            {revision.bindings.map((binding) => {
              const health = healthLabel(binding, now);
              return (
                <tr key={`${binding.account_id}:${binding.installation_id}`}>
                  <td className="py-1 pr-4">
                    {binding.login}{" "}
                    <span className="text-muted-foreground">#{binding.account_id}</span>
                  </td>
                  <td className="py-1 pr-4">{binding.account_kind}</td>
                  <td className="py-1 pr-4">#{binding.installation_id}</td>
                  <td className="py-1 pr-4">
                    {binding.repository_selection === "all"
                      ? "All repositories (includes future)"
                      : "Selected repositories only"}
                  </td>
                  <td className="py-1 pr-4">
                    <span>{health.state}</span>
                    {health.reason && (
                      <p className="text-xs text-muted-foreground">{health.reason}</p>
                    )}
                    {health.state === "Validated" && (
                      <p className="text-xs">
                        Candidate validation only; current access is checked per Fleet.
                      </p>
                    )}
                    {binding.checked_at_ms && (
                      <p className="text-xs">
                        Checked {new Date(binding.checked_at_ms).toLocaleString()}
                      </p>
                    )}
                    {binding.affected_fleets?.map((fleet) => (
                      <Link
                        key={fleet}
                        className="text-link mr-2"
                        to={`/fleets/${encodeURIComponent(fleet)}`}
                      >
                        {fleet}
                      </Link>
                    ))}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </section>
  );
}

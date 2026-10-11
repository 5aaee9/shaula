import { useState } from "react";
import { initialPlacement, type Placement } from "./fleet-form-body";
import { initialPoolMember } from "./template-pool";
import type { TemplatePoolSpec } from "./types";

/** Draft state for each way a fleet can source templates; only the chosen one is submitted. */
export function useFleetPlacement(original?: {
  template_pool?: TemplatePoolSpec;
  template_pool_ref?: string;
}) {
  const [placement, setPlacement] = useState<Placement>(() => initialPlacement(original));
  // The loaded pool is kept as-is so an unchanged pool can be recognised.
  const [pool, setPool] = useState<TemplatePoolSpec>(
    () =>
      original?.template_pool || {
        members: [initialPoolMember(0)],
        failure_policy: "backpressure",
      },
  );
  const [poolRef, setPoolRef] = useState(original?.template_pool_ref || "");
  return { placement, setPlacement, pool, setPool, poolRef, setPoolRef };
}

export type FleetPlacementDraft = ReturnType<typeof useFleetPlacement>;

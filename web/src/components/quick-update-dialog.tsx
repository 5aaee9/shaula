import { useQuery } from "@tanstack/react-query";
import { ArrowRight, ArrowUpFromLine, LoaderCircle } from "lucide-react";
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import type { Resource } from "@/lib/api";
import { loadQuickUpdate, missingRequiredBindings, shortDigest } from "@/lib/quick-update";
import type { ChangeRef, SourceUpdate, TemplateResource, TemplateRevision } from "@/lib/types";
import { useTemplateUpdate } from "@/lib/use-template-update";
import { Modal } from "./modal";
import { ErrorNotice, KeyValue, Loading } from "./status";
import { Button } from "./ui/button";

function Transition({ from, to }: { from: string; to: string }) {
  return (
    <span className="inline-flex flex-wrap items-center gap-1.5 font-mono text-xs">
      <span className="text-muted-foreground">{from}</span>
      <ArrowRight className="size-3" aria-label="to" />
      <span>{to}</span>
    </span>
  );
}

function QuickUpdateForm({
  resource,
  revision,
  update,
  missing,
  onClose,
  onReview,
  onAccepted,
}: {
  resource: Resource<TemplateResource>;
  revision: TemplateRevision;
  update: SourceUpdate & { targetDigest: string; targetEngine: string };
  missing: string[];
  onClose: () => void;
  onReview: () => void;
  onAccepted: (change: ChangeRef | null) => void;
}) {
  const send = useTemplateUpdate(resource, onAccepted);
  const blocked = missing.length > 0 || send.conflict;
  return (
    <div className="form-stack">
      <dl className="space-y-3 text-sm">
        <KeyValue label="Default template">{update.sourceKey}</KeyValue>
        <KeyValue label="Artifact">
          <Transition
            from={shortDigest(revision.artifactDigest)}
            to={shortDigest(update.targetDigest)}
          />
        </KeyValue>
        {revision.engineRef !== update.targetEngine && (
          <KeyValue label="Engine">
            <Transition from={revision.engineRef} to={update.targetEngine} />
          </KeyValue>
        )}
      </dl>
      {missing.length > 0 && (
        <p role="status" className="text-sm">
          Needs review: new required {missing.length === 1 ? "binding" : "bindings"}{" "}
          {missing.join(", ")}.
        </p>
      )}
      {send.error !== null && <ErrorNotice error={send.error} />}
      <div className="dialog-actions">
        <Button variant="outline" disabled={send.busy} onClick={onClose}>
          Cancel
        </Button>
        <Button variant="outline" disabled={send.busy} onClick={onReview}>
          Review changes
        </Button>
        <Button
          disabled={send.busy || blocked}
          onClick={() =>
            void send.send(() =>
              JSON.stringify({
                artifact_digest: update.targetDigest,
                engine_ref: update.targetEngine,
                source_key: update.sourceKey,
              }),
            )
          }
        >
          {send.busy ? <LoaderCircle className="animate-spin" /> : <ArrowUpFromLine />}
          {send.busy ? "Updating…" : "Update"}
        </Button>
      </div>
    </div>
  );
}

/**
 * One-click update to the recorded default source. Bindings and input policy
 * are inherited by the server; anything needing a decision goes to review.
 */
export function QuickUpdateDialog({
  profileKey,
  onClose,
  onAccepted,
}: {
  profileKey: string;
  onClose: () => void;
  onAccepted: (change: ChangeRef | null) => void;
}) {
  const navigate = useNavigate();
  const [reviewId] = useState(() => crypto.randomUUID());
  const query = useQuery({
    queryKey: ["template-quick-update", profileKey, reviewId],
    // The confirmation freezes its base; background invalidation cannot retarget it.
    staleTime: "static",
    gcTime: 0,
    queryFn: ({ signal }) => loadQuickUpdate(profileKey, signal),
  });
  const review = () => navigate(`/templates/${encodeURIComponent(profileKey)}/update`);
  const update = query.data?.resource.data.sourceUpdate;
  return (
    <Modal
      open
      title={`Update ${profileKey}?`}
      description="Bindings and input policy are kept; fleets following this template use the new revision for new jobs."
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      {query.isPending ? (
        <Loading />
      ) : query.error ? (
        <ErrorNotice error={query.error} retry={() => void query.refetch()} />
      ) : update?.state === "Available" && update.targetDigest && update.targetEngine ? (
        <QuickUpdateForm
          resource={query.data.resource}
          revision={query.data.revision}
          update={{
            ...update,
            targetDigest: update.targetDigest,
            targetEngine: update.targetEngine,
          }}
          missing={missingRequiredBindings(query.data.variables, query.data.revision.bindings)}
          onClose={onClose}
          onReview={review}
          onAccepted={onAccepted}
        />
      ) : (
        <div className="form-stack">
          <p role="status" className="text-sm">
            {update?.state === "UpToDate"
              ? "Up to date with the default template."
              : "No default template update is available."}
          </p>
          <div className="dialog-actions">
            <Button variant="outline" onClick={onClose}>
              Close
            </Button>
          </div>
        </div>
      )}
    </Modal>
  );
}

import { useId, useState, type FormEvent } from "react";
import type { Resource } from "@/lib/api";
import type { ChangeRef, TemplateResource } from "@/lib/types";
import { useTemplateFormSource } from "@/lib/use-template-form-source";
import {
  invalidPublishSetting,
  templateProfileBody,
  useTemplatePublish,
} from "@/lib/use-template-publish";
import { useTemplateVariables, type TemplateSource } from "@/lib/template-variables";
import { ManualPublishSettings, TemplateDetailsSection } from "./template-form-fields";
import { PublicationSummary, PublishBar } from "./template-form-summary";
import { TemplateVariablesSection } from "./template-form-variables";
import { TemplateSourceFields } from "./template-source-fields";

export function TemplateForm({
  resource,
  initialSource,
  scopes,
  onClose,
  onAccepted,
}: {
  resource?: Resource<TemplateResource>;
  initialSource?: TemplateSource;
  scopes: string[];
  onClose: () => void;
  onAccepted: (change: ChangeRef | null) => void;
}) {
  const id = useId();
  const [snapshot] = useState(resource);
  const [key, setKey] = useState(snapshot?.data.key || "");
  const form = useTemplateFormSource(initialSource);
  const [advanced, setAdvanced] = useState(false);
  const [bindings, setBindings] = useState("{}");
  const [policy, setPolicy] = useState("{}");
  const { busy, error, publish } = useTemplatePublish(snapshot, onAccepted);
  const canRead = scopes.includes("template.read");
  const discovery = useTemplateVariables(form.artifactDigest, form.file, form.source, canRead);

  async function submit(event: FormEvent) {
    event.preventDefault();
    await publish(key, async () => {
      const invalid = invalidPublishSetting(bindings, policy);
      if (invalid) {
        setAdvanced(true);
        throw new Error(`${invalid} must be a JSON object.`);
      }
      return templateProfileBody({
        artifactDigest: await discovery.artifact(),
        engineRef: form.engineRef,
        sourceKey: form.source === "default" ? form.selectedSource?.key : undefined,
        bindings,
        policy,
      });
    });
  }

  return (
    <form aria-label="Template configuration" className="min-w-0" onSubmit={submit}>
      <div className="grid min-w-0 items-start gap-6 xl:grid-cols-[minmax(0,1fr)_17rem]">
        <fieldset disabled={busy} className="min-w-0 space-y-6">
          <TemplateDetailsSection id={id} locked={!!snapshot} profileKey={key} onKeyChange={setKey}>
            <TemplateSourceFields
              source={form.source}
              setSource={form.setSource}
              selectedSource={form.selectedSource}
              onSelect={form.setSelectedSource}
              digest={form.digest}
              setDigest={form.setDigest}
              file={form.file}
              setFile={form.setFile}
              canRead={canRead}
              busy={busy}
            />
          </TemplateDetailsSection>

          <TemplateVariablesSection
            id={id}
            canRead={canRead}
            discovery={discovery}
            ready={form.ready}
            source={form.source}
            bindings={bindings}
            policy={policy}
            setBindings={setBindings}
            setPolicy={setPolicy}
          />

          <ManualPublishSettings
            open={advanced}
            onOpenChange={setAdvanced}
            fixedEngine={form.source === "default"}
            engineRef={form.engineRef}
            setEngine={form.setEngine}
            bindings={bindings}
            setBindings={setBindings}
            policy={policy}
            setPolicy={setPolicy}
          />
        </fieldset>

        <PublicationSummary
          profileKey={key}
          source={form.source}
          selectedSource={form.selectedSource}
          file={form.file}
          engineRef={form.engineRef}
        />
      </div>

      <PublishBar busy={busy} inspecting={discovery.loading} error={error} onClose={onClose} />
    </form>
  );
}

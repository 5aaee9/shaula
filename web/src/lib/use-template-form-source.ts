import { useState } from "react";
import type { TemplateSourceKind } from "@/components/template-source-fields";
import type { TemplateSource } from "./template-variables";

/**
 * Where a published template comes from. A default template fixes both the
 * artifact and engine; archive and existing-artifact sources use form values.
 */
export function useTemplateFormSource(initialSource?: TemplateSource) {
  const [digest, setDigest] = useState("");
  const [file, setFile] = useState<File | null>(null);
  const [source, setSource] = useState<TemplateSourceKind>(initialSource ? "default" : "archive");
  const [selectedSource, setSelectedSource] = useState(initialSource);
  const [engine, setEngine] = useState(initialSource?.engineRef || "terraform");
  const fromDefault = source === "default";
  const artifactDigest = fromDefault ? selectedSource?.artifactDigest || "" : digest;
  const engineRef = fromDefault ? selectedSource?.engineRef || "" : engine;
  const ready = source === "archive" ? !!file : /^sha256:[a-f0-9]{64}$/.test(artifactDigest);
  return {
    source,
    setSource,
    selectedSource,
    setSelectedSource,
    digest,
    setDigest,
    file,
    setFile,
    setEngine,
    artifactDigest,
    engineRef,
    ready,
  };
}

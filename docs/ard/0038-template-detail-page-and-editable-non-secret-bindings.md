---
status: proposed
date: 2026-09-14
---

# Template detail page and editable non-secret bindings on Update

Spec 0005 §5.2 deliberately projects Template reads down to a single
`bindings_present: boolean`: every binding value — including non-secret fields
such as a Proxmox host URL or VMID range — is withheld. Spec 0021 §4 then makes
Update reuse the whole immutable binding set verbatim, so a publisher who wants
to change only a non-secret field (the Proxmox API origin, the template name,
an ISO storage name) must republish through New revision and retype every
secret. The Templates list also exposes three overlapping actions — the row
**Update**, the detail **Update from default**, and **New revision** — and a
template's name merely expands an inline section rather than opening a page.

The user asked for three things: a dedicated Template detail page that shows
the non-secret binding values, an Update flow that can edit those non-secret
values while keeping secrets server-side, and fewer, clearer update actions.

We relax the conservative read projection to a **schema-driven split**. The
artifact's versioned bindings schema already annotates every field
`sensitive: true|false`. A revision read now returns, alongside
`bindings_present`, a `bindings` object containing only the `sensitive: false`
fields verbatim plus one boolean presence marker per `sensitive: true` field —
never the secret value, prefix, suffix, hash or length. Secret bytes still live
only in the immutable Revision and the exact-Revision handoff to the IaC child;
they never enter a GET/list/status/attestation response, log, span or metric.
Spec 0005 §5.2's "omit `bindings` values altogether" becomes "omit sensitive
binding values, expose non-sensitive binding values".

Update grows an optional `bindings` field so non-sensitive fields can be
changed. The server resolves the submitted object against the base Revision's
stored bindings per field: a `sensitive: true` field that is omitted or
submitted as the keep-sentinel reuses the stored value; a new secret value
replaces it; a `sensitive: false` field uses whatever is submitted, and omitting
it retains the stored value. The merged result is validated against the target
artifact's schema before admission, so a non-secret edit cannot silently drop a
required field. Secret equality for idempotency is still computed in protected
memory against the committed Revision; the idempotency record stores no secret
bytes or secret-derived verifier.

On the UI, the template row's name links to `/templates/{key}` — a real detail
route showing profile metadata and the non-secret binding values (secrets shown
only as a "configured" marker). The three actions collapse to two intent-driven
buttons on that page: **Update** (adopt the default source, edit non-secret
bindings and policy) and **New revision** (publish a fresh artifact). The row
keeps a single Update action.

Alternatives considered and rejected:

- **Keep the presence-only projection, add a separate non-secret-binding
  read.** A dedicated endpoint would work but leaves the revision read telling
  the user only "configured" while a second call fetches the editable fields —
  two sources of truth for one page. Rejected in favour of projecting the
  non-secret fields directly into the revision read the detail page already
  consumes.
- **Expose bindings on Update as a full round-trip.** Returning even
  non-secret bindings to the browser and resubmitting them wholesale would
  reintroduce redaction round-trips spec 0005 rejected, and would tempt the UI
  to echo secrets back. Rejected: secrets stay write-only; only non-sensitive
  fields round-trip, and only on Update.
- **Let Update carry a partial non-secret patch.** A sparse patch object is
  ambiguous against a `sensitive: false` field that should be *cleared* versus
  merely *not mentioned*. Rejected: the Update `bindings` object is a complete
  desired binding set; omission means "keep the stored value", so the merged
  result is always a full, schema-valid object.
- **Route the detail view to a query param like the inline section.** Keeping
  `?key=` preserves the current expansion but cannot be linked, refreshed or
  returned-to after login cleanly, and mixes list and detail state. Rejected in
  favour of a dedicated route matching Fleet and Runner detail pages.

Implications:

- The revision GET gains a `bindings` field (non-sensitive values + secret
  presence markers); `bindings_present` is retained for compatibility.
- `POST /template-profiles/{key}/updates` accepts an optional `bindings`
  object; `template_put_impl` merges it with the stored bindings instead of
  always reusing them, and NoOp/identity equality now covers the merged set.
- A new `/templates/{key}` detail route and page; the list's name cell links to
  it; the inline `TemplateDetails` section is removed.
- Secret handling invariants are unchanged: secret bytes never cross the read
  boundary, the keep-sentinel is never stored as a value, and idempotent replay
  compares secrets only in protected memory.

Implementation and acceptance are tracked separately from this proposed
decision.

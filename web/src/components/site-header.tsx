import { Separator } from "@/components/ui/separator";
import { SidebarTrigger, useSidebar } from "@/components/ui/sidebar";
import { Tip } from "@/components/icon-tooltip";
import { StatusBadge } from "@/components/status";

/** Navigation toggle plus a health badge that appears only when the server needs attention. */
export function SiteHeader({ problem }: { problem?: string }) {
  const { isMobile, open } = useSidebar();
  const triggerLabel = isMobile
    ? "Open navigation"
    : open
      ? "Collapse navigation"
      : "Expand navigation";
  return (
    <header className="flex h-(--header-height) shrink-0 items-center gap-2 border-b px-4 lg:px-6">
      <Tip label={triggerLabel}>
        <SidebarTrigger className="-ml-1" aria-label={triggerLabel} />
      </Tip>
      {problem && (
        <>
          <Separator orientation="vertical" className="mx-2 data-[orientation=vertical]:h-4" />
          <StatusBadge value={problem} />
        </>
      )}
    </header>
  );
}

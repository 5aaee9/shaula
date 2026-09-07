import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { SidebarTrigger, useSidebar } from "@/components/ui/sidebar";
import { Tip } from "@/components/icon-tooltip";
import { StatusBadge } from "@/components/status";

export function SiteHeader({ title, health }: { title: string; health: string }) {
  const { isMobile, open } = useSidebar();
  const triggerLabel = isMobile
    ? "Open navigation"
    : open
      ? "Collapse navigation"
      : "Expand navigation";
  return (
    <header className="flex min-h-(--header-height) shrink-0 items-center gap-2 border-b transition-[width,height] ease-linear group-has-data-[collapsible=icon]/sidebar-wrapper:h-(--header-height)">
      <div className="flex w-full min-w-0 items-center gap-1 px-4 lg:gap-2 lg:px-6">
        <Tip label={triggerLabel}>
          <SidebarTrigger className="-ml-1" aria-label={triggerLabel} />
        </Tip>
        <Separator orientation="vertical" className="mx-2 data-[orientation=vertical]:h-4" />
        <span className="min-w-0 flex-1 py-2 text-sm font-medium break-words">{title}</span>
        <div className="ml-auto flex shrink-0 items-center gap-2">
          <StatusBadge value={health} />
          <Button variant="ghost" asChild size="sm" className="hidden sm:flex">
            <a href="https://github.com/5aaee9/shaula" rel="noopener noreferrer" target="_blank">
              GitHub
            </a>
          </Button>
        </div>
      </div>
    </header>
  );
}
